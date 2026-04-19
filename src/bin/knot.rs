//! knot — CLI tool for semantic search and code exploration
//!
//! A standalone command-line interface for querying an indexed codebase.
//! Provides the same capabilities as the knot-mcp server via CLI commands.

use clap::{Parser, Subcommand};
use std::sync::{Arc, Mutex};

use knot::{
    cli_tools,
    config::Config,
    db::{graph::ConnectExt, vector::VectorConnectExt},
    pipeline::embed::Embedder,
    utils,
};

#[derive(Parser)]
#[command(name = "knot")]
#[command(about = "Semantic search and code exploration for indexed codebases", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search for code entities by semantic meaning
    Search {
        /// Search query (e.g., 'user authentication', 'API error handling')
        query: String,

        /// Maximum number of results to return (default: 5)
        #[arg(short, long, default_value = "5")]
        max_results: usize,

        /// Repository name to filter results
        #[arg(short, long)]
        repo: Option<String>,
    },

    /// Find all references to an entity (reverse dependency lookup)
    Callers {
        /// Entity name to find references for
        entity_name: String,

        /// Repository name to filter results
        #[arg(short, long)]
        repo: Option<String>,
    },

    /// Explore all entities in a source file
    Explore {
        /// Path to the source file
        file_path: String,

        /// Repository name to filter results
        #[arg(short, long)]
        repo: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging (CLI-specific: defaults to error level, not info)
    utils::init_logging_for_cli()?;

    // Parse CLI arguments
    let cli = Cli::parse();

    // Load configuration for CLI (.env takes precedence over environment variables)
    // Uses load_knot_cli() which properly handles the knot subcommand structure
    let cfg = Config::load_knot_cli().expect("Failed to load configuration");

    // Initialize database connections
    let vector_db = Arc::new(
        knot::db::vector::VectorDb::connect(&cfg.qdrant_url, &cfg.qdrant_collection, cfg.embed_dim)
            .await?,
    );

    let graph_db = Arc::new(
        knot::db::graph::GraphDb::connect(&cfg.neo4j_uri, &cfg.neo4j_user, &cfg.neo4j_password)
            .await?,
    );

    let embedder = Arc::new(Mutex::new(Embedder::init()?));

    // Execute the appropriate command
    match cli.command {
        Commands::Search {
            query,
            max_results,
            repo,
        } => {
            // Use provided repo or default to current repository name
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let result = cli_tools::run_search_hybrid_context(
                &query,
                max_results,
                Some(target_repo),
                &vector_db,
                &graph_db,
                &embedder,
            )
            .await?;
            println!("{}", result);
        }

        Commands::Callers { entity_name, repo } => {
            // Use provided repo or default to current repository name
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let result =
                cli_tools::run_find_callers(&entity_name, Some(target_repo), &graph_db).await?;
            println!("{}", result);
        }

        Commands::Explore { file_path, repo } => {
            // Use provided repo or default to current repository name
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let result =
                cli_tools::run_explore_file(&file_path, Some(target_repo), &graph_db).await?;
            println!("{}", result);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parser_search_command() {
        let args = vec!["knot", "search", "test query"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { query, .. } => {
                assert_eq!(query, "test query");
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_with_max_results() {
        let args = vec!["knot", "search", "test", "--max-results", "10"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { max_results, .. } => {
                assert_eq!(max_results, 10);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_with_repo() {
        let args = vec!["knot", "search", "test", "--repo", "my-repo"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { repo, .. } => {
                assert_eq!(repo, Some("my-repo".to_string()));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_callers_command() {
        let args = vec!["knot", "callers", "MyClass"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { entity_name, .. } => {
                assert_eq!(entity_name, "MyClass");
            }
            _ => panic!("Expected Callers command"),
        }
    }

    #[test]
    fn test_cli_parser_callers_with_repo() {
        let args = vec!["knot", "callers", "MyClass", "--repo", "my-repo"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { repo, .. } => {
                assert_eq!(repo, Some("my-repo".to_string()));
            }
            _ => panic!("Expected Callers command"),
        }
    }

    #[test]
    fn test_cli_parser_explore_command() {
        let args = vec!["knot", "explore", "src/main.java"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Explore { file_path, .. } => {
                assert_eq!(file_path, "src/main.java");
            }
            _ => panic!("Expected Explore command"),
        }
    }

    #[test]
    fn test_cli_parser_explore_with_repo() {
        let args = vec!["knot", "explore", "src/main.java", "--repo", "my-repo"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Explore { repo, .. } => {
                assert_eq!(repo, Some("my-repo".to_string()));
            }
            _ => panic!("Expected Explore command"),
        }
    }

    #[test]
    fn test_cli_parser_search_without_repo() {
        // Verify search command can be parsed without --repo
        let args = vec!["knot", "search", "test query"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { query, repo, .. } => {
                assert_eq!(query, "test query");
                assert_eq!(repo, None); // No repo specified, will use default
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_callers_without_repo() {
        // Verify callers command can be parsed without --repo
        let args = vec!["knot", "callers", "MyClass"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { repo, .. } => {
                assert_eq!(repo, None); // No repo specified, will use default
            }
            _ => panic!("Expected Callers command"),
        }
    }

    #[test]
    fn test_cli_parser_explore_without_repo() {
        // Verify explore command can be parsed without --repo
        let args = vec!["knot", "explore", "src/main.java"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Explore { file_path, repo } => {
                assert_eq!(file_path, "src/main.java");
                assert_eq!(repo, None); // No repo specified, will use default
            }
            _ => panic!("Expected Explore command"),
        }
    }
}
