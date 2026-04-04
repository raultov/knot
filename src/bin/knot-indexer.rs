//! knot — Codebase Graph + Vector RAG Indexer
//!
//! Batch/nightly indexer binary. Coordinates the five pipeline stages:
//!
//! ```text
//! 1. Input   → discover source files
//! 2. Parse   → extract AST entities (Rayon, Tree-sitter)
//! 3. Prepare → assign UUIDs + build embedding text
//! 4. Embed   → generate vectors (fastembed)
//! 5. Ingest  → dual-write to Qdrant + Neo4j (Tokio)
//! ```

use anyhow::Result;
use tracing::info;

use knot::{
    config::Config,
    db::{graph::GraphDb, vector::VectorDb},
    pipeline::{
        embed::Embedder,
        ingest::ingest_batch,
        input::discover_files,
        parse::{ParseConfig, parse_files},
        prepare::prepare_entities,
    },
    utils,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Logging must be initialised before anything else.
    utils::init_logging()?;

    // Load configuration (.env takes precedence over CLI args).
    let cfg = Config::load()?;

    info!("knot indexer starting");
    info!("Repository path : {}", cfg.repo_path);
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
    // Synchronization: clear stale data for this repository               //
    // ------------------------------------------------------------------ //

    info!("Clearing stale data for repo '{}'…", cfg.repo_path);
    tokio::try_join!(
        vector_db.delete_by_repo(&cfg.repo_path),
        graph_db.delete_by_repo(&cfg.repo_path),
    )?;

    // ------------------------------------------------------------------ //
    // Stage 1 — Input: discover source files                              //
    // ------------------------------------------------------------------ //

    let files = discover_files(&cfg.repo_path)?;

    if files.is_empty() {
        info!("No supported source files found. Exiting.");
        return Ok(());
    }

    // ------------------------------------------------------------------ //
    // Stage 2 — Parse: extract AST entities (CPU-bound, runs on Rayon)   //
    // ------------------------------------------------------------------ //

    let parse_cfg = ParseConfig {
        custom_queries_path: cfg.custom_queries_path.clone(),
    };

    // Offload blocking Rayon work to a dedicated OS thread so the Tokio
    // executor is not starved during the parse phase.
    let files_clone = files.clone();
    let mut entities =
        tokio::task::spawn_blocking(move || parse_files(&files_clone, &parse_cfg)).await?;

    info!(
        "Extracted {} entities from {} files",
        entities.len(),
        files.len()
    );

    if entities.is_empty() {
        info!("No entities extracted. Exiting.");
        return Ok(());
    }

    // ------------------------------------------------------------------ //
    // Stage 3 — Prepare: build embedding text                             //
    // ------------------------------------------------------------------ //

    prepare_entities(&mut entities);

    // ------------------------------------------------------------------ //
    // Stage 4 — Embed: generate vectors (CPU-bound ONNX, runs blocking)  //
    // ------------------------------------------------------------------ //

    let batch_size = cfg.batch_size;
    let mut embedded = tokio::task::spawn_blocking(move || {
        let mut embedder = Embedder::init()?;
        embedder.embed(entities, batch_size)
    })
    .await??;

    // ------------------------------------------------------------------ //
    // Stage 5 — Ingest: dual-write in chunks to avoid OOM on huge repos  //
    // ------------------------------------------------------------------ //

    // Resolve call intents globally before chunking
    knot::pipeline::ingest::resolve_call_intents(&mut embedded);

    let chunk_size = cfg.batch_size;
    for chunk in embedded.chunks(chunk_size) {
        ingest_batch(chunk, &vector_db, &graph_db).await?;
    }

    // After all nodes are in Neo4j, create the relationships
    info!("Creating CALLS relationships globally...");
    graph_db.upsert_calls(&embedded).await?;

    info!("Indexing complete. {} entities ingested.", embedded.len());
    Ok(())
}
