//! CLI argument definitions for the `knot` CLI tool.
//!
//! Defines the `Cli` struct and `Commands` enum used by `clap` for parsing
//! subcommands: search, callers, explore, and deps.

use crate::config::OutputFormat;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "knot")]
#[command(about = "Semantic search and code exploration for indexed codebases", long_about = None)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
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

    /// Show dependency graph for a repository (forward and reverse DEPENDS_ON edges)
    Deps {
        /// Repository name to show dependencies for
        repo_name: String,

        /// Maximum depth for transitive dependencies (default: 3)
        #[arg(short, long, default_value = "3")]
        depth: u32,

        /// Show reverse dependencies (who depends on this repo)
        #[arg(long)]
        reverse: bool,

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },

    /// List all indexed repositories with their status (entity count, file count, build system, language)
    Repos {
        /// Filter repositories by name (case-insensitive substring match)
        #[arg(short, long)]
        filter: Option<String>,

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },

    /// Portfolio view across all indexed repos — state, correlations, Gemini recommendations
    Portfolio {
        /// Filter repositories by name (case-insensitive substring match)
        #[arg(short, long)]
        filter: Option<String>,

        /// Skip Gemini API call; return structured data only
        #[arg(long)]
        no_ai: bool,

        /// Additional repos to exclude (prowler is excluded by default)
        #[arg(long = "exclude", action = clap::ArgAction::Append)]
        exclude: Vec<String>,

        /// Write report to this file instead of the pager (e.g. portfolio-report.md)
        #[arg(long = "output-file")]
        output_file: Option<std::path::PathBuf>,

        /// Forecast horizon for strategic advisor (default: 18m; env: KNOT_PORTFOLIO_HORIZON)
        #[arg(long, default_value = "18m")]
        horizon: String,

        /// Engineering team size hint for resource planning (env: KNOT_PORTFOLIO_TEAM_SIZE)
        #[arg(long = "team-size")]
        team_size: Option<u32>,

        /// Strategic focus hint for the advisor (env: KNOT_PORTFOLIO_FOCUS)
        #[arg(long)]
        focus: Option<String>,

        /// Output format (default: markdown)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Markdown)]
        output: OutputFormat,
    },
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
    fn test_cli_parser_repos_command() {
        let args = vec!["knot", "repos"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Repos { filter, output } => {
                assert_eq!(filter, None);
                assert_eq!(output, OutputFormat::Table);
            }
            _ => panic!("Expected Repos command"),
        }
    }

    #[test]
    fn test_cli_parser_repos_with_output_format() {
        let args = vec!["knot", "repos", "--output", "json"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Repos { filter, output } => {
                assert_eq!(filter, None);
                assert_eq!(output, OutputFormat::Json);
            }
            _ => panic!("Expected Repos command"),
        }
    }

    #[test]
    fn test_cli_parser_repos_short_output_flag() {
        let args = vec!["knot", "repos", "-o", "markdown"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Repos { filter, output } => {
                assert_eq!(filter, None);
                assert_eq!(output, OutputFormat::Markdown);
            }
            _ => panic!("Expected Repos command"),
        }
    }

    #[test]
    fn test_cli_parser_repos_with_filter() {
        let args = vec!["knot", "repos", "--filter", "search_term"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Repos { filter, output } => {
                assert_eq!(filter, Some("search_term".to_string()));
                assert_eq!(output, OutputFormat::Table);
            }
            _ => panic!("Expected Repos command"),
        }
    }

    #[test]
    fn test_cli_parser_repos_with_filter_and_output() {
        let args = vec!["knot", "repos", "--filter", "app", "--output", "json"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Repos { filter, output } => {
                assert_eq!(filter, Some("app".to_string()));
                assert_eq!(output, OutputFormat::Json);
            }
            _ => panic!("Expected Repos command"),
        }
    }
}
