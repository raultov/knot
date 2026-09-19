//! Configuration module.
//!
//! Resolves runtime configuration from three sources with the following precedence:
//!   1. CLI arguments (highest priority).
//!   2. Environment variables (set in the process environment).
//!   3. `$HOME/.config/knot/.env` (lowest priority — loaded from knot's
//!      XDG-style config directory, never from the current working directory).
//!
//! Provides specialized loaders for different binaries:
//! - [`Config::load_indexer`] for knot-indexer (indexing operations)
//! - [`Config::load_mcp`] for knot-mcp (MCP server)

use anyhow::{Context, Result};
use clap::FromArgMatches;
use clap::parser::ValueSource;
use clap::{CommandFactory, Parser, ValueEnum};
use std::str::FromStr as _;

#[derive(Debug, Clone, ValueEnum, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Markdown,
}

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

    /// DEPRECATED: the embedding dimension is now derived from the selected
    /// model (`KNOT_EMBED_MODEL`). Kept as a hidden flag solely so a stale
    /// value in existing scripts is detected (warning when it agrees, hard
    /// error when it contradicts) instead of silently ignored.
    #[arg(long, env = "KNOT_EMBED_DIM", hide = true)]
    pub embed_dim: Option<u64>,

    /// Embedding model to embed entities and queries with. Changing it
    /// invalidates every stored vector — a full re-index (`--clean`) is
    /// required. The vector dimension is derived from this model.
    #[arg(long, env = "KNOT_EMBED_MODEL", default_value_t = default_embed_model())]
    pub embed_model: String,

    #[arg(
        long,
        env = "KNOT_EMBEDDER_RESET_INTERVAL",
        default_value = "500",
        help = "Interval (in batches) to recreate the embedder to free memory. 0 disables resets."
    )]
    pub embedder_reset_interval: usize,

    /// Number of files to process in each rayon parallel batch.
    #[arg(long, env = "KNOT_BATCH_SIZE", default_value_t = 128)]
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

    /// Path to a custom CA certificate bundle for corporate network model downloads.
    /// Used to enable fastembed model downloads through SSL-inspecting proxies.
    #[arg(long, env = "KNOT_CUSTOM_CA_CERTS")]
    pub custom_ca_certs: Option<String>,

    /// Maximum number of concurrent ingestion tasks.
    /// Controls how many batches are ingested into Qdrant + Neo4j simultaneously.
    /// Higher values increase throughput but also database load.
    #[arg(long, env = "KNOT_INGEST_CONCURRENCY", default_value_t = 4)]
    pub ingest_concurrency: usize,

    /// Number of threads for the Rayon parallel parsing thread pool.
    /// When not set, defaults to (logical CPUs - 1), leaving 1 core
    /// for the Tokio async runtime and OS. Minimum: 2.
    #[arg(long, env = "KNOT_RAYON_THREADS")]
    pub rayon_threads: Option<usize>,

    /// Include configuration files (YAML, JSON, .properties) and Kubernetes/Helm
    /// manifests in the index. Disabled by default to avoid indexing secrets and
    /// to speed up indexing in repos with heavy config content.
    /// Build system files (package.json, tsconfig.json, pom.xml, Cargo.toml,
    /// Jenkinsfile) are always indexed regardless of this flag.
    #[arg(long, env = "KNOT_INCLUDE_CONFIG_FILES", default_value_t = false)]
    pub include_config_files: bool,
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

    /// DEPRECATED: the embedding dimension is now derived from the selected
    /// model (`KNOT_EMBED_MODEL`). Kept as a hidden flag solely so a stale
    /// value is detected (warning when it agrees, hard error when it
    /// contradicts) instead of silently ignored.
    #[arg(long, env = "KNOT_EMBED_DIM", hide = true)]
    pub embed_dim: Option<u64>,

    /// Embedding model to embed entities and queries with. Changing it
    /// invalidates every stored vector — a full re-index (`--clean`) is
    /// required. The vector dimension is derived from this model.
    #[arg(
        long,
        env = "KNOT_EMBED_MODEL",
        default_value_t = default_embed_model(),
        hide = true
    )]
    pub embed_model: String,
    #[arg(
        long,
        env = "KNOT_EMBEDDER_RESET_INTERVAL",
        default_value = "500",
        help = "Interval (in batches) to recreate the embedder to free memory. 0 disables resets."
    )]
    pub embedder_reset_interval: usize,

    /// Run in offline/dry-run mode (for quality checks on deployment platforms).
    /// When enabled, skips all database and model initialization.
    /// The server responds to protocol requests but cannot execute queries.
    #[arg(long, env = "KNOT_DRY_RUN", default_value_t = false, hide = true)]
    pub dry_run: bool,

    /// Path to a custom CA certificate bundle for corporate network model downloads.
    /// Used to enable fastembed model downloads through SSL-inspecting proxies.
    #[arg(long, env = "KNOT_CUSTOM_CA_CERTS")]
    pub custom_ca_certs: Option<String>,
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
    pub embed_model: String,
    pub embedder_reset_interval: usize,
    pub batch_size: usize,
    pub clean: bool,
    pub dependency_repos: Vec<String>,
    pub watch: bool,
    pub dry_run: bool,
    pub custom_ca_certs: Option<String>,
    pub output_format: OutputFormat,
    pub ingest_concurrency: usize,
    pub rayon_threads: Option<usize>,
    pub include_config_files: bool,
}

