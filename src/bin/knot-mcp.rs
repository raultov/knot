//! knot — MCP Server for Codebase Semantic Search & Exploration
//!
//! Provides four tools to LLM clients:
//! 1. `search_hybrid_context` — semantic + structural search with context expansion
//! 2. `find_callers` — reverse dependency lookup (who calls this entity?)
//! 3. `explore_file` — file structure exploration (classes, methods, functions)
//! 4. `list_repo_dependencies` — cross-repository dependency graph
//!
//! Server communicates via stdio (stdin/stdout) following the MCP protocol.

use rust_mcp_sdk::{
    McpServer, StdioTransport, ToMcpServerHandler, TransportOptions,
    error::SdkResult,
    mcp_server::{McpServerOptions, server_runtime},
};
use tracing::info;

use knot::{config::Config, mcp_handler, utils};

#[tokio::main]
async fn main() -> SdkResult<()> {
    utils::init_logging().expect("Failed to initialize logging");

    let cfg = Config::load_mcp().expect("Failed to load configuration");

    utils::inject_custom_ca_certs(&cfg.custom_ca_certs);

    info!("knot MCP server starting");
    info!("Repository path : {}", cfg.repo_path);
    info!(
        "Qdrant          : {} / {}",
        cfg.qdrant_url, cfg.qdrant_collection
    );
    info!("Neo4j           : {}", cfg.neo4j_uri);

    let handler = if cfg.dry_run {
        info!("Running in dry-run mode (no database connections)");
        mcp_handler::KnotMcpHandler::new_dry_run()
    } else {
        let h = mcp_handler::KnotMcpHandler::new(
            &cfg.qdrant_url,
            &cfg.qdrant_collection,
            &cfg.neo4j_uri,
            &cfg.neo4j_user,
            &cfg.neo4j_password,
            cfg.embed_dim,
            knot::pipeline::state::fastembed_cache_dir(&cfg.repo_path),
        )
        .await
        .expect("Failed to initialize MCP handler");

        info!("Databases initialized successfully");
        h
    };

    let server_details = mcp_handler::build_server_details();

    let transport = StdioTransport::new(TransportOptions::default())?;

    let server = server_runtime::create_server(McpServerOptions {
        server_details,
        transport,
        handler: handler.to_mcp_server_handler(),
        task_store: None,
        client_task_store: None,
    });

    info!("MCP server listening on stdio");
    if let Err(start_error) = server.start().await {
        eprintln!(
            "{}",
            start_error
                .rpc_error_message()
                .unwrap_or(&start_error.to_string())
        );
    }

    Ok(())
}
