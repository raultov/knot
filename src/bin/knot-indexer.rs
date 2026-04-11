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
    models::{EmbeddedEntity, ParsedEntity},
    pipeline::{
        embed::Embedder,
        ingest::{ingest_batch, resolve_reference_intents_with_context},
        input::discover_files,
        parser::{ParseConfig, parse_files},
        prepare::prepare_entities,
        state::IndexState,
    },
    utils,
};

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

    // Stage 2: Parse and prepare entities.
    let mut entities = run_parse_stage(&files_to_parse, &cfg).await?;
    if !should_continue_after_parse(entities.len()) {
        info!("No entities extracted. Saving state and exiting.");
        return save_index_state_and_exit(
            &mut index_state,
            &files_to_parse,
            &deleted_files,
            &cfg.repo_path,
            0,
        );
    }

    prepare_entities(&mut entities);

    // Stage 4 & 5: Embed and ingest entities in chunks.
    let mut all_embedded =
        embed_and_ingest_chunks(entities, &vector_db, &graph_db, cfg.batch_size).await?;

    // Stage 6: Resolve references and relationships.
    resolve_and_save_relationships(&mut all_embedded, &graph_db, &cfg).await?;

    // Save final state and exit.
    save_index_state_and_exit(
        &mut index_state,
        &files_to_parse,
        &deleted_files,
        &cfg.repo_path,
        all_embedded.len(),
    )
}

/// Print startup banner with configuration details.
fn print_startup_banner(cfg: &Config) {
    info!("knot indexer starting (v0.4.3 - incremental mode)");
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

/// Run the parse stage: extract AST entities from source files.
async fn run_parse_stage(files_to_parse: &[PathBuf], cfg: &Config) -> Result<Vec<ParsedEntity>> {
    let parse_cfg = build_parse_config(cfg.custom_queries_path.clone(), cfg.repo_name.clone());

    // Offload blocking Rayon work to a dedicated OS thread
    let files_clone = files_to_parse.to_vec();
    let entities =
        tokio::task::spawn_blocking(move || parse_files(&files_clone, &parse_cfg)).await?;

    info!(
        "Extracted {} entities from {} file(s)",
        entities.len(),
        files_to_parse.len()
    );

    Ok(entities)
}

/// Embed and ingest entities in memory-efficient chunks.
async fn embed_and_ingest_chunks(
    entities: Vec<ParsedEntity>,
    vector_db: &VectorDb,
    graph_db: &GraphDb,
    batch_size: usize,
) -> Result<Vec<EmbeddedEntity>> {
    let chunk_size = calculate_chunk_size(batch_size, 512);
    let total_entities = entities.len();
    let mut all_embedded = Vec::with_capacity(total_entities);

    info!(
        "Processing {} entities in chunks of {} (memory-efficient mode)",
        total_entities, chunk_size
    );

    // Initialize embedder once (reuse across chunks)
    let mut embedder = Embedder::init()?;

    for (chunk_idx, entity_chunk) in entities.chunks(chunk_size).enumerate() {
        info!(
            "Processing chunk {}/{} ({} entities)...",
            chunk_idx + 1,
            total_entities.div_ceil(chunk_size),
            entity_chunk.len()
        );

        // Embed this chunk (CPU-bound ONNX)
        let embedded_chunk = embedder.embed(entity_chunk.to_vec(), chunk_size)?;

        // Ingest nodes immediately (free up memory after ingestion)
        ingest_batch(&embedded_chunk, vector_db, graph_db).await?;

        // Keep track of all embedded entities for relationship resolution
        all_embedded.extend(embedded_chunk);

        info!(
            "Chunk {}/{} ingested successfully",
            chunk_idx + 1,
            total_entities.div_ceil(chunk_size)
        );
    }

    Ok(all_embedded)
}

/// Resolve references and create relationships in the graph database.
async fn resolve_and_save_relationships(
    all_embedded: &mut [EmbeddedEntity],
    graph_db: &GraphDb,
    cfg: &Config,
) -> Result<()> {
    if requires_global_resolution(all_embedded.len()) {
        info!("Loading global entity context from Neo4j for relationship resolution...");
        let (fqn_to_uuid, name_to_uuids) = graph_db.load_entity_mappings(&cfg.repo_name).await?;

        info!(
            "Resolving reference intents with global context ({} FQNs, {} names)...",
            fqn_to_uuid.len(),
            name_to_uuids.len()
        );

        resolve_reference_intents_with_context(all_embedded, fqn_to_uuid, name_to_uuids);

        // Create typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
        info!("Creating typed relationships in Neo4j...");
        graph_db.upsert_relationships(all_embedded).await?;
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

/// Stage 3: Check if we should proceed to preparation after parsing.
fn should_continue_after_parse(extracted_entities_count: usize) -> bool {
    extracted_entities_count > 0
}

/// Stage 4 & 5: Determine batch size for memory safety.
fn calculate_chunk_size(requested_batch_size: usize, memory_safety_limit: usize) -> usize {
    requested_batch_size.min(memory_safety_limit)
}

/// Stage 6: Check if global relationship resolution is needed.
fn requires_global_resolution(total_embedded_count: usize) -> bool {
    total_embedded_count > 0
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
    fn test_should_continue_after_parse() {
        assert!(should_continue_after_parse(10));
        assert!(!should_continue_after_parse(0));
    }

    #[test]
    fn test_calculate_chunk_size() {
        assert_eq!(calculate_chunk_size(100, 512), 100);
        assert_eq!(calculate_chunk_size(1000, 512), 512);
    }

    #[test]
    fn test_requires_global_resolution() {
        assert!(requires_global_resolution(1));
        assert!(!requires_global_resolution(0));
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
