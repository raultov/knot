//! knot — CLI tool for semantic search and code exploration
//!
//! A standalone command-line interface for querying an indexed codebase.
//! Provides the same capabilities as the knot-mcp server via CLI commands.

use clap::Parser;
use std::sync::{Arc, Mutex};

use knot::{
    cli_tools,
    config::{Config, OutputFormat},
    db::{graph::ConnectExt, vector::VectorConnectExt},
    models::{Cli, Commands, RepoScope},
    pipeline::embed::Embedder,
    utils,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    utils::init_logging_for_cli()?;

    let cli = Cli::parse();

    let cfg = Config::load_knot_cli().expect("Failed to load configuration");

    utils::inject_custom_ca_certs(&cfg.custom_ca_certs);

    let vector_db = Arc::new(
        knot::db::vector::VectorDb::connect(&cfg.qdrant_url, &cfg.qdrant_collection, cfg.embed_dim)
            .await?,
    );

    let graph_db = Arc::new(
        knot::db::graph::GraphDb::connect(&cfg.neo4j_uri, &cfg.neo4j_user, &cfg.neo4j_password)
            .await?,
    );

    let embedder = Arc::new(Mutex::new(Embedder::init(
        knot::pipeline::state::fastembed_cache_dir(&cfg.repo_path),
    )?));

    match cli.command {
        Commands::Search {
            query,
            max_results,
            repo,
            output,
        } => {
            let target_repo = build_repo_scope(repo.as_deref(), &cfg.repo_name);
            let json_result = cli_tools::run_search_hybrid_context(
                &query,
                max_results,
                &target_repo,
                &cli_tools::SearchContext {
                    vector_db: &vector_db,
                    graph_db: &graph_db,
                    embedder: &embedder,
                },
            )
            .await?;
            let formatted = utils::format_output(json_result, output);
            utils::print_with_pager(&formatted);
        }

        Commands::Callers {
            entity_name,
            repo,
            output,
        } => {
            let target_repo = build_repo_scope(repo.as_deref(), &cfg.repo_name);
            let json_result =
                cli_tools::run_find_callers(&entity_name, &target_repo, &graph_db).await?;
            let formatted = utils::format_callers_output(&entity_name, json_result, output);
            utils::print_with_pager(&formatted);
        }

        Commands::Explore {
            file_path,
            repo,
            output,
        } => {
            let target_repo = build_repo_scope(repo.as_deref(), &cfg.repo_name);
            let (fp, json_result) =
                cli_tools::run_explore_file(&file_path, &target_repo, &graph_db).await?;
            let formatted = utils::format_explore_output(&fp, json_result, output);
            utils::print_with_pager(&formatted);
        }

        Commands::Deps {
            repo_name,
            depth,
            reverse,
            output,
        } => {
            let json_result = cli_tools::run_deps(&repo_name, depth, reverse, &graph_db).await?;
            match output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json_result).unwrap_or_default()
                    );
                }
                OutputFormat::Table | OutputFormat::Markdown => {
                    let formatted =
                        cli_tools::format_deps_output(&repo_name, reverse, &json_result);
                    utils::print_with_pager(&formatted);
                }
            }
        }

        Commands::Repos { filter, output } => {
            // `repos` only needs the graph database — no vector DB or embedder.
            // We keep the standard initialization above to avoid splitting
            // the binary's startup flow; the overhead is negligible compared
            // to the network round-trips that follow.
            let json_result = cli_tools::run_list_repos(filter.as_deref(), &graph_db).await?;
            let formatted = cli_tools::format_repos_output(&json_result, output);
            utils::print_with_pager(&formatted);
        }
    }

    Ok(())
}

/// Build a [`RepoScope`] from the parsed `--repo` flag and the configured default.
///
/// When the flag is present, [`RepoScope::parse`] owns splitting on `,` and the
/// `all`/`*` sentinel (single spelling for "every indexed repository"). When the
/// flag is absent, the working-directory default from `cfg.repo_name` is used so
/// existing CLI invocations keep their current behavior.
fn build_repo_scope(repo: Option<&str>, default: &str) -> RepoScope {
    repo.map(RepoScope::parse)
        .unwrap_or_else(|| RepoScope::One(default.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_flag_builds_scope_list() {
        let scope = build_repo_scope(Some("a,b"), "default-repo");
        assert_eq!(
            scope,
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn repo_flag_builds_scope_all() {
        let scope = build_repo_scope(Some("all"), "default-repo");
        assert_eq!(scope, RepoScope::All);
    }

    #[test]
    fn repo_flag_absent_uses_config_default() {
        let scope = build_repo_scope(None, "default-repo");
        assert_eq!(scope, RepoScope::One("default-repo".to_string()));
    }
}
