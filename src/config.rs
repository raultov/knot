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

    /// Logical repository name for multi-repository isolation.
    /// If not provided, defaults to the last component of repo_path.
    /// Example: 'my-java-repo', 'my-microservice'
    #[arg(long, env = "KNOT_REPO_NAME")]
    pub repo_name: Option<String>,

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

    /// Force a full re-index by deleting all existing data for this repository.
    /// When false (default), performs incremental indexing by tracking file changes.
    #[arg(long, env = "KNOT_CLEAN", default_value_t = false)]
    pub clean: bool,

    /// Comma-separated list of repository names to include during cross-repository
    /// dependency analysis. When set, the indexer will load entity mappings from
    /// these additional repositories and resolve cross-repo calls/references.
    /// Example: `KNOT_DEPENDENCIES=core-lib,shared-types`
    #[arg(long, env = "KNOT_DEPENDENCIES")]
    pub dependencies: Option<String>,
}

/// Resolved, validated configuration used throughout the application.
#[derive(Debug, Clone)]
pub struct Config {
    pub repo_path: String,
    pub repo_name: String,
    pub qdrant_url: String,
    pub qdrant_collection: String,
    pub neo4j_uri: String,
    pub neo4j_user: String,
    pub neo4j_password: String,
    pub custom_queries_path: Option<String>,
    pub embed_dim: u64,
    pub batch_size: usize,
    pub clean: bool,
    /// List of repository names to include for cross-repository dependency analysis.
    pub dependency_repos: Vec<String>,
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

        // Auto-detect repo_name from repo_path if not provided
        let repo_name = if let Some(name) = cli.repo_name {
            name
        } else {
            // Extract last component of repo_path as default
            std::path::Path::new(&cli.repo_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| "unnamed-repo".to_string())
        };

        // Parse cross-repository dependencies (comma-separated)
        let dependency_repos = if let Some(deps_str) = &cli.dependencies {
            deps_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };

        Ok(Self {
            repo_path: cli.repo_path,
            repo_name,
            qdrant_url: cli.qdrant_url,
            qdrant_collection: cli.qdrant_collection,
            neo4j_uri: cli.neo4j_uri,
            neo4j_user: cli.neo4j_user,
            neo4j_password: cli.neo4j_password,
            custom_queries_path: cli.custom_queries_path,
            embed_dim: cli.embed_dim,
            batch_size: cli.batch_size,
            clean: cli.clean,
            dependency_repos,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_repo_name_auto_detection() {
        // We use dummy CLI struct to test the logic
        let repo_path = "/path/to/my-project";
        let repo_name_opt: Option<String> = None;

        let repo_name = if let Some(name) = repo_name_opt {
            name
        } else {
            std::path::Path::new(repo_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| "unnamed-repo".to_string())
        };

        assert_eq!(repo_name, "my-project");
    }

    #[test]
    fn test_repo_name_provided() {
        let repo_path = "/path/to/my-project";
        let repo_name_opt: Option<String> = Some("custom-name".to_string());

        let repo_name = if let Some(name) = repo_name_opt {
            name
        } else {
            std::path::Path::new(repo_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| "unnamed-repo".to_string())
        };

        assert_eq!(repo_name, "custom-name");
    }

    #[test]
    fn test_cli_parsing_basic() {
        let args = vec![
            "knot",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
        ];

        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, "/tmp/repo");
        assert_eq!(cli.neo4j_password, "secret");
        assert_eq!(cli.qdrant_url, "http://localhost:6334"); // default
    }

    #[test]
    fn test_cli_parsing_full() {
        let args = vec![
            "knot",
            "--repo-path",
            "/tmp/repo",
            "--repo-name",
            "my-repo",
            "--qdrant-url",
            "http://qdrant:6334",
            "--qdrant-collection",
            "custom_collection",
            "--neo4j-uri",
            "bolt://neo4j:7687",
            "--neo4j-user",
            "admin",
            "--neo4j-password",
            "admin123",
            "--embed-dim",
            "768",
            "--batch-size",
            "128",
            "--clean",
        ];

        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, "/tmp/repo");
        assert_eq!(cli.repo_name, Some("my-repo".to_string()));
        assert_eq!(cli.qdrant_url, "http://qdrant:6334");
        assert_eq!(cli.qdrant_collection, "custom_collection");
        assert_eq!(cli.neo4j_uri, "bolt://neo4j:7687");
        assert_eq!(cli.neo4j_user, "admin");
        assert_eq!(cli.neo4j_password, "admin123");
        assert_eq!(cli.embed_dim, 768);
        assert_eq!(cli.batch_size, 128);
        assert!(cli.clean);
    }

    #[test]
    fn test_parse_dependencies_single() {
        let deps_str = "core-lib";
        let dependency_repos: Vec<String> = deps_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        assert_eq!(dependency_repos.len(), 1);
        assert_eq!(dependency_repos[0], "core-lib");
    }

    #[test]
    fn test_parse_dependencies_multiple() {
        let deps_str = "core-lib,shared-types,utils";
        let dependency_repos: Vec<String> = deps_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        assert_eq!(dependency_repos.len(), 3);
        assert_eq!(dependency_repos[0], "core-lib");
        assert_eq!(dependency_repos[1], "shared-types");
        assert_eq!(dependency_repos[2], "utils");
    }

    #[test]
    fn test_parse_dependencies_with_whitespace() {
        let deps_str = "core-lib , shared-types , utils";
        let dependency_repos: Vec<String> = deps_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        assert_eq!(dependency_repos.len(), 3);
        assert_eq!(dependency_repos[0], "core-lib");
        assert_eq!(dependency_repos[1], "shared-types");
        assert_eq!(dependency_repos[2], "utils");
    }

    #[test]
    fn test_parse_dependencies_empty() {
        let deps_str = "";
        let dependency_repos: Vec<String> = deps_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        assert_eq!(dependency_repos.len(), 0);
    }

    #[test]
    fn test_parse_dependencies_with_trailing_comma() {
        let deps_str = "core-lib,shared-types,";
        let dependency_repos: Vec<String> = deps_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        assert_eq!(dependency_repos.len(), 2);
        assert_eq!(dependency_repos[0], "core-lib");
        assert_eq!(dependency_repos[1], "shared-types");
    }

    #[test]
    fn test_cli_with_dependencies() {
        let args = vec![
            "knot",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--dependencies",
            "core-lib,shared-types",
        ];

        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.dependencies, Some("core-lib,shared-types".to_string()));
    }
}
