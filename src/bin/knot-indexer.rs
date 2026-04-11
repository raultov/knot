//! knot — Codebase Graph + Vector RAG Indexer
//!
//! Entry point for the knot indexing binary.
//! Handles CLI, database initialization, and watch mode.
//! Delegates actual pipeline execution to the runner module.

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use knot::{
    config::Config,
    db::{
        graph::{ConnectExt, GraphDb},
        vector::{VectorConnectExt, VectorDb},
    },
    pipeline::runner::{run_indexing_pipeline, setup_watch_mode},
    pipeline::state::IndexState,
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

    let vector_db = Arc::new(vector_db);
    let graph_db = Arc::new(graph_db);

    // Initial indexing run
    info!("Performing initial indexing run...");
    run_indexing_pipeline(&cfg, &vector_db, &graph_db, &mut index_state).await?;

    // Watch mode: Monitor filesystem for real-time incremental updates
    if cfg.watch {
        info!(
            "Watch mode enabled. Monitoring {} for changes...",
            cfg.repo_path
        );
        setup_watch_mode(&cfg, &vector_db, &graph_db, &mut index_state).await?;
    }

    Ok(())
}

/// Print startup banner with configuration details.
fn print_startup_banner(cfg: &Config) {
    info!("knot indexer starting (v0.5.2 - parallel streaming + watch mode)");
    info!("Repository path : {}", cfg.repo_path);
    info!("Repository name : {}", cfg.repo_name);
    info!("Clean mode      : {}", cfg.clean);
    info!("Watch mode      : {}", cfg.watch);
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


    Ok(())
}

/// Setup and run the watch mode event loop.
///
/// This function:
/// 1. Creates a debounced filesystem watcher
/// 2. Listens for changes to supported source files (.java, .ts, .tsx, .cts)
/// 3. Triggers incremental re-indexing on file changes
async fn setup_watch_mode(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(100);

    // Setup debounced filesystem watcher
    let mut debouncer = notify_debouncer_mini::new_debouncer(
        std::time::Duration::from_millis(500),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            if let Ok(events) = res {
                for event in events {
                    let _ = tx.blocking_send(event.path);
                }
            }
        },
    )?;

    debouncer
        .watcher()
        .watch(Path::new(&cfg.repo_path), notify::RecursiveMode::Recursive)?;

    // Event loop for watch mode
    while let Some(path) = rx.recv().await {
        // Check if the changed file is a supported source file
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        if matches!(ext, "java" | "ts" | "tsx" | "cts") {
            info!("Change detected in: {}", path.display());
            if let Err(e) = run_indexing_pipeline(cfg, vector_db, graph_db, index_state).await {
                tracing::error!("Error during incremental update: {e:#}");
            }
        }
    }

    Ok(())
}

/// Print startup banner with configuration details.
fn print_startup_banner(cfg: &Config) {
    info!("knot indexer starting (v0.5.2 - parallel streaming + watch mode)");
    info!("Repository path : {}", cfg.repo_path);
    info!("Repository name : {}", cfg.repo_name);
    info!("Clean mode      : {}", cfg.clean);
    info!("Watch mode      : {}", cfg.watch);
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
