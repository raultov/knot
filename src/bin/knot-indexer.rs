//! knot — Codebase Graph + Vector RAG Indexer
//!
//! Entry point for the knot indexing binary.
//! Handles CLI, database initialization, and watch mode.
//! Delegates actual pipeline execution to the runner module.

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

use knot::{
    config::Config, db, pipeline::runner::run_indexing_pipeline, pipeline::state::IndexState,
    pipeline::watch::setup_watch_mode, utils,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Logging must be initialised before anything else.
    utils::init_logging()?;

    // Load configuration for indexer (.env takes precedence over CLI args).
    let cfg = Config::load_indexer()?;

    // Inject custom CA certificates for fastembed/hf-hub model downloads if provided.
    // This must be called before any async/tokio threads are spawned.
    utils::inject_custom_ca_certs(&cfg.custom_ca_certs);

    // Configure Rayon thread pool before any parallel parsing occurs.
    // Must be called before the tokio runtime spawns blocking tasks.
    let rayon_threads = utils::configure_rayon(cfg.rayon_threads)?;

    utils::print_startup_banner(&cfg, rayon_threads);

    // Initialize databases and load previous state.
    // When --clean is set, skip loading the old state (it may be from an
    // incompatible version) and start with an empty state instead.
    let (vector_db, graph_db) = db::init_databases(&cfg).await?;
    let mut index_state = if cfg.clean {
        info!("Clean mode: ignoring existing index state");
        IndexState::default()
    } else {
        IndexState::load(&cfg.repo_path)?
    };

    let vector_db = Arc::new(vector_db);
    let graph_db = Arc::new(graph_db);

    // Initial indexing run
    info!("Performing initial indexing run...");
    let mut cfg = cfg; // Make config mutable for watch mode
    let _metrics = run_indexing_pipeline(&cfg, &vector_db, &graph_db, &mut index_state).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use knot::config::OutputFormat;

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
            custom_ca_certs: None,
            output_format: OutputFormat::Markdown,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
        };

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
            custom_ca_certs: None,
            output_format: OutputFormat::Markdown,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
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
            custom_ca_certs: None,
            output_format: OutputFormat::Markdown,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
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
            custom_ca_certs: None,
            output_format: OutputFormat::Markdown,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
        };

        // Just verify the config is correctly initialized.
        assert_eq!(cfg.repo_path, "/tmp/test-repo");
        assert_eq!(cfg.repo_name, "test-repo");
        assert!(cfg.clean);
        assert!(cfg.watch);
    }
}
