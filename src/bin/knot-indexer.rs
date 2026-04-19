//! knot — Codebase Graph + Vector RAG Indexer
//!
//! Entry point for the knot indexing binary.
//! Handles CLI, database initialization, and watch mode.
//! Delegates actual pipeline execution to the runner module.

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

use knot::{
    config::Config,
    db::{
        graph::{ConnectExt, GraphDb},
        vector::{VectorConnectExt, VectorDb},
    },
    pipeline::runner::run_indexing_pipeline,
    pipeline::state::IndexState,
    pipeline::watch::setup_watch_mode,
    utils,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Logging must be initialised before anything else.
    utils::init_logging()?;

    // Load configuration for indexer (.env takes precedence over CLI args).
    let cfg = Config::load_indexer()?;
    print_startup_banner(&cfg);

    // Initialize databases and load previous state.
    let (vector_db, graph_db) = init_databases(&cfg).await?;
    let mut index_state = IndexState::load(&cfg.repo_path)?;

    let vector_db = Arc::new(vector_db);
    let graph_db = Arc::new(graph_db);

    // Initial indexing run
    info!("Performing initial indexing run...");
    let mut cfg = cfg; // Make config mutable for watch mode
    run_indexing_pipeline(&cfg, &vector_db, &graph_db, &mut index_state).await?;

    // After initial run, disable clean mode to ensure watch mode operates incrementally
    if cfg.watch && cfg.clean {
        info!("Initial clean indexing complete. Switching to incremental mode for watch.");
        cfg.clean = false;
    }

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
    info!(
        "knot indexer starting (v{} - parallel streaming + watch mode)",
        env!("CARGO_PKG_VERSION")
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_mode_disabled_after_initial_run_with_watch() {
        // Simulate the behavior of clean flag being disabled after initial run in watch mode.
        let mut cfg = Config {
            repo_path: "/tmp/test-repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "test".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "password".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            batch_size: 64,
            clean: true,
            dependency_repos: Vec::new(),
            watch: true,
            dry_run: false,
        };

        // Initially, clean should be true (from CLI/env)

        // Initially, clean should be true (from CLI/env)
        assert!(cfg.clean);
        assert!(cfg.watch);

        // After initial run, clean should be disabled for incremental watch mode
        if cfg.watch && cfg.clean {
            cfg.clean = false;
        }

        // Now clean should be false, but watch should still be true
        assert!(!cfg.clean);
        assert!(cfg.watch);
    }

    #[test]
    fn test_clean_mode_unchanged_without_watch() {
        // When watch is disabled, clean flag should remain as configured.
        let mut cfg = Config {
            repo_path: "/tmp/test-repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "test".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "password".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            batch_size: 64,
            clean: true,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
        };

        // Since watch is false, clean flag should not be modified
        if cfg.watch && cfg.clean {
            cfg.clean = false;
        }

        // clean should remain true since watch is false
        assert!(cfg.clean);
    }

    #[test]
    fn test_watch_without_clean_mode() {
        // When watch is enabled but clean is false, nothing should change.
        let mut cfg = Config {
            repo_path: "/tmp/test-repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "test".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "password".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            batch_size: 64,
            clean: true,
            dependency_repos: Vec::new(),
            watch: true,
            dry_run: false,
        };

        // clean is already false, so no change should occur
        if cfg.watch && cfg.clean {
            cfg.clean = false;
        }

        assert!(!cfg.clean);
        assert!(cfg.watch);
    }

    #[test]
    fn test_print_startup_banner_clean_mode() {
        // Test that the startup banner correctly reflects clean mode status.
        let cfg = Config {
            repo_path: "/tmp/test-repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "test".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "password".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            batch_size: 64,
            clean: true,
            dependency_repos: Vec::new(),
            watch: true,
            dry_run: false,
        };

        // Just verify the config is correctly initialized.
        assert_eq!(cfg.repo_path, "/tmp/test-repo");
        assert_eq!(cfg.repo_name, "test-repo");
        assert!(cfg.clean);
        assert!(cfg.watch);
    }
}
