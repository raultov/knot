//! Configuration module.
//!
//! Resolves runtime configuration from two sources with the following precedence:
//!   1. `.env` file (highest priority — if a `.env` file is present, its values
//!      override everything provided on the command line).
//!   2. CLI arguments (used as fallback when `.env` is absent or a key is missing).
//!
//! Provides specialized loaders for different binaries:
//! - [`Config::load_indexer`] for knot-indexer (indexing operations)
//! - [`Config::load_mcp`] for knot-mcp (MCP server)

use anyhow::{Context, Result};
use clap::Parser;

/// Command-line arguments for knot-indexer.
/// Includes all options for indexing, file watching, and query customization.
#[derive(Debug, Parser)]
#[command(
    name = "knot-indexer",
    version,
    about = "Codebase Graph + Vector RAG Indexer"
)]
pub struct IndexerCli {
    /// Path to the repository root that will be indexed.
    /// If not provided, defaults to the current working directory.
    #[arg(long, env = "KNOT_REPO_PATH")]
    pub repo_path: Option<String>,

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
    pub neo4j_password: Option<String>,

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

    /// Run the indexer in watch mode.
    /// When enabled, the indexer will watch for filesystem changes and
    /// perform real-time incremental updates.
    #[arg(long, env = "KNOT_WATCH", default_value_t = false)]
    pub watch: bool,
}

/// Command-line arguments for knot-mcp.
/// Only includes options necessary for the MCP server to connect to databases.
#[derive(Debug, Parser)]
#[command(
    name = "knot-mcp",
    version,
    about = "knot MCP Server for Codebase Semantic Search"
)]
pub struct McpCli {
    /// Path to the repository root that will be indexed.
    /// If not provided, defaults to the current working directory.
    #[arg(long, env = "KNOT_REPO_PATH")]
    pub repo_path: Option<String>,

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
    pub neo4j_password: Option<String>,

    /// Embedding model dimension (must match the deployed fastembed model).
    #[arg(long, env = "KNOT_EMBED_DIM", default_value_t = 384, hide = true)]
    pub embed_dim: u64,
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
    /// Whether to run in watch mode.
    pub watch: bool,
}

impl Config {
    /// Load configuration for the indexer binary (knot-indexer).
    /// Parses IndexerCli and includes all indexing-specific options.
    pub fn load_indexer() -> Result<Self> {
        Self::load_env_and_parse(IndexerCli::parse).map(
            |(cli, repo_path, repo_name, neo4j_password)| {
                let dependency_repos = if let Some(deps_str) = &cli.dependencies {
                    deps_str
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                } else {
                    Vec::new()
                };

                Self {
                    repo_path,
                    repo_name,
                    qdrant_url: cli.qdrant_url,
                    qdrant_collection: cli.qdrant_collection,
                    neo4j_uri: cli.neo4j_uri,
                    neo4j_user: cli.neo4j_user,
                    neo4j_password,
                    custom_queries_path: cli.custom_queries_path,
                    embed_dim: cli.embed_dim,
                    batch_size: cli.batch_size,
                    clean: cli.clean,
                    dependency_repos,
                    watch: cli.watch,
                }
            },
        )
    }

    /// Load configuration for the MCP server binary (knot-mcp).
    /// Parses McpCli and only includes MCP-relevant options.
    pub fn load_mcp() -> Result<Self> {
        Self::load_env_and_parse(McpCli::parse).map(
            |(cli, repo_path, repo_name, neo4j_password)| Self {
                repo_path,
                repo_name,
                qdrant_url: cli.qdrant_url,
                qdrant_collection: cli.qdrant_collection,
                neo4j_uri: cli.neo4j_uri,
                neo4j_user: cli.neo4j_user,
                neo4j_password,
                custom_queries_path: None,
                embed_dim: cli.embed_dim,
                batch_size: 0, // Not used by MCP
                clean: false,  // Not used by MCP
                dependency_repos: Vec::new(),
                watch: false, // Not used by MCP
            },
        )
    }

