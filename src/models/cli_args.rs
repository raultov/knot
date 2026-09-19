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

#[derive(Clone, PartialEq, Eq, Subcommand)]
pub enum Commands {
    /// Search for code entities by semantic meaning
    Search {
        /// Search query (e.g., 'user authentication', 'API error handling')
        query: String,

        /// Maximum number of results to return (default: 5, max: 100;
        /// larger values are clamped)
        #[arg(short, long, default_value = "5")]
        max_results: usize,

        /// Repository scope: one name, comma-separated list, or 'all'/'*'
        #[arg(short, long)]
        repo: Option<String>,

        /// Optional entity-kind filter: exact kinds ('rust_function',
        /// 'markdown_section', ...) or aliases ('definition', 'callable',
        /// 'class', 'type'). Comma-separated for multiple values.
        #[arg(short, long)]
        kinds: Option<String>,

        /// Optional path filter: repo-relative directory prefix ('src/api',
        /// matched on a path boundary) or glob ('src/**/*_test.rs')
        #[arg(short, long)]
        path: Option<String>,

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },

    /// Find all references to an entity (reverse dependency lookup)
    Callers {
        /// Entity name to find references for
        entity_name: String,

        /// Repository scope: one name, comma-separated list, or 'all'/'*'
        #[arg(short, long)]
        repo: Option<String>,

        /// Maximum number of resolved targets (default: 25, max: 500). Raise
        /// when the response reports a truncated target list.
        #[arg(short = 'm', long)]
        max_targets: Option<usize>,

        /// Optional entity-kind filter for target resolution: 'all'/'*' to
        /// disable the default code-only scope (docs/config/build metadata
        /// stay hidden by default), or exact kinds/aliases ('callable',
        /// 'config', 'rust_function', ...) comma-separated for more values.
        #[arg(short = 'k', long)]
        kinds: Option<String>,

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },

    /// Explore all entities in a source file
    Explore {
        /// Path to the source file
        file_path: String,

        /// Repository scope: one name, comma-separated list, or 'all'/'*'
        #[arg(short, long)]
        repo: Option<String>,

        /// Output format (default: table)
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
    },

    /// List the indexed files of a repository (read-only), optionally narrowed
    /// by a directory prefix or glob (`src/api`, `src/**/*_test.rs`)
    Files {
        /// Optional directory prefix or glob, repo-relative
        #[arg(short, long)]
        path: Option<String>,

        /// Repository scope: one name, comma-separated list, or 'all'/'*'
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

        /// Maximum depth for transitive dependencies (default: 3, max: 10;
        /// larger values are clamped).
        /// Applies to --reverse too: it follows dependents transitively.
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

    /// Print the active embedding model (resolved from KNOT_EMBED_MODEL)
    /// as `name dim default_collection`, one per line. Harness support.
    EmbedModel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parser_callers_command_with_kinds() {
        let args = vec!["knot", "callers", "Hikari", "--kinds", "all"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers {
                entity_name, kinds, ..
            } => {
                assert_eq!(entity_name, "Hikari");
                assert_eq!(kinds.as_deref(), Some("all"));
            }
            _ => panic!("Expected Callers command"),
        }

        // Absent `--kinds` keeps the None default (code-only scope).
        let args = vec!["knot", "callers", "Hikari"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers {
                entity_name,
                kinds,
                max_targets,
                ..
            } => {
                assert_eq!(entity_name, "Hikari");
                assert_eq!(kinds, None);
                assert_eq!(max_targets, None);
            }
            _ => panic!("Expected Callers command"),
        }

        // `-k` short flag and explicit kind lists parse too.
        let args = vec!["knot", "callers", "X", "-k", "build_dependency,callable"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { kinds, .. } => {
                assert_eq!(kinds.as_deref(), Some("build_dependency,callable"));
            }
            _ => panic!("Expected Callers command"),
        }
    }

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
    fn test_cli_parser_search_with_repo_list() {
        let args = vec!["knot", "search", "test", "--repo", "a,b"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { repo, .. } => {
                assert_eq!(repo, Some("a,b".to_string()));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_with_repo_all() {
        let args = vec!["knot", "search", "test", "--repo", "all"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { repo, .. } => {
                assert_eq!(repo, Some("all".to_string()));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_kinds_default_none() {
        let args = vec!["knot", "search", "test"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { kinds, .. } => assert!(kinds.is_none()),
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_with_kinds_alias() {
        let args = vec!["knot", "search", "test", "--kinds", "definition"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { kinds, .. } => {
                assert_eq!(kinds, Some("definition".to_string()));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parser_search_with_kinds_list() {
        let args = vec![
            "knot",
            "search",
            "test",
            "--kinds",
            "rust_function,markdown_section",
        ];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Search { kinds, .. } => {
                assert_eq!(kinds, Some("rust_function,markdown_section".to_string()));
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
    fn test_cli_parser_callers_with_repo_list() {
        let args = vec!["knot", "callers", "MyClass", "--repo", "a,b"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { repo, .. } => {
                assert_eq!(repo, Some("a,b".to_string()));
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
    fn test_cli_parser_callers_max_targets_defaults_to_none() {
        // Absent flag → None → the shared core falls back to the
        // 25-target default; the CLI must not pin its own default that could
        // drift from the MCP tool.
        let args = vec!["knot", "callers", "MyClass"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { max_targets, .. } => {
                assert_eq!(max_targets, None);
            }
            _ => panic!("Expected Callers command"),
        }
    }

    #[test]
    fn test_cli_parser_callers_with_max_targets() {
        let args = vec!["knot", "callers", "MyClass", "--max-targets", "500"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Callers { max_targets, .. } => {
                assert_eq!(max_targets, Some(500));
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
    fn test_cli_parser_files_defaults() {
        let args = vec!["knot", "files"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Files { path, repo, .. } => {
                assert_eq!(path, None);
                assert_eq!(repo, None);
            }
            _ => panic!("Expected Files command"),
        }
    }

    #[test]
    fn test_cli_parser_files_with_path_and_repo() {
        let args = vec![
            "knot",
            "files",
            "--path",
            "src/**/*_test.rs",
            "--repo",
            "my-repo",
        ];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Files { path, repo, .. } => {
                assert_eq!(path, Some("src/**/*_test.rs".to_string()));
                assert_eq!(repo, Some("my-repo".to_string()));
            }
            _ => panic!("Expected Files command"),
        }
    }

    #[test]
    fn test_cli_parser_explore_with_repo_list() {
        let args = vec!["knot", "explore", "src/main.java", "--repo", "a,b"];
        let cli = Cli::try_parse_from(args).expect("Failed to parse CLI");
        match cli.command {
            Commands::Explore { repo, .. } => {
                assert_eq!(repo, Some("a,b".to_string()));
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
