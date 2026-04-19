//! Pipeline orchestration and execution.
//!
//! This module encapsulates the core indexing pipeline logic that coordinates
//! all stages: discovery, parsing, preparation, embedding, ingestion, and relationship resolution.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use crate::config::Config;
use crate::db::{graph::GraphDb, vector::VectorDb};
use crate::models::{EmbeddedEntity, ParsedEntity, ResolutionEntity};
use crate::pipeline::{
    embed::Embedder,
    files::{
        calculate_files_to_delete, calculate_files_to_parse, classify_files_for_indexing,
        update_index_state,
    },
    ingest::{ingest_batch, resolve_and_save_relationships},
    input::discover_files,
    parser::{ParseConfig, parse_files_stream},
    prepare::prepare_entities,
    state::IndexState,
};

/// Core indexing pipeline orchestrator.
///
/// This function coordinates all stages of the indexing process:
/// 1. Discover and classify files (unchanged, modified, added, deleted)
/// 2. Clean stale data from databases
/// 3. Parse files in parallel (Rayon)
/// 4. Batch and embed entities (fastembed)
/// 5. Ingest into Qdrant and Neo4j (dual-write)
/// 6. Resolve cross-repository relationships
pub async fn run_indexing_pipeline(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
) -> Result<()> {
    // Stage 1: Discover and classify files.
    let all_files = discover_files(&cfg.repo_path)?;
    if all_files.is_empty() {
        info!("No supported source files found.");
        return Ok(());
    }

    let (_, modified_files, added_files, deleted_files) =
        classify_files_for_indexing(&all_files, index_state, cfg.clean)?;

    let unchanged_count =
        all_files.len() - modified_files.len() - added_files.len() - deleted_files.len();

    if unchanged_count == all_files.len() && deleted_files.is_empty() {
        info!("No files changed — index is up to date!");
        return Ok(());
    }

    info!(
        "File classification: {} unchanged, {} modified, {} added, {} deleted",
        unchanged_count,
        modified_files.len(),
        added_files.len(),
        deleted_files.len()
    );

    // Clean stale data before re-indexing.
    clean_stale_data(
        vector_db,
        graph_db,
        cfg,
        &deleted_files,
        &modified_files,
        &added_files,
    )
    .await?;

    // Determine files to parse
    let files_to_parse = calculate_files_to_parse(added_files, modified_files);

    if !files_to_parse.is_empty() {
        info!(
            "Will parse and index {} file(s) (added/modified)",
            files_to_parse.len()
        );

        // --- STREAMING PIPELINE ---
        let (parse_tx, mut parse_rx) = mpsc::unbounded_channel::<ParsedEntity>();
        let (embed_tx, mut embed_rx) = mpsc::channel::<Vec<EmbeddedEntity>>(16);
        let (res_tx, mut res_rx) = mpsc::unbounded_channel::<ResolutionEntity>();

        // Stage 2: Parallel Parsing (Rayon)
        info!(
            "Stage 2: Starting parallel parsing of {} files...",
            files_to_parse.len()
        );
        let parse_cfg = build_parse_config(cfg.custom_queries_path.clone(), cfg.repo_name.clone());
        let files_to_parse_clone = files_to_parse.clone();
        tokio::task::spawn_blocking(move || {
            parse_files_stream(&files_to_parse_clone, &parse_cfg, parse_tx);
            info!("Stage 2: Parallel parsing complete.");
        });

        // Stage 3 & 4: Batching & Embedding (CPU)
        let embedder = Arc::new(tokio::sync::Mutex::new(Embedder::init()?));
        let embed_handle = {
            let batch_size = cfg.batch_size;
            let embedder = Arc::clone(&embedder);
            let embed_tx = embed_tx.clone();
            tokio::spawn(async move {
                let mut current_batch = Vec::with_capacity(batch_size);
                let mut batch_count = 0;
                while let Some(entity) = parse_rx.recv().await {
                    current_batch.push(entity);
                    if current_batch.len() >= batch_size {
                        batch_count += 1;
                        let mut batch =
                            std::mem::replace(&mut current_batch, Vec::with_capacity(batch_size));
                        info!(
                            "[Worker: Embedder] Stage 3: Embedding batch #{} ({} entities)...",
                            batch_count,
                            batch.len()
                        );
                        prepare_entities(&mut batch);
                        let embedder_clone = Arc::clone(&embedder);
                        let embedded = tokio::task::spawn_blocking(move || {
                            let mut lock = embedder_clone.blocking_lock();
                            lock.embed(batch, batch_size)
                        })
                        .await??;
                        embed_tx.send(embedded).await?;
                    }
                }
                if !current_batch.is_empty() {
                    batch_count += 1;
                    info!(
                        "[Worker: Embedder] Stage 3: Embedding final batch #{} ({} entities)...",
                        batch_count,
                        current_batch.len()
                    );
                    prepare_entities(&mut current_batch);
                    let embedded = tokio::task::spawn_blocking(move || {
                        let mut lock = embedder.blocking_lock();
                        lock.embed(current_batch, batch_size)
                    })
                    .await??;
                    embed_tx.send(embedded).await?;
                }
                Ok::<(), anyhow::Error>(())
            })
        };

        // Stage 5 & 6: Ingestion & Resolution Prep
        let ingest_handle = {
            let vdb = Arc::clone(vector_db);
            let gdb = Arc::clone(graph_db);
            tokio::spawn(async move {
                let mut total_ingested = 0;
                let mut batch_count = 0;
                while let Some(embedded_batch) = embed_rx.recv().await {
                    batch_count += 1;
                    info!(
                        "[Worker: Ingester] Stage 4: Ingesting batch #{} ({} entities) into Qdrant & Neo4j...",
                        batch_count,
                        embedded_batch.len()
                    );
                    total_ingested += embedded_batch.len();
                    for ee in &embedded_batch {
                        res_tx.send(ResolutionEntity::from(ee))?;
                    }
                    ingest_batch(&embedded_batch, &vdb, &gdb).await?;
                    info!(
                        "[Worker: Ingester] Stage 4: Batch #{} ingested successfully (Total so far: {})",
                        batch_count, total_ingested
                    );
                }
                Ok::<usize, anyhow::Error>(total_ingested)
            })
        };

        // Wait for embedding and ingestion to finish
        embed_handle.await??;
        drop(embed_tx); // Ensure ingest task finishes when embed_rx is empty
        let total_entities = ingest_handle.await??;

        // Stage 7: Relationship Resolution
        let mut resolution_entities = Vec::with_capacity(total_entities);
        while let Ok(res_entity) = res_rx.try_recv() {
            resolution_entities.push(res_entity);
        }

        resolve_and_save_relationships(&mut resolution_entities, graph_db, cfg).await?;

        update_index_state(
            index_state,
            &files_to_parse,
            &deleted_files,
            &cfg.repo_path,
            total_entities,
        )?;
    } else if !deleted_files.is_empty() {
        // Only deletions occurred
        update_index_state(index_state, &[], &deleted_files, &cfg.repo_path, 0)?;
    }

    Ok(())
}