    /// Common shared logic for loading environment and resolving repo_path/repo_name.
    /// Takes a closure that parses the CLI arguments.
    fn load_env_and_parse<T, F>(parse_cli: F) -> Result<(T, String, String, String)>
    where
        T: HasCommonFields,
        F: Fn() -> T,
    {
        // Try to load .env — it is not an error if the file does not exist.
        match dotenvy::dotenv() {
            Ok(path) => tracing::info!("Loaded env from {}", path.display()),
            Err(dotenvy::Error::Io(_)) => {
                tracing::debug!("No .env file found, falling back to CLI arguments")
            }
            Err(e) => return Err(e).context("Failed to parse .env file"),
        }

        let cli = parse_cli();

        // Validate required fields that can come from CLI or Env.
        let neo4j_password = cli.neo4j_password()
            .or_else(|| std::env::var("KNOT_NEO4J_PASSWORD").ok())
            .context("Neo4j password is required. Provide it via --neo4j-password or KNOT_NEO4J_PASSWORD environment variable.")?;

        // Resolve repo_path: if not provided, use current directory and canonicalize
        let repo_path = if let Some(path) = cli.repo_path() {
            // If provided, canonicalize it to get an absolute path
            std::fs::canonicalize(&path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(path)
        } else {
            // Fallback: use current working directory
            std::env::current_dir()
                .context("Failed to determine current working directory for repo_path")?
                .to_string_lossy()
                .into_owned()
        };

        // Auto-detect repo_name from the resolved canonical repo_path if not provided
        let repo_name = if let Some(name) = cli.repo_name() {
            name
        } else {
            // Extract last component of the canonical repo_path as default
            std::path::Path::new(&repo_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| "unnamed-repo".to_string())
        };

        Ok((cli, repo_path, repo_name, neo4j_password))
    }
}

/// Trait to abstract common fields between IndexerCli and McpCli.
trait HasCommonFields {
    fn repo_path(&self) -> Option<String>;
    fn repo_name(&self) -> Option<String>;
    fn neo4j_password(&self) -> Option<String>;
}

impl HasCommonFields for IndexerCli {
    fn repo_path(&self) -> Option<String> {
        self.repo_path.clone()
    }

    fn repo_name(&self) -> Option<String> {
        self.repo_name.clone()
    }

    fn neo4j_password(&self) -> Option<String> {
        self.neo4j_password.clone()
    }
}

impl HasCommonFields for McpCli {
    fn repo_path(&self) -> Option<String> {
        self.repo_path.clone()
    }

    fn repo_name(&self) -> Option<String> {
        self.repo_name.clone()
    }

    fn neo4j_password(&self) -> Option<String> {
        self.neo4j_password.clone()
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
    fn test_indexer_cli_parsing_basic() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, Some("/tmp/repo".to_string()));
        assert_eq!(cli.neo4j_password, Some("secret".to_string()));
        assert_eq!(cli.qdrant_url, "http://localhost:6334"); // default
    }

    #[test]
    fn test_indexer_cli_parsing_full() {
        let args = vec![
            "knot-indexer",
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

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, Some("/tmp/repo".to_string()));
        assert_eq!(cli.repo_name, Some("my-repo".to_string()));
        assert_eq!(cli.qdrant_url, "http://qdrant:6334");
        assert_eq!(cli.qdrant_collection, "custom_collection");
        assert_eq!(cli.neo4j_uri, "bolt://neo4j:7687");
        assert_eq!(cli.neo4j_user, "admin");
        assert_eq!(cli.neo4j_password, Some("admin123".to_string()));
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
    fn test_indexer_cli_with_watch() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--watch",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert!(cli.watch);
    }