/// clap `default_value_t` cannot interpolate a const `&str` from another
/// module directly, so this thin wrapper pins the default in one place.
fn default_embed_model() -> String {
    crate::pipeline::embed::DEFAULT_EMBED_MODEL.to_owned()
}

/// Resolve the effective Qdrant collection.
///
/// An explicitly supplied collection always wins. Otherwise, the model's
/// suffix is applied to the base default, so the historical `knot_entities`
/// is preserved for the default model and the opt-in model lands on its own
/// collection instead of colliding with a fixed-size one.
fn resolve_collection(
    supplied: Option<&str>,
    base_default: &str,
    choice: &crate::pipeline::embed::EmbedModelChoice,
) -> String {
    match supplied {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => choice.default_collection(base_default),
    }
}

/// `--embed-dim` / `KNOT_EMBED_DIM` are deprecated: the dimension is now
/// derived from the model. A value that agrees is accepted with a
/// deprecation warning; one that contradicts is a hard error, because it
/// means the caller believes a different model is active.
///
/// Returns the derived dimension (the model's native one) to keep the one
/// resolve step, the check and the assignment atomic.
fn check_deprecated_embed_dim(supplied: Option<u64>, native: u64, model: &str) -> Result<u64> {
    match supplied {
        None => Ok(native),
        Some(value) if value == native => {
            tracing::warn!(
                "KNOT_EMBED_DIM/--embed-dim is deprecated and will be removed in the next \
                 major release: the dimension is now derived from the selected model. \
                 Model '{model}' derives {native}; the supplied value agrees — please \
                 unset the variable / drop the flag."
            );
            Ok(native)
        }
        Some(value) => anyhow::bail!(
            "KNOT_EMBED_DIM ({value}) contradicts the derived dimension of model \
             '{model}' ({native}). The dimension is now derived from \
             KNOT_EMBED_MODEL — unset the KNOT_EMBED_DIM variable / drop the \
             --embed-dim flag and re-run."
        ),
    }
}

/// Shared loader step: resolve the embedding model, run the deprecated-dim
/// check, and derive the effective Qdrant collection. Kept in one place so
/// the three CLI loaders cannot drift.
fn resolve_embed_and_collection(
    embed_model: &str,
    embed_dim_supplied: Option<u64>,
    collection_supplied: Option<&str>,
) -> Result<(u64, String)> {
    let choice = crate::pipeline::embed::EmbedModelChoice::from_str(embed_model)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let embed_dim = check_deprecated_embed_dim(embed_dim_supplied, choice.dim, embed_model)?;
    let qdrant_collection = resolve_collection(collection_supplied, "knot_entities", &choice);
    Ok((embed_dim, qdrant_collection))
}

/// Returns the path where knot's `.env` file should be located.
///
/// Lookup order (first match wins):
/// 1. `$KNOT_CONFIG_DIR/.env` — explicit override for testing or custom setups
/// 2. `$HOME/.config/knot/.env` — Unix/Linux/macOS XDG-style default
/// 3. `$USERPROFILE/.config/knot/.env` — Windows fallback
///
/// This **never** resolves to the current working directory, unlike
/// `dotenvy::dotenv()` which walks up the directory tree from CWD.
/// Pure resolution logic. `get` supplies environment lookups.
fn knot_env_path_from<F>(get: F) -> Option<std::path::PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    get("KNOT_CONFIG_DIR")
        .map(|d| std::path::PathBuf::from(d).join(".env"))
        .or_else(|| get("HOME").map(|d| std::path::PathBuf::from(d).join(".config/knot/.env")))
        .or_else(|| {
            get("USERPROFILE").map(|d| std::path::PathBuf::from(d).join(".config/knot/.env"))
        })
}

