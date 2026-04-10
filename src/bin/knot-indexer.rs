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
use tracing::info;

use knot::{
    config::Config,
    db::{graph::GraphDb, vector::VectorDb},
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

    info!("knot indexer starting (v0.4.0 - incremental mode)");
    info!("Repository path : {}", cfg.repo_path);
    info!("Repository name : {}", cfg.repo_name);
    info!("Clean mode      : {}", cfg.clean);
    info!(
        "Qdrant          : {} / {}",
        cfg.qdrant_url, cfg.qdrant_collection
    );
    info!("Neo4j           : {}", cfg.neo4j_uri);

    // ------------------------------------------------------------------ //
    // Database connections & pre-flight checks                            //
    // ------------------------------------------------------------------ //

    let vector_db =
        VectorDb::connect(&cfg.qdrant_url, &cfg.qdrant_collection, cfg.embed_dim).await?;
    vector_db.ensure_collection().await?;

    let graph_db = GraphDb::connect(&cfg.neo4j_uri, &cfg.neo4j_user, &cfg.neo4j_password).await?;
    graph_db.ensure_indexes().await?;

    // ------------------------------------------------------------------ //
    // Stage 0 — State tracking: load previous index state                //
    // ------------------------------------------------------------------ //

    let mut index_state = IndexState::load(&cfg.repo_path)?;

    // ------------------------------------------------------------------ //
    // Stage 1 — Input: discover source files                             //
    // ------------------------------------------------------------------ //

    let all_files = discover_files(&cfg.repo_path)?;

    if all_files.is_empty() {
        info!("No supported source files found. Exiting.");
        return Ok(());
    }

    // ------------------------------------------------------------------ //
    // File classification: determine what needs re-indexing              //
    // ------------------------------------------------------------------ //

    let (unchanged_files, modified_files, added_files, deleted_files) = if cfg.clean {
        // Clean mode: treat all files as new
        info!(
            "Clean mode enabled — treating all {} files as new",
            all_files.len()
        );
        (vec![], vec![], all_files.clone(), vec![])
    } else {
        index_state.classify_files(&all_files)?
    };

    info!(
        "File classification: {} unchanged, {} modified, {} added, {} deleted",
        unchanged_files.len(),
        modified_files.len(),
        added_files.len(),
        deleted_files.len()
    );

    // ------------------------------------------------------------------ //
    // Selective deletion: remove stale data                              //
    // ------------------------------------------------------------------ //

    if cfg.clean {
        // Full clean: delete entire repository
        info!("Performing full clean for repo '{}'", cfg.repo_name);
        tokio::try_join!(
            vector_db.delete_by_repo(&cfg.repo_name),
            graph_db.delete_by_repo(&cfg.repo_name),
        )?;
    } else {
        // Incremental: delete only modified and deleted files
        let mut files_to_delete = deleted_files.clone();
        files_to_delete.extend(
            modified_files
                .iter()
                .chain(added_files.iter())
                .filter_map(|p| p.to_str().map(String::from)),
        );

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

    // ------------------------------------------------------------------ //
    // Determine which files to parse                                     //
    // ------------------------------------------------------------------ //

    let mut files_to_parse = Vec::new();
    files_to_parse.extend(added_files);
    files_to_parse.extend(modified_files);

    if files_to_parse.is_empty() {
        info!("No files changed — index is up to date!");
        return Ok(());
    }

    info!(
        "Will parse and index {} file(s) (added/modified)",
        files_to_parse.len()
    );

    // ------------------------------------------------------------------ //
    // Stage 2 — Parse: extract AST entities (CPU-bound, runs on Rayon)  //
    // ------------------------------------------------------------------ //

    let parse_cfg = ParseConfig {
        custom_queries_path: cfg.custom_queries_path.clone(),
        repo_name: cfg.repo_name.clone(),
    };

    // Offload blocking Rayon work to a dedicated OS thread
    let files_clone = files_to_parse.clone();
    let entities =
        tokio::task::spawn_blocking(move || parse_files(&files_clone, &parse_cfg)).await?;

    info!(
        "Extracted {} entities from {} file(s)",
        entities.len(),
        files_to_parse.len()
    );

    if entities.is_empty() {
        info!("No entities extracted. Saving state and exiting.");
        index_state.update_files(&files_to_parse)?;
        index_state.remove_files(&deleted_files);
        index_state.save(&cfg.repo_path)?;
        return Ok(());
    }

    // ------------------------------------------------------------------ //
    // Stage 3 — Prepare: build embedding text                            //
    // ------------------------------------------------------------------ //

    let mut entities = entities;
    prepare_entities(&mut entities);

    // ------------------------------------------------------------------ //
    // Stage 4 & 5 — Memory-efficient chunking: embed + ingest            //
    // Process entities in chunks to avoid OOM on large repositories      //
    // ------------------------------------------------------------------ //

    let chunk_size = cfg.batch_size.min(512); // Cap at 512 for memory safety
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
        ingest_batch(&embedded_chunk, &vector_db, &graph_db).await?;

        // Keep track of all embedded entities for relationship resolution
        all_embedded.extend(embedded_chunk);

        info!(
            "Chunk {}/{} ingested successfully",
            chunk_idx + 1,
            total_entities.div_ceil(chunk_size)
        );
    }

    // ------------------------------------------------------------------ //
    // Stage 6 — Hydrate global context + resolve relationships           //
    // ------------------------------------------------------------------ //

    info!("Loading global entity context from Neo4j for relationship resolution...");
    let (fqn_to_uuid, name_to_uuids) = graph_db.load_entity_mappings(&cfg.repo_name).await?;

    info!(
        "Resolving reference intents with global context ({} FQNs, {} names)...",
        fqn_to_uuid.len(),
        name_to_uuids.len()
    );

    resolve_reference_intents_with_context(&mut all_embedded, fqn_to_uuid, name_to_uuids);

    // Create typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
    info!("Creating typed relationships in Neo4j...");
    graph_db.upsert_relationships(&all_embedded).await?;

    // ------------------------------------------------------------------ //
    // Update index state with newly processed files                      //
    // ------------------------------------------------------------------ //

    info!("Updating index state...");
    index_state.update_files(&files_to_parse)?;
    index_state.remove_files(&deleted_files);
    index_state.save(&cfg.repo_path)?;

    info!(
        "Indexing complete! Processed {} entities from {} file(s).",
        all_embedded.len(),
        files_to_parse.len()
    );

    Ok(())
}
