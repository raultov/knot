//! knot — CLI tool for semantic search and code exploration
//!
//! A standalone command-line interface for querying an indexed codebase.
//! Provides the same capabilities as the knot-mcp server via CLI commands.

use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use knot::{
    cli_tools,
    config::{Config, OutputFormat},
    db::{graph::ConnectExt, vector::VectorConnectExt},
    pipeline::embed::Embedder,
    utils,
};

#[inline(always)]
fn inject_custom_ca_certs(cert_path: &Option<String>) {
    if let Some(path) = cert_path {
        unsafe {
            std::env::set_var("SSL_CERT_FILE", path);
            std::env::set_var("SSL_CERT_DIR", path);
        }
        tracing::info!("Injected custom CA certificate path: {}", path);
    }
}

fn print_with_pager(content: &str) {
    if std::io::stdout().is_terminal()
        && let Ok(mut child) = Command::new("less")
            .arg("-R")
            .arg("-e")
            .stdin(Stdio::piped())
            .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(content.as_bytes());
        }
        let _ = child.wait();
        return;
    }
    println!("{}", content);
}

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

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },

    /// Find all references to an entity (reverse dependency lookup)
    Callers {
        /// Entity name to find references for
        entity_name: String,

        /// Repository name to filter results
        #[arg(short, long)]
        repo: Option<String>,

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },

    /// Explore all entities in a source file
    Explore {
        /// Path to the source file
        file_path: String,

        /// Repository name to filter results
        #[arg(short, long)]
        repo: Option<String>,

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },
}

fn format_output(json_value: serde_json::Value, output_format: OutputFormat) -> String {
    match output_format {
        OutputFormat::Table => {
            if json_value.is_null() {
                return "No matching code found for your query.".to_string();
            }
            cli_tools::formatters::format_search_table(&json_value)
        }
        OutputFormat::Json => serde_json::to_string_pretty(&json_value).unwrap_or_default(),
        OutputFormat::Markdown => cli_tools::formatters::format_search_results(&json_value),
    }
}

fn format_callers_output(
    entity_name: &str,
    json_value: serde_json::Value,
    output_format: OutputFormat,
) -> String {
    match output_format {
        OutputFormat::Table => {
            cli_tools::formatters::format_callers_table(entity_name, &json_value)
        }
        OutputFormat::Json => serde_json::to_string_pretty(&json_value).unwrap_or_default(),
        OutputFormat::Markdown => cli_tools::format_references_result(entity_name, &json_value),
    }
}

fn format_explore_output(
    file_path: &str,
    json_value: serde_json::Value,
    output_format: OutputFormat,
) -> String {
    match output_format {
        OutputFormat::Table => cli_tools::formatters::format_explore_table(file_path, &json_value),
        OutputFormat::Json => serde_json::to_string_pretty(&json_value).unwrap_or_default(),
        OutputFormat::Markdown => cli_tools::format_file_entities(file_path, &json_value),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    utils::init_logging_for_cli()?;

    let cli = Cli::parse();

    let cfg = Config::load_knot_cli().expect("Failed to load configuration");

    inject_custom_ca_certs(&cfg.custom_ca_certs);

    let vector_db = Arc::new(
        knot::db::vector::VectorDb::connect(&cfg.qdrant_url, &cfg.qdrant_collection, cfg.embed_dim)
            .await?,
    );

    let graph_db = Arc::new(
        knot::db::graph::GraphDb::connect(&cfg.neo4j_uri, &cfg.neo4j_user, &cfg.neo4j_password)
            .await?,
    );

    let embedder = Arc::new(Mutex::new(Embedder::init()?));

    match cli.command {
        Commands::Search {
            query,
            max_results,
            repo,
            output,
        } => {
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let json_result = cli_tools::run_search_hybrid_context(
                &query,
                max_results,
                Some(target_repo),
                &vector_db,
                &graph_db,
                &embedder,
            )
            .await?;
            let formatted = format_output(json_result, output);
            print_with_pager(&formatted);
        }

        Commands::Callers {
            entity_name,
            repo,
            output,
        } => {
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let json_result =
                cli_tools::run_find_callers(&entity_name, Some(target_repo), &graph_db).await?;
            let formatted = format_callers_output(&entity_name, json_result, output);
            print_with_pager(&formatted);
        }

        Commands::Explore {
            file_path,
            repo,
            output,
        } => {
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let (fp, json_result) =
                cli_tools::run_explore_file(&file_path, Some(target_repo), &graph_db).await?;
            let formatted = format_explore_output(&fp, json_result, output);
            print_with_pager(&formatted);
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
            Commands::Search { query, .. } => assert_eq!(query, "test query"),
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_with_max_results() {
        let args = vec!["knot", "search", "test", "--max-results", "10"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { max_results, .. } => assert_eq!(max_results, 10),
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
    fn test_cli_parser_search_with_output_format() {
        let args = vec!["knot", "search", "test", "--output", "json"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { output, .. } => {
                assert_eq!(output, OutputFormat::Json);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_default_output_table() {
        let args = vec!["knot", "search", "test query"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { output, .. } => {
                assert_eq!(output, OutputFormat::Table);
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
    fn test_cli_parser_callers_with_output_format() {
        let args = vec!["knot", "callers", "MyClass", "--output", "markdown"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { output, .. } => {
                assert_eq!(output, OutputFormat::Markdown);
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
    fn test_cli_parser_explore_with_output_format() {
        let args = vec!["knot", "explore", "src/main.java", "--output", "table"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Explore { output, .. } => {
                assert_eq!(output, OutputFormat::Table);
            }
            _ => panic!("Expected Explore command"),
        }
    }

    #[test]
    fn test_cli_parser_search_short_output_flag() {
        let args = vec!["knot", "search", "test", "-o", "json"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { output, .. } => {
                assert_eq!(output, OutputFormat::Json);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_format_output_null_returns_no_match_message() {
        let result = format_output(serde_json::Value::Null, OutputFormat::Table);
        assert_eq!(result, "No matching code found for your query.");
    }

    #[test]
    fn test_format_output_json_pretty_print() {
        let json = serde_json::json!({"name": "Test", "kind": "class"});
        let result = format_output(json.clone(), OutputFormat::Json);
        assert!(result.contains("Test"));
        assert!(result.contains("class"));
    }

    #[test]
    fn test_format_callers_output_json() {
        let json = serde_json::json!({
            "calls": [{"name": "caller1"}],
            "extends": [],
            "implements": [],
            "references": []
        });
        let result = format_callers_output("MyEntity", json, OutputFormat::Json);
        assert!(result.contains("caller1"));
    }

    #[test]
    fn test_format_explore_output_json() {
        let json = serde_json::json!([{"name": "MyClass", "kind": "class"}]);
        let result = format_explore_output("test.java", json, OutputFormat::Json);
        assert!(result.contains("MyClass"));
    }
}
