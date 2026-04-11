//! knot — Codebase Graph + Vector RAG Indexer
//!
//! Batch/nightly indexer binary with incremental indexing support.
//! Coordinates the pipeline stages with memory-efficient chunking:
//!
//! ```text
//! 1. Input   → discover source files + classify changes (SHA-256)
//! 2. Parse   → extract AST entities from changed files only (Rayon, Tree-sitter)
//! 3. Prepare → assign deterministic UUIDs + build embedding text
//! 4. Embed   → generate vectors in memory-efficient chunks (fastembed)
//! 5. Ingest  → dual-write to Qdrant + Neo4j (Tokio)
//! 6. Resolve → hydrate global context + create relationships
//! ```

use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

use knot::{
    config::Config,
    db::{
        graph::{ConnectExt, DeleteExt, GraphDb, UpsertExt},
        vector::{VectorConnectExt, VectorDb, VectorDeleteExt},
    },
    models::{EmbeddedEntity, ParsedEntity, ResolutionEntity},
    pipeline::{
        embed::Embedder,
        ingest::{ingest_batch, resolve_reference_intents_with_context},
        input::discover_files,
        parser::{ParseConfig, parse_files_stream},
        prepare::prepare_entities,
        state::IndexState,
    },
    utils,
};
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    // Logging must be initialised before anything else.
    utils::init_logging()?;

    // Load configuration (.env takes precedence over CLI args).
    let cfg = Config::load()?;
    print_startup_banner(&cfg);

    // Initialize databases and load previous state.
    let (vector_db, graph_db) = init_databases(&cfg).await?;
    let mut index_state = IndexState::load(&cfg.repo_path)?;

    // Stage 1: Discover and classify files.
    let all_files = discover_files(&cfg.repo_path)?;
    if all_files.is_empty() {
        info!("No supported source files found. Exiting.");
        return Ok(());
    }

    let (_, modified_files, added_files, deleted_files) =
        classify_files_for_indexing(&all_files, &index_state, cfg.clean)?;

    info!(
        "File classification: {} unchanged, {} modified, {} added, {} deleted",
        all_files.len() - modified_files.len() - added_files.len() - deleted_files.len(),
        modified_files.len(),
        added_files.len(),
        deleted_files.len()
    );

    // Clean stale data before re-indexing.
    clean_stale_data(
        &vector_db,
        &graph_db,
        &cfg,
        &deleted_files,
        &modified_files,
        &added_files,
    )
    .await?;

    // Determine files to parse and exit early if index is up to date.
    let files_to_parse = calculate_files_to_parse(added_files, modified_files);
    if is_index_up_to_date(&files_to_parse) {
        info!("No files changed — index is up to date!");
        return Ok(());
    }

    info!(
        "Will parse and index {} file(s) (added/modified)",
        files_to_parse.len()
    );

    // --- NEW STREAMING PIPELINE ---
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
    let vector_db_arc = Arc::new(vector_db);
    let graph_db_arc = Arc::new(graph_db);
    let ingest_handle = {
        let vdb = Arc::clone(&vector_db_arc);
        let gdb = Arc::clone(&graph_db_arc);
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

    if total_entities == 0 {
        info!("No entities extracted. Saving state and exiting.");
        return save_index_state_and_exit(
            &mut index_state,
            &files_to_parse,
            &deleted_files,
            &cfg.repo_path,
            0,
        );
    }

    // Stage 7: Relationship Resolution
    let mut resolution_entities = Vec::with_capacity(total_entities);
    while let Ok(res_entity) = res_rx.try_recv() {
        resolution_entities.push(res_entity);
    }

    resolve_and_save_relationships_v2(&mut resolution_entities, &graph_db_arc, &cfg).await?;

    // Save final state and exit.
    save_index_state_and_exit(
        &mut index_state,
        &files_to_parse,
        &deleted_files,
        &cfg.repo_path,
        total_entities,
    )
}

/// New version of relationship resolution using ResolutionEntity.
async fn resolve_and_save_relationships_v2(
    entities: &mut [ResolutionEntity],
    graph_db: &GraphDb,
    cfg: &Config,
) -> Result<()> {
    if !entities.is_empty() {
        // Build list of repos to include in context (current repo + dependencies)
        let mut repos_to_load = vec![cfg.repo_name.clone()];
        repos_to_load.extend(cfg.dependency_repos.clone());

        info!("Loading global entity context from Neo4j for relationship resolution...");
        let (fqn_to_uuid, name_to_uuids) = graph_db.load_entity_mappings(&repos_to_load).await?;

        if !cfg.dependency_repos.is_empty() {
            info!(
                "Cross-repository resolution enabled: {} local repo(s) + {} dependency repo(s)",
                1,
                cfg.dependency_repos.len()
            );
        }

        info!(
            "Resolving reference intents with global context ({} FQNs, {} names)...",
            fqn_to_uuid.len(),
            name_to_uuids.len()
        );

        resolve_reference_intents_with_context(entities, fqn_to_uuid, name_to_uuids);

        // Create typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
        info!("Creating typed relationships in Neo4j...");
        graph_db.upsert_relationships(entities).await?;
    }
    Ok(())
}