    #[test]
    fn test_indexer_cli_without_watch() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert!(!cli.watch);
    }

    #[test]
    fn test_indexer_cli_repo_path_optional() {
        let args = vec!["knot-indexer", "--neo4j-password", "secret"];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, None);
        assert_eq!(cli.neo4j_password, Some("secret".to_string()));
    }

    #[test]
    fn test_mcp_cli_parsing_basic() {
        let args = vec![
            "knot-mcp",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
        ];

        let cli = McpCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, Some("/tmp/repo".to_string()));
        assert_eq!(cli.neo4j_password, Some("secret".to_string()));
        assert_eq!(cli.qdrant_url, "http://localhost:6334"); // default
        assert_eq!(cli.embed_dim, 384); // default
    }

    #[test]
    fn test_mcp_cli_repo_path_optional() {
        let args = vec!["knot-mcp", "--neo4j-password", "secret"];

        let cli = McpCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, None);
        assert_eq!(cli.neo4j_password, Some("secret".to_string()));
    }

    #[test]
    fn test_mcp_cli_parsing_full() {
        let args = vec![
            "knot-mcp",
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
        ];

        let cli = McpCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, Some("/tmp/repo".to_string()));
        assert_eq!(cli.repo_name, Some("my-repo".to_string()));
        assert_eq!(cli.qdrant_url, "http://qdrant:6334");
        assert_eq!(cli.qdrant_collection, "custom_collection");
        assert_eq!(cli.neo4j_uri, "bolt://neo4j:7687");
        assert_eq!(cli.neo4j_user, "admin");
        assert_eq!(cli.neo4j_password, Some("admin123".to_string()));
        assert_eq!(cli.embed_dim, 768);
    }

    #[test]
    fn test_mcp_cli_no_indexer_specific_options() {
        // Verify that McpCli doesn't accept indexer-specific options
        let args = vec![
            "knot-mcp",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--watch",
        ];

        // This should fail because --watch is not a valid option for knot-mcp
        assert!(McpCli::try_parse_from(args).is_err());
    }

    #[test]
    fn test_indexer_cli_accepts_all_options() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--watch",
            "--clean",
            "--dependencies",
            "core-lib,shared-types",
            "--custom-queries-path",
            "/custom/queries",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.repo_path, Some("/tmp/repo".to_string()));
        assert!(cli.watch);
        assert!(cli.clean);
        assert_eq!(cli.dependencies, Some("core-lib,shared-types".to_string()));
        assert_eq!(cli.custom_queries_path, Some("/custom/queries".to_string()));
    }

    #[test]
    fn test_resolve_repo_path_with_dot() {
        // Test that passing "." gets canonicalized to an absolute path
        let repo_path_opt: Option<String> = Some(".".to_string());

        let repo_path = if let Some(path) = repo_path_opt {
            std::fs::canonicalize(&path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(path)
        } else {
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };

        // Should be an absolute path, not "."
        assert!(!repo_path.contains("."));
        assert!(std::path::Path::new(&repo_path).is_absolute());
    }

    #[test]
    fn test_resolve_repo_path_fallback_current_dir() {
        // Test that when repo_path is None, it defaults to current directory
        let repo_path_opt: Option<String> = None;

        let repo_path = if let Some(path) = repo_path_opt {
            std::fs::canonicalize(&path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(path)
        } else {
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };

        let expected = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        assert_eq!(repo_path, expected);
        assert!(std::path::Path::new(&repo_path).is_absolute());
    }

    #[test]
    fn test_repo_name_extraction_from_canonical_path() {
        // When repo_path is canonicalized, repo_name should extract correctly
        let current_dir = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let repo_name = std::path::Path::new(&current_dir)
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| "unnamed-repo".to_string());

        // Should get the actual directory name, not something empty or weird
        assert!(!repo_name.is_empty());
        assert!(
            repo_name != "unnamed-repo" || std::env::current_dir().unwrap().file_name().is_none()
        );
        assert!(!repo_name.contains("/"));
        assert!(!repo_name.contains("\\"));
    }

    #[test]
    fn test_repo_name_explicit_override() {
        // When --repo-name is provided, it should override auto-detection
        let repo_path = "/some/path/to/project";
        let repo_name_opt: Option<String> = Some("custom-repo-name".to_string());

        let repo_name = if let Some(name) = repo_name_opt {
            name
        } else {
            std::path::Path::new(repo_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_else(|| "unnamed-repo".to_string())
        };

        assert_eq!(repo_name, "custom-repo-name");
    }
}