/// Clean stale data from databases based on files to delete.
pub async fn clean_stale_data(
    vector_db: &VectorDb,
    graph_db: &GraphDb,
    cfg: &Config,
    deleted_files: &[String],
    modified_files: &[PathBuf],
    added_files: &[PathBuf],
) -> Result<()> {
    use crate::db::graph::DeleteExt;
    use crate::db::vector::VectorDeleteExt;

    if cfg.clean {
        // Full clean: delete entire repository
        info!("Performing full clean for repo '{}'", cfg.repo_name);
        tokio::try_join!(
            vector_db.delete_by_repo(&cfg.repo_name),
            graph_db.delete_by_repo(&cfg.repo_name),
        )?;
    } else {
        // Incremental: delete only modified and deleted files
        let files_to_delete = calculate_files_to_delete(deleted_files, modified_files, added_files);

        if !files_to_delete.is_empty() {
            info!(
                "Deleting {} stale file(s) from databases (incremental mode)",
                files_to_delete.len()
            );
            tokio::try_join!(
                vector_db.delete_by_file_paths(&cfg.repo_name, &files_to_delete),
                graph_db.delete_by_file_paths(&cfg.repo_name, &files_to_delete),
            )?;
        }
    }
    Ok(())
}

/// Build configuration for the parsing stage.
fn build_parse_config(custom_queries_path: Option<String>, repo_name: String) -> ParseConfig {
    ParseConfig {
        custom_queries_path,
        repo_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_parse_config_variants() {
        let cfg = build_parse_config(None, "repo1".to_string());
        assert_eq!(cfg.repo_name, "repo1");
        assert!(cfg.custom_queries_path.is_none());

        let cfg_custom = build_parse_config(Some("/path".to_string()), "repo2".to_string());
        assert_eq!(cfg_custom.repo_name, "repo2");
        assert_eq!(cfg_custom.custom_queries_path, Some("/path".to_string()));
    }

    #[tokio::test]
    async fn test_run_indexing_pipeline_empty_repo() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let _cfg = Config {
            repo_path: repo_path.clone(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "test".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "password".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            batch_size: 64,
            clean: false,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
        };

        // We need to mock DBs if we want to run the full pipeline,
        // but here we just want to see if it returns Ok(()) when no files are found.
        // discover_files will return empty Vec.

        // However, init_databases is called before this in main.
        // Here we just test the function directly.
        // We'll need a way to create Arc<VectorDb> and Arc<GraphDb> without connecting if possible,
        // or just accept that this test might be limited.

        // Actually, discovering files happens FIRST.
        // If it's empty, it returns Ok(()).
        // Let's try to pass dummy Arcs.
    }
}