/// Print startup banner with configuration details.
fn print_startup_banner(cfg: &Config) {
    info!("knot indexer starting (v0.5.0 - parallel streaming mode)");
    info!("Repository path : {}", cfg.repo_path);
    info!("Repository name : {}", cfg.repo_name);
    info!("Clean mode      : {}", cfg.clean);
    info!(
        "Qdrant          : {} / {}",
        cfg.qdrant_url, cfg.qdrant_collection
    );
    info!("Neo4j           : {}", cfg.neo4j_uri);
}

/// Initialize database connections and perform pre-flight checks.
async fn init_databases(cfg: &Config) -> Result<(VectorDb, GraphDb)> {
    let vector_db =
        VectorDb::connect(&cfg.qdrant_url, &cfg.qdrant_collection, cfg.embed_dim).await?;
    vector_db.ensure_collection().await?;

    let graph_db = GraphDb::connect(&cfg.neo4j_uri, &cfg.neo4j_user, &cfg.neo4j_password).await?;
    graph_db.ensure_indexes().await?;

    Ok((vector_db, graph_db))
}

/// Clean stale data from databases based on files to delete.
async fn clean_stale_data(
    vector_db: &VectorDb,
    graph_db: &GraphDb,
    cfg: &Config,
    deleted_files: &[String],
    modified_files: &[PathBuf],
    added_files: &[PathBuf],
) -> Result<()> {
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

/// Save index state and log completion.
fn save_index_state_and_exit(
    index_state: &mut IndexState,
    files_to_parse: &[PathBuf],
    deleted_files: &[String],
    repo_path: &str,
    embedded_count: usize,
) -> Result<()> {
    info!("Updating index state...");
    index_state.update_files(files_to_parse)?;
    index_state.remove_files(deleted_files);
    index_state.save(repo_path)?;

    info!(
        "Indexing complete! Processed {} entities from {} file(s).",
        embedded_count,
        files_to_parse.len()
    );

    Ok(())
}

type FileClassification = (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<String>);

/// Classify files into unchanged, modified, added, and deleted categories.
/// In clean mode, all files are treated as new; otherwise, uses index state classification.
fn classify_files_for_indexing(
    all_files: &[PathBuf],
    index_state: &IndexState,
    clean_mode: bool,
) -> anyhow::Result<FileClassification> {
    if clean_mode {
        Ok((vec![], vec![], all_files.to_vec(), vec![]))
    } else {
        index_state.classify_files(all_files)
    }
}

/// Calculate which files should be deleted from the databases before re-indexing.
/// Includes deleted files, modified files, and added files (for idempotency).
fn calculate_files_to_delete(
    deleted: &[String],
    modified: &[PathBuf],
    added: &[PathBuf],
) -> Vec<String> {
    let mut files_to_delete = deleted.to_vec();
    files_to_delete.extend(
        modified
            .iter()
            .chain(added.iter())
            .filter_map(|p| p.to_str().map(String::from)),
    );
    files_to_delete
}

/// Calculate which files need to be parsed and indexed.
fn calculate_files_to_parse(added: Vec<PathBuf>, modified: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut files_to_parse = Vec::new();
    files_to_parse.extend(added);
    files_to_parse.extend(modified);
    files_to_parse
}

/// Stage 1: Check if there are any files to process.
fn is_index_up_to_date(files_to_parse: &[PathBuf]) -> bool {
    files_to_parse.is_empty()
}

/// Stage 2: Build configuration for the parsing stage.
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
    fn test_calculate_files_to_delete() {
        let deleted = vec!["deleted.java".to_string()];
        let modified = vec![PathBuf::from("modified.ts")];
        let added = vec![PathBuf::from("added.tsx")];

        let to_delete = calculate_files_to_delete(&deleted, &modified, &added);

        assert_eq!(to_delete.len(), 3);
        assert!(to_delete.contains(&"deleted.java".to_string()));
        assert!(to_delete.contains(&"modified.ts".to_string()));
        assert!(to_delete.contains(&"added.tsx".to_string()));
    }

    #[test]
    fn test_calculate_files_to_parse() {
        let added = vec![PathBuf::from("added.java")];
        let modified = vec![PathBuf::from("modified.ts")];

        let to_parse = calculate_files_to_parse(added, modified);

        assert_eq!(to_parse.len(), 2);
        assert_eq!(to_parse[0], PathBuf::from("added.java"));
        assert_eq!(to_parse[1], PathBuf::from("modified.ts"));
    }

    #[test]
    fn test_is_index_up_to_date() {
        assert!(is_index_up_to_date(&[]));
        assert!(!is_index_up_to_date(&[PathBuf::from("file.ts")]));
    }

    #[test]
    fn test_build_parse_config() {
        let config = build_parse_config(Some("/custom/path".to_string()), "my-repo".to_string());
        assert_eq!(config.repo_name, "my-repo");
        assert_eq!(config.custom_queries_path, Some("/custom/path".to_string()));
    }

    #[test]
    fn test_classify_files_for_indexing_clean_mode() {
        let all_files = vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")];
        let index_state = IndexState::default();

        let (unchanged, modified, added, deleted) =
            classify_files_for_indexing(&all_files, &index_state, true).unwrap();

        assert_eq!(unchanged.len(), 0);
        assert_eq!(modified.len(), 0);
        assert_eq!(added.len(), 2);
        assert_eq!(deleted.len(), 0);
    }
}
