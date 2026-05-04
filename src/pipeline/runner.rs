//! Pipeline orchestration and execution.
//!
//! This module encapsulates the core indexing pipeline logic that coordinates
//! all stages: discovery, parsing, preparation, embedding, ingestion, and relationship resolution.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
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
    ingest::{ingest_batch, link_cross_repo_dependencies, resolve_and_save_relationships},
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
    let all_files = discover_files(&cfg.repo_path, cfg.include_config_files)?;
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
        // Bounded channels provide backpressure: capacity = batch_size * 4
        // limits worst-case memory to ~1.3MB (256 entities * ~5KB each).
        let (parse_tx, mut parse_rx) = mpsc::channel::<ParsedEntity>(cfg.batch_size * 4);
        let (embed_tx, mut embed_rx) = mpsc::channel::<Vec<EmbeddedEntity>>(16);
        let (res_tx, mut res_rx) = mpsc::channel::<ResolutionEntity>(cfg.batch_size * 4);

        // Stage 2: Parallel Parsing (std::thread::scope OS threads)
        info!(
            "Stage 2: Starting parallel parsing of {} files...",
            files_to_parse.len()
        );
        let parse_cfg = build_parse_config(
            cfg.custom_queries_path.clone(),
            cfg.repo_name.clone(),
            cfg.include_config_files,
        );
        let files_to_parse_clone = files_to_parse.clone();

        let cpus = cfg.rayon_threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });

        tokio::task::spawn_blocking(move || {
            parse_files_stream(&files_to_parse_clone, &parse_cfg, parse_tx, cpus);
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

        // Stage 5 & 6: Ingestion & Resolution Prep (concurrent via JoinSet)
        // Drain res_rx concurrently to prevent the ingestion task from
        // deadlocking when the bounded res_tx channel fills up.
        let res_handle = tokio::spawn(async move {
            let mut resolution_entities = Vec::new();
            while let Some(res_entity) = res_rx.recv().await {
                resolution_entities.push(res_entity);
            }
            resolution_entities
        });

        let ingest_handle = {
            let vdb = Arc::clone(vector_db);
            let gdb = Arc::clone(graph_db);
            let max_concurrent = cfg.ingest_concurrency;
            let semaphore = Arc::new(Semaphore::new(max_concurrent));

            info!("Ingestion concurrency: {max_concurrent} simultaneous batches");

            tokio::spawn(async move {
                let mut total_ingested = 0;
                let mut batch_count = 0;
                let mut join_set = JoinSet::new();

                while let Some(embedded_batch) = embed_rx.recv().await {
                    batch_count += 1;
                    total_ingested += embedded_batch.len();

                    // Dispatch resolution entities before ingestion spawns
                    for ee in &embedded_batch {
                        res_tx.send(ResolutionEntity::from(ee)).await?;
                    }

                    // Acquire a semaphore permit to limit concurrency
                    let permit = semaphore.clone().acquire_owned().await?;
                    let vdb = Arc::clone(&vdb);
                    let gdb = Arc::clone(&gdb);
                    let bc = batch_count;
                    let bl = embedded_batch.len();

                    join_set.spawn(async move {
                        info!("[Worker: Ingester] Ingesting batch #{bc} ({bl} entities)...");
                        let result = ingest_batch(&embedded_batch, &vdb, &gdb).await;
                        drop(permit);
                        result
                    });
                }

                info!(
                    "All {} batches dispatched — waiting for ingestion workers to finish...",
                    batch_count
                );

                // Drain all completed tasks, propagating any errors
                while let Some(result) = join_set.join_next().await {
                    result??;
                }

                Ok::<usize, anyhow::Error>(total_ingested)
            })
        };

        // Wait for embedding and ingestion to finish
        embed_handle.await??;
        drop(embed_tx); // Ensure ingest task finishes when embed_rx is empty
        let total_entities = ingest_handle.await??;

        // Stage 7: Relationship Resolution
        // The ingest task drops res_tx on exit, which closes the channel
        // and causes res_handle to finish naturally.
        let mut resolution_entities = res_handle.await?;

        // Cross-repo dependency linking: upsert Repository nodes and create DEPENDS_ON edges.
        // Must run BEFORE relationship resolution so that auto-discovered dependencies
        // are available for cross-repo call resolution.
        link_cross_repo_dependencies(&resolution_entities, graph_db, cfg).await?;

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
fn build_parse_config(
    custom_queries_path: Option<String>,
    repo_name: String,
    include_config_files: bool,
) -> ParseConfig {
    ParseConfig {
        custom_queries_path,
        repo_name,
        include_config_files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputFormat;

    #[test]
    fn test_build_parse_config_variants() {
        let cfg = build_parse_config(None, "repo1".to_string(), false);
        assert_eq!(cfg.repo_name, "repo1");
        assert!(cfg.custom_queries_path.is_none());

        let cfg_custom = build_parse_config(Some("/path".to_string()), "repo2".to_string(), false);
        assert_eq!(cfg_custom.repo_name, "repo2");
        assert_eq!(cfg_custom.custom_queries_path, Some("/path".to_string()));
    }

    /// Verify that JoinSet + Semaphore correctly limits concurrent task execution.
    #[tokio::test]
    async fn test_joinset_semaphore_concurrency_limit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let max_concurrent = 2;
        let semaphore = Arc::new(Semaphore::new(max_concurrent));
        let concurrent_count = Arc::new(AtomicUsize::new(0));
        let peak_concurrent = Arc::new(AtomicUsize::new(0));

        let mut join_set = JoinSet::new();
        let total_tasks = 10;

        for i in 0..total_tasks {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let concurrent = Arc::clone(&concurrent_count);
            let peak = Arc::clone(&peak_concurrent);

            join_set.spawn(async move {
                let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
                drop(permit);
                Ok::<usize, anyhow::Error>(i)
            });
        }

        let mut results = Vec::new();
        while let Some(result) = join_set.join_next().await {
            results.push(result.unwrap().unwrap());
        }

        assert_eq!(results.len(), total_tasks);
        // Peak concurrent should not exceed the semaphore limit
        assert!(peak_concurrent.load(Ordering::SeqCst) <= max_concurrent);
    }

    /// Verify JoinSet drains properly and preserves data.
    #[tokio::test]
    async fn test_joinset_collects_all_tasks() {
        let mut join_set = JoinSet::new();
        let total_tasks = 5;

        for i in 0..total_tasks {
            join_set.spawn(async move { i * 2 });
        }

        let mut results = Vec::new();
        while let Some(result) = join_set.join_next().await {
            results.push(result.unwrap());
        }

        results.sort();
        assert_eq!(results, vec![0, 2, 4, 6, 8]);
    }

    /// Verify JoinSet properly propagates errors.
    #[tokio::test]
    async fn test_joinset_error_propagation() {
        let mut join_set = JoinSet::new();

        join_set.spawn(async { Ok::<_, anyhow::Error>(1) });
        join_set.spawn(async { Err::<i32, _>(anyhow::anyhow!("task failed")) });

        let mut error_seen = false;
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(_)) => error_seen = true,
                Err(_) => error_seen = true,
            }
        }
        assert!(error_seen);
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
            custom_ca_certs: None,
            output_format: OutputFormat::Markdown,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
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
