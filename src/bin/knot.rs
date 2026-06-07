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
    models::{Cli, Commands},
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
            let formatted = utils::format_output(json_result, output);
            utils::print_with_pager(&formatted);
        }

        Commands::Callers {
            entity_name,
            repo,
            output,
        } => {
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let json_result =
                cli_tools::run_find_callers(&entity_name, Some(target_repo), &graph_db).await?;
            let formatted = utils::format_callers_output(&entity_name, json_result, output);
            utils::print_with_pager(&formatted);
        }

        Commands::Explore {
            file_path,
            repo,
            output,
        } => {
            let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);
            let (fp, json_result) =
                cli_tools::run_explore_file(&file_path, Some(target_repo), &graph_db).await?;
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
    }

    Ok(())
}