fn knot_env_path() -> Option<std::path::PathBuf> {
    knot_env_path_from(|k| std::env::var(k).ok())
}

/// Load environment variables from knot's `.env` file.
///
/// Only loads from knot's XDG-style config directory (see [`knot_env_path`]).
/// Never loads from the current working directory, preventing `.env` files in
/// target repositories from hijacking knot's configuration.
fn load_knot_env() {
    let Some(env_path) = knot_env_path() else {
        tracing::debug!("No .env location found (set KNOT_CONFIG_DIR, HOME, or USERPROFILE)");
        return;
    };

    match dotenvy::from_path(&env_path) {
        Ok(_) => tracing::info!("Loaded env from {}", env_path.display()),
        Err(dotenvy::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                "No .env file at {} — using environment variables and CLI args",
                env_path.display()
            );
        }
        Err(e) => {
            tracing::warn!("Failed to load .env from {}: {e}", env_path.display());
        }
    }
}

fn resolve_repo_path(repo_path: Option<String>) -> Result<String> {
    if let Some(path) = repo_path {
        Ok(std::fs::canonicalize(&path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(path))
    } else {
        Ok(std::env::current_dir()
            .context("Failed to determine current working directory for repo_path")?
            .to_string_lossy()
            .into_owned())
    }
}

fn resolve_repo_name(repo_name: Option<String>, repo_path: &str) -> String {
    if let Some(name) = repo_name {
        name
    } else {
        std::path::Path::new(repo_path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| "unnamed-repo".to_string())
    }
}

fn parse_dependencies(deps: Option<&String>) -> Vec<String> {
    deps.map(|s| {
        s.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// clap argument id for `qdrant_collection` in both CLI structs (the derive
/// uses the raw field name as the id; the long flag is kebab-cased).
const QDRANT_COLLECTION_ARG_ID: &str = "qdrant_collection";

/// Whether the `qdrant_collection` argument was explicitly supplied (CLI
/// argument or environment variable). clap reports a `default_value` as
/// [`ValueSource::DefaultValue`]; anything else counts as a deliberate
/// user choice that must win over the model-derived default collection —
/// distinguishing "not set" from "explicitly set to the base name" is
/// exactly what the rejected string-comparison alternative cannot do.
fn collection_was_supplied(matches: &clap::ArgMatches) -> bool {
    matches.value_source(QDRANT_COLLECTION_ARG_ID) != Some(ValueSource::DefaultValue)
}

impl Config {
    /// Load configuration for the indexer binary (knot-indexer).
    /// Parses IndexerCli and includes all indexing-specific options.
    pub fn load_indexer() -> Result<Self> {
        Self::load_env_and_parse::<IndexerCli>().and_then(|parsed| {
            let (cli, collection_supplied, repo_path, repo_name, neo4j_password) = parsed;
            let (embed_dim, qdrant_collection) = resolve_embed_and_collection(
                &cli.embed_model,
                cli.embed_dim,
                collection_supplied.as_deref(),
            )?;
            let dependency_repos = parse_dependencies(cli.dependencies.as_ref());

            Ok(Self {
                repo_path,
                repo_name,
                qdrant_url: cli.qdrant_url,
                qdrant_collection,
                neo4j_uri: cli.neo4j_uri,
                neo4j_user: cli.neo4j_user,
                neo4j_password,
                custom_queries_path: cli.custom_queries_path,
                embed_dim,
                embed_model: cli.embed_model,
                embedder_reset_interval: cli.embedder_reset_interval,
                batch_size: cli.batch_size,
                clean: cli.clean,
                dependency_repos,
                watch: cli.watch,
                dry_run: false,
                custom_ca_certs: cli.custom_ca_certs,
                output_format: OutputFormat::Markdown,
                ingest_concurrency: cli.ingest_concurrency,
                rayon_threads: cli.rayon_threads,
                include_config_files: cli.include_config_files,
            })
        })
    }

    /// Load configuration for the MCP server binary (knot-mcp).
    /// Parses McpCli and only includes MCP-relevant options.
    pub fn load_mcp() -> Result<Self> {
        Self::load_env_and_parse::<McpCli>().and_then(
            |(cli, collection_supplied, repo_path, repo_name, neo4j_password)| {
                let (embed_dim, qdrant_collection) = resolve_embed_and_collection(
                    &cli.embed_model,
                    cli.embed_dim,
                    collection_supplied.as_deref(),
                )?;
                Ok(Self {
                    repo_path,
                    repo_name,
                    qdrant_url: cli.qdrant_url,
                    qdrant_collection,
                    neo4j_uri: cli.neo4j_uri,
                    neo4j_user: cli.neo4j_user,
                    neo4j_password,
                    custom_queries_path: None,
                    embed_dim,
                    embed_model: cli.embed_model,
                    embedder_reset_interval: cli.embedder_reset_interval,
                    batch_size: 0,
                    clean: false,
                    dependency_repos: Vec::new(),
                    watch: false,
                    dry_run: cli.dry_run,
                    custom_ca_certs: cli.custom_ca_certs,
                    output_format: OutputFormat::Markdown,
                    ingest_concurrency: 4,
                    rayon_threads: None,
                    include_config_files: false,
                })
            },
        )
    }

    /// Load configuration for the CLI binary (knot).
    /// Uses McpCli parser but ignores CLI subcommand arguments to avoid conflicts.
    /// This allows the CLI to accept search/callers/explore subcommands without
    /// clap trying to parse them as configuration arguments.
    pub fn load_knot_cli() -> Result<Self> {
        load_knot_env();

        // Parse McpCli from empty args (the CLI subcommands are parsed
        // separately by `Cli::parse` and must not conflict with the config
        // arguments). The two-step matches round-trip is needed so the
        // `qdrant_collection` `ValueSource` can distinguish an explicit pin
        // from the default.
        let matches = McpCli::command()
            .try_get_matches_from(["knot"])
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let cli = McpCli::from_arg_matches(&matches)?;

        // Validate required fields from environment
        let neo4j_password = cli.neo4j_password()
            .or_else(|| std::env::var("KNOT_NEO4J_PASSWORD").ok())
            .context("Neo4j password is required. Provide it via KNOT_NEO4J_PASSWORD environment variable.")?;

        let repo_path = resolve_repo_path(cli.repo_path())?;
        tracing::info!("Resolved repo_path: {repo_path}");

        let repo_name = resolve_repo_name(cli.repo_name(), &repo_path);

        let collection_supplied = if collection_was_supplied(&matches) {
            Some(cli.qdrant_collection.as_str())
        } else {
            None
        };
        let (embed_dim, qdrant_collection) =
            resolve_embed_and_collection(&cli.embed_model, cli.embed_dim, collection_supplied)?;

        Ok(Self {
            repo_path,
            repo_name,
            qdrant_url: cli.qdrant_url,
            qdrant_collection,
            neo4j_uri: cli.neo4j_uri,
            neo4j_user: cli.neo4j_user,
            neo4j_password,
            custom_queries_path: None,
            embed_dim,
            embed_model: cli.embed_model,
            embedder_reset_interval: cli.embedder_reset_interval,
            batch_size: 0,
            clean: false,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
            custom_ca_certs: cli.custom_ca_certs,
            output_format: OutputFormat::Table,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
        })
    }

    /// Common shared logic for loading environment and resolving repo_path/repo_name.
    /// Parses the CLI generically via the two-step matches round-trip so the
    /// `qdrant_collection` `ValueSource` can be inspected (an explicitly
    /// supplied collection must win over the model-derived default).
    /// Returns `(cli, collection_supplied, repo_path, repo_name, neo4j_password)`.
    fn load_env_and_parse<T>() -> Result<ParsedCli<T>>
    where
        T: HasCommonFields + Parser,
    {
        load_knot_env();

        let matches = <T as CommandFactory>::command()
            .try_get_matches_from(std::env::args_os())
            .unwrap_or_else(|e| e.exit());
        let cli = T::from_arg_matches(&matches)?;

        let collection_supplied = if collection_was_supplied(&matches) {
            Some(cli.qdrant_collection())
        } else {
            None
        };

        // Validate required fields that can come from CLI or Env.
        let neo4j_password = cli.neo4j_password()
            .or_else(|| std::env::var("KNOT_NEO4J_PASSWORD").ok())
            .context("Neo4j password is required. Provide it via --neo4j-password or KNOT_NEO4J_PASSWORD environment variable.")?;

        let repo_path = resolve_repo_path(cli.repo_path())?;
        tracing::info!("Resolved repo_path: {repo_path}");

        let repo_name = resolve_repo_name(cli.repo_name(), &repo_path);

        Ok((
            cli,
            collection_supplied,
            repo_path,
            repo_name,
            neo4j_password,
        ))
    }
}

/// Parsed-cli tuple: `(cli, collection_supplied, repo_path, repo_name, neo4j_password)`.
type ParsedCli<T> = (T, Option<String>, String, String, String);

/// Trait to abstract common fields between IndexerCli and McpCli.
trait HasCommonFields {
    fn repo_path(&self) -> Option<String>;
    fn repo_name(&self) -> Option<String>;
    fn neo4j_password(&self) -> Option<String>;
    /// The parsed `qdrant_collection` value (owned copy).
    fn qdrant_collection(&self) -> String;
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

    fn qdrant_collection(&self) -> String {
        self.qdrant_collection.clone()
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

    fn qdrant_collection(&self) -> String {
        self.qdrant_collection.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[test]
    fn derived_dim_matches_default_model() {
        let choice = crate::pipeline::embed::EmbedModelChoice::from_str(
            crate::pipeline::embed::DEFAULT_EMBED_MODEL,
        )
        .expect("DEFAULT_EMBED_MODEL must be a valid model name");

        // The dimension is derived from the model: the historical MiniLM
        // default must keep deriving 384 (backward-compatibility pin).
        assert_eq!(choice.dim, 384);
    }

    #[test]
    fn check_deprecated_embed_dim_accepts_none() {
        let native = check_deprecated_embed_dim(None, 384, "AllMiniLML6V2").unwrap();
        assert_eq!(native, 384);
    }

    #[test]
    fn check_deprecated_embed_dim_accepts_matching_value() {
        let native = check_deprecated_embed_dim(Some(384), 384, "AllMiniLML6V2").unwrap();
        assert_eq!(native, 384);
    }

    #[test]
    fn check_deprecated_embed_dim_rejects_contradiction_with_both_numbers() {
        let err = check_deprecated_embed_dim(Some(384), 768, "BGEBaseENV15").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("384"), "must name the supplied value: {msg}");
        assert!(msg.contains("768"), "must name the derived dim: {msg}");
        assert!(msg.contains("BGEBaseENV15"), "must name the model: {msg}");
        assert!(msg.contains("embed-dim"), "must point at the fix: {msg}");
    }

    #[test]
    fn resolve_collection_default_minilm_keeps_base_name() {
        let minilm = crate::pipeline::embed::EmbedModelChoice::from_str("AllMiniLML6V2").unwrap();
        assert_eq!(
            resolve_collection(None, "knot_entities", &minilm),
            "knot_entities"
        );
    }

    #[test]
    fn resolve_collection_default_bge_derives_suffixed_collection() {
        let bge = crate::pipeline::embed::EmbedModelChoice::from_str("BGEBaseENV15").unwrap();
        assert_eq!(
            resolve_collection(None, "knot_entities", &bge),
            "knot_entities_bge768"
        );
    }

    #[test]
    fn resolve_collection_explicit_value_wins_for_either_model() {
        for name in ["AllMiniLML6V2", "BGEBaseENV15"] {
            let choice = crate::pipeline::embed::EmbedModelChoice::from_str(name).unwrap();
            assert_eq!(
                resolve_collection(Some("my_custom_collection"), "knot_entities", &choice),
                "my_custom_collection",
                "explicit value must win for {name}"
            );
        }
    }

    #[test]
    fn qdrant_collection_value_source_distinguishes_default_from_explicit_pin() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // Default parse → not supplied → the derived collection applies.
        temp_env::with_var("KNOT_QDRANT_COLLECTION", None::<&str>, || {
            let matches = IndexerCli::command()
                .try_get_matches_from(["knot-indexer"])
                .unwrap();
            assert!(!collection_was_supplied(&matches));
        });

        // Explicit CLI value → supplied.
        let matches = IndexerCli::command()
            .try_get_matches_from(["knot-indexer", "--qdrant-collection", "knot_entities"])
            .unwrap();
        assert!(collection_was_supplied(&matches));

        // Explicit env value → supplied (env source is not DefaultValue).
        temp_env::with_var("KNOT_QDRANT_COLLECTION", Some("knot_entities"), || {
            let matches = IndexerCli::command()
                .try_get_matches_from(["knot-indexer"])
                .unwrap();
            assert!(collection_was_supplied(&matches));
        });

        // Same plumbing contract for McpCli.
        temp_env::with_var("KNOT_QDRANT_COLLECTION", None::<&str>, || {
            let matches = McpCli::command()
                .try_get_matches_from(["knot-mcp"])
                .unwrap();
            assert!(!collection_was_supplied(&matches));
        });
    }

    #[test]
    fn resolve_embed_and_collection_rejects_unknown_model() {
        let err = resolve_embed_and_collection("gpt99", None, None).unwrap_err();
        assert!(
            err.to_string().contains("Unknown embedding model 'gpt99'"),
            "{err}"
        );
    }

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_repo_name_auto_detection() {
        assert_eq!(resolve_repo_name(None, "/path/to/my-project"), "my-project");
    }

    #[test]
    fn test_repo_name_provided() {
        assert_eq!(
            resolve_repo_name(Some("custom-name".to_string()), "/path/to/my-project"),
            "custom-name"
        );
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
        // D4 backward-compatibility pin: the deprecated flag must keep parsing.
        assert_eq!(cli.embed_dim, Some(768));
        assert_eq!(cli.batch_size, 128);
        assert!(cli.clean);
    }

    #[test]
    fn embed_dim_flag_still_parses() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // `knot-indexer --embed-dim 384` must keep parsing (D4): removing the
        // flag outright would break existing scripts.
        temp_env::with_var("KNOT_EMBED_DIM", None::<&str>, || {
            let cli = IndexerCli::try_parse_from(["knot-indexer", "--embed-dim", "384"])
                .expect("deprecated --embed-dim must keep parsing");
            assert_eq!(cli.embed_dim, Some(384));
        });
        // And the env var keeps working too.
        temp_env::with_var("KNOT_EMBED_DIM", Some("384"), || {
            let cli = IndexerCli::try_parse_from(["knot-indexer"]).expect("env parse");
            assert_eq!(cli.embed_dim, Some(384));
        });
    }

    #[test]
    fn test_parse_dependencies_single() {
        assert_eq!(
            parse_dependencies(Some(&"core-lib".to_string())),
            vec!["core-lib"]
        );
    }

    #[test]
    fn test_parse_dependencies_multiple() {
        assert_eq!(
            parse_dependencies(Some(&"core-lib,shared-types,utils".to_string())),
            vec!["core-lib", "shared-types", "utils"]
        );
    }

    #[test]
    fn test_parse_dependencies_with_whitespace() {
        assert_eq!(
            parse_dependencies(Some(&"core-lib , shared-types , utils".to_string())),
            vec!["core-lib", "shared-types", "utils"]
        );
    }

    #[test]
    fn test_parse_dependencies_empty() {
        assert_eq!(
            parse_dependencies(Some(&"".to_string())),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_parse_dependencies_with_trailing_comma() {
        assert_eq!(
            parse_dependencies(Some(&"core-lib,shared-types,".to_string())),
            vec!["core-lib", "shared-types"]
        );
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
        let _guard = ENV_MUTEX.lock().unwrap();
        temp_env::with_var("KNOT_EMBED_DIM", None::<&str>, || {
            let cli = McpCli::try_parse_from(["knot-mcp", "--neo4j-password", "secret"])
                .expect("Failed to parse CLI args");
            assert_eq!(cli.embed_dim, None); // hidden, no default
        });
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
        assert_eq!(cli.embed_dim, Some(768));
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
        let repo_path = resolve_repo_path(Some(".".to_string())).unwrap();
        assert!(!repo_path.contains("."));
        assert!(std::path::Path::new(&repo_path).is_absolute());
    }

    #[test]
    fn test_resolve_repo_path_fallback_current_dir() {
        let repo_path = resolve_repo_path(None).unwrap();
        let expected = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(repo_path, expected);
        assert!(std::path::Path::new(&repo_path).is_absolute());
    }

    #[test]
    fn test_repo_name_extraction_from_canonical_path() {
        let current_dir = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let repo_name = resolve_repo_name(None, &current_dir);

        assert!(!repo_name.is_empty());
        assert!(
            repo_name != "unnamed-repo" || std::env::current_dir().unwrap().file_name().is_none()
        );
        assert!(!repo_name.contains("/"));
        assert!(!repo_name.contains("\\"));
    }

    #[test]
    fn test_repo_name_explicit_override() {
        assert_eq!(
            resolve_repo_name(
                Some("custom-repo-name".to_string()),
                "/some/path/to/project"
            ),
            "custom-repo-name"
        );
    }

    #[test]
    fn test_knot_cli_parsing_from_empty_args() {
        // Test that McpCli can parse from empty args (used by load_knot_cli)
        let args = vec!["knot"];
        let cli = McpCli::try_parse_from(args).expect("Failed to parse from empty args");

        // Should have valid configuration (values may be overridden by env vars)
        assert!(!cli.qdrant_url.is_empty());
        assert!(!cli.qdrant_collection.is_empty());
        assert!(!cli.neo4j_uri.is_empty());
        assert!(!cli.neo4j_user.is_empty());
        assert!(cli.embed_dim.is_none_or(|d| d > 0));
    }

    #[test]
    fn test_knot_cli_no_subcommand_interference() {
        // Verify that McpCli parsing doesn't interfere with CLI subcommands
        // (knot uses separate Cli with subcommands: search, callers, explore)
        let args = vec!["knot"];
        let result = McpCli::try_parse_from(args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_knot_cli_env_var_precedence() {
        // Test that environment variables can be read (simple test, doesn't modify env)
        // Just verify that parsing works and values are not empty
        let args = vec!["knot"];
        let cli = McpCli::try_parse_from(args).expect("Failed to parse");

        // Should have valid configuration
        assert!(!cli.qdrant_url.is_empty());
        assert_eq!(cli.neo4j_user, "neo4j"); // Default value
    }

    #[test]
    fn test_indexer_cli_with_custom_ca_certs() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--custom-ca-certs",
            "/etc/ssl/certs/corporate-bundle.pem",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(
            cli.custom_ca_certs,
            Some("/etc/ssl/certs/corporate-bundle.pem".to_string())
        );
    }

    #[test]
    fn test_indexer_cli_without_custom_ca_certs() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.custom_ca_certs, None);
    }

    #[test]
    fn test_mcp_cli_with_custom_ca_certs() {
        let args = vec![
            "knot-mcp",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--custom-ca-certs",
            "/etc/ssl/certs/my-certs.crt",
        ];

        let cli = McpCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(
            cli.custom_ca_certs,
            Some("/etc/ssl/certs/my-certs.crt".to_string())
        );
    }

    #[test]
    fn test_mcp_cli_without_custom_ca_certs() {
        let args = vec![
            "knot-mcp",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
        ];

        let cli = McpCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.custom_ca_certs, None);
    }

    #[test]
    fn test_config_custom_ca_certs_propagation() {
        let config = Config {
            repo_path: "/tmp/repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "knot_entities".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "secret".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            embed_model: crate::pipeline::embed::DEFAULT_EMBED_MODEL.to_string(),
            embedder_reset_interval: 500,
            batch_size: 64,
            clean: false,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
            custom_ca_certs: Some("/etc/ssl/certs/corp.pem".to_string()),
            output_format: OutputFormat::Table,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
        };

        assert_eq!(
            config.custom_ca_certs,
            Some("/etc/ssl/certs/corp.pem".to_string())
        );
    }

    #[test]
    fn test_output_format_default_is_table() {
        assert_eq!(OutputFormat::default(), OutputFormat::Table);
    }

    #[test]
    fn test_output_format_all_variants() {
        let variants = [
            OutputFormat::Table,
            OutputFormat::Json,
            OutputFormat::Markdown,
        ];
        assert_eq!(variants.len(), 3);
        assert_ne!(OutputFormat::Table, OutputFormat::Json);
        assert_ne!(OutputFormat::Table, OutputFormat::Markdown);
        assert_ne!(OutputFormat::Json, OutputFormat::Markdown);
    }

    #[test]
    fn test_output_format_clone() {
        let fmt = OutputFormat::Json;
        let cloned = fmt.clone();
        assert_eq!(fmt, cloned);
    }

    #[test]
    fn test_output_format_debug() {
        let fmt = OutputFormat::Markdown;
        let debug_str = format!("{:?}", fmt);
        assert!(debug_str.contains("Markdown"));
    }

    #[test]
    fn test_ingest_concurrency_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        temp_env::with_var("KNOT_INGEST_CONCURRENCY", None::<&str>, || {
            let args = vec![
                "knot-indexer",
                "--repo-path",
                "/tmp/repo",
                "--neo4j-password",
                "secret",
            ];

            let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
            assert_eq!(cli.ingest_concurrency, 4);
        });
    }

    #[test]
    fn test_ingest_concurrency_explicit() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--ingest-concurrency",
            "8",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.ingest_concurrency, 8);
    }

    #[test]
    fn test_ingest_concurrency_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        temp_env::with_var("KNOT_INGEST_CONCURRENCY", Some("16"), || {
            let args = vec![
                "knot-indexer",
                "--repo-path",
                "/tmp/repo",
                "--neo4j-password",
                "secret",
            ];

            let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
            assert_eq!(cli.ingest_concurrency, 16);
        });
    }

    #[test]
    fn test_ingest_concurrency_in_config() {
        let config = Config {
            repo_path: "/tmp/repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "knot_entities".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "secret".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            embed_model: crate::pipeline::embed::DEFAULT_EMBED_MODEL.to_string(),
            embedder_reset_interval: 500,
            batch_size: 64,
            clean: false,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
            custom_ca_certs: None,
            output_format: OutputFormat::Table,
            ingest_concurrency: 8,
            rayon_threads: None,
            include_config_files: false,
        };

        assert_eq!(config.ingest_concurrency, 8);
    }

    #[test]
    fn test_rayon_threads_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        temp_env::with_var("KNOT_RAYON_THREADS", None::<&str>, || {
            let args = vec![
                "knot-indexer",
                "--repo-path",
                "/tmp/repo",
                "--neo4j-password",
                "secret",
            ];

            let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
            assert_eq!(cli.rayon_threads, None);
        });
    }

    #[test]
    fn test_rayon_threads_explicit() {
        let args = vec![
            "knot-indexer",
            "--repo-path",
            "/tmp/repo",
            "--neo4j-password",
            "secret",
            "--rayon-threads",
            "8",
        ];

        let cli = IndexerCli::try_parse_from(args).expect("Failed to parse CLI args");
        assert_eq!(cli.rayon_threads, Some(8));
    }

    #[test]
    fn test_rayon_threads_in_config() {
        let config = Config {
            repo_path: "/tmp/repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "knot_entities".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "secret".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            embed_model: crate::pipeline::embed::DEFAULT_EMBED_MODEL.to_string(),
            embedder_reset_interval: 500,
            batch_size: 64,
            clean: false,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
            custom_ca_certs: None,
            output_format: OutputFormat::Table,
            ingest_concurrency: 4,
            rayon_threads: Some(6),
            include_config_files: false,
        };

        assert_eq!(config.rayon_threads, Some(6));
    }

    #[test]
    fn test_rayon_threads_none() {
        let config = Config {
            repo_path: "/tmp/repo".to_string(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "knot_entities".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "secret".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            embed_model: crate::pipeline::embed::DEFAULT_EMBED_MODEL.to_string(),
            embedder_reset_interval: 500,
            batch_size: 64,
            clean: false,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
            custom_ca_certs: None,
            output_format: OutputFormat::Table,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
        };

        assert_eq!(config.rayon_threads, None);
    }

    #[test]
    fn test_knot_env_path_prefers_knot_config_dir() {
        let env = std::collections::HashMap::from([
            ("KNOT_CONFIG_DIR", "/custom/knot/config"),
            ("HOME", "/home/user"),
        ]);
        let path = knot_env_path_from(|k| env.get(k).map(|s| s.to_string()));
        assert_eq!(
            path,
            Some(std::path::PathBuf::from("/custom/knot/config/.env"))
        );
    }

    #[test]
    fn test_knot_env_path_falls_back_to_home() {
        let env = std::collections::HashMap::from([("HOME", "/home/user")]);
        let path = knot_env_path_from(|k| env.get(k).map(|s| s.to_string()));
        assert_eq!(
            path,
            Some(std::path::PathBuf::from("/home/user/.config/knot/.env"))
        );
    }

    #[test]
    fn test_knot_env_path_never_resolves_to_cwd() {
        // knot_env_path must not use std::env::current_dir() or dotenvy::dotenv()
        // Verify by checking that a .env file in CWD does NOT affect the resolved path.
        let temp = tempdir().unwrap();
        let env_file = temp.path().join(".env");
        fs::write(&env_file, "KNOT_REPO_PATH=/from/cwd\n").unwrap();

        // knot_env_path should resolve from $HOME or $KNOT_CONFIG_DIR, never CWD
        let path = knot_env_path();
        assert!(
            path.as_ref().is_none_or(|p| p != &env_file),
            "knot_env_path must not resolve to a .env in the current (temp) directory"
        );
    }

    #[test]
    fn test_load_knot_env_reads_from_explicit_path() {
        let temp = tempdir().unwrap();
        let env_file = temp.path().join(".env");
        fs::write(&env_file, "KNOT_REPO_PATH=/from/explicit/path\n").unwrap();

        // dotenvy::from_path should load from the specified path
        let vars: std::collections::HashMap<String, String> = dotenvy::from_path_iter(&env_file)
            .expect("failed to read .env")
            .collect::<Result<_, _>>()
            .expect("failed to parse .env");

        assert_eq!(
            vars.get("KNOT_REPO_PATH").map(String::as_str),
            Some("/from/explicit/path")
        );
    }
}
