//! Configuration module.
//!
//! Resolves runtime configuration from two sources with the following precedence:
//!   1. `.env` file (highest priority — if a `.env` file is present, its values
//!      override everything provided on the command line).
//!   2. CLI arguments (used as fallback when `.env` is absent or a key is missing).
//!
//! Callers should use [`Config::load`] as the single entry point.

use anyhow::{Context, Result};
use clap::Parser;

/// Command-line arguments (also usable as environment-variable overrides via clap's `env` attr).
#[derive(Debug, Parser)]
#[command(name = "knot", version, about = "Codebase Graph + Vector RAG Indexer")]
pub struct Cli {
    /// Path to the repository root that will be indexed.
    #[arg(long, env = "KNOT_REPO_PATH")]
    pub repo_path: String,

    /// Qdrant server URL (e.g. http://localhost:6334).
    #[arg(long, env = "KNOT_QDRANT_URL", default_value = "http://localhost:6334")]
    pub qdrant_url: String,

    /// Qdrant collection name where vectors will be stored.
    #[arg(long, env = "KNOT_QDRANT_COLLECTION", default_value = "knot_entities")]
    pub qdrant_collection: String,

    /// Neo4j Bolt URI (e.g. bolt://localhost:7687).
    #[arg(long, env = "KNOT_NEO4J_URI", default_value = "bolt://localhost:7687")]
    pub neo4j_uri: String,

    /// Neo4j username.
    #[arg(long, env = "KNOT_NEO4J_USER", default_value = "neo4j")]
    pub neo4j_user: String,

    /// Neo4j password.
    #[arg(long, env = "KNOT_NEO4J_PASSWORD")]
    pub neo4j_password: String,

    /// Optional path to a directory containing custom Tree-sitter query files
    /// (`java.scm`, `typescript.scm`). When set, these override the built-in
    /// queries shipped with the binary.
    #[arg(long, env = "KNOT_CUSTOM_QUERIES_PATH")]
    pub custom_queries_path: Option<String>,

    /// Embedding model dimension (must match the deployed fastembed model).
    #[arg(long, env = "KNOT_EMBED_DIM", default_value_t = 384)]
    pub embed_dim: u64,

    /// Number of files to process in each rayon parallel batch.
    #[arg(long, env = "KNOT_BATCH_SIZE", default_value_t = 64)]
    pub batch_size: usize,
}

/// Resolved, validated configuration used throughout the application.
#[derive(Debug, Clone)]
pub struct Config {
    pub repo_path: String,
    pub qdrant_url: String,
    pub qdrant_collection: String,
    pub neo4j_uri: String,
    pub neo4j_user: String,
    pub neo4j_password: String,
    pub custom_queries_path: Option<String>,
    pub embed_dim: u64,
    pub batch_size: usize,
}

impl Config {
    /// Load configuration.
    ///
    /// Attempts to load a `.env` file from the current working directory first.
    /// If present, its variables are injected into the process environment **before**
    /// clap parses `std::env::args`, so they naturally take precedence.
    pub fn load() -> Result<Self> {
        // Try to load .env — it is not an error if the file does not exist.
        match dotenvy::dotenv() {
            Ok(path) => tracing::info!("Loaded env from {}", path.display()),
            Err(dotenvy::Error::Io(_)) => {
                tracing::debug!("No .env file found, falling back to CLI arguments")
            }
            Err(e) => return Err(e).context("Failed to parse .env file"),
        }

        let cli = Cli::parse();

        Ok(Self {
            repo_path: cli.repo_path,
            qdrant_url: cli.qdrant_url,
            qdrant_collection: cli.qdrant_collection,
            neo4j_uri: cli.neo4j_uri,
            neo4j_user: cli.neo4j_user,
            neo4j_password: cli.neo4j_password,
            custom_queries_path: cli.custom_queries_path,
            embed_dim: cli.embed_dim,
            batch_size: cli.batch_size,
        })
    }
}
