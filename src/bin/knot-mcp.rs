//! knot — MCP Server for Codebase Semantic Search & Exploration
//!
//! Provides three tools to LLM clients:
//! 1. `search_hybrid_context` — semantic + structural search with context expansion
//! 2. `find_callers` — reverse dependency lookup (who calls this entity?)
//! 3. `explore_file` — file structure exploration (classes, methods, functions)
//!
//! Server communicates via stdio (stdin/stdout) following the MCP protocol.

use rust_mcp_sdk::{
    McpServer, StdioTransport, ToMcpServerHandler, TransportOptions,
    error::SdkResult,
    mcp_server::{McpServerOptions, server_runtime},
    schema::*,
};
use tracing::info;

use knot::{config::Config, mcp_handler::KnotMcpHandler, utils};

#[tokio::main]
async fn main() -> SdkResult<()> {
    // Logging must be initialized before anything else.
    utils::init_logging().expect("Failed to initialize logging");

    // Load configuration (.env takes precedence over CLI args).
    let cfg = Config::load().expect("Failed to load configuration");

    info!("knot MCP server starting");
    info!("Repository path : {}", cfg.repo_path);
    info!(
        "Qdrant          : {} / {}",
        cfg.qdrant_url, cfg.qdrant_collection
    );
    info!("Neo4j           : {}", cfg.neo4j_uri);

    // ------------------------------------------------------------------ //
    // Initialize MCP handler with database connections                    //
    // ------------------------------------------------------------------ //

    let handler = KnotMcpHandler::new(
        &cfg.qdrant_url,
        &cfg.qdrant_collection,
        &cfg.neo4j_uri,
        &cfg.neo4j_user,
        &cfg.neo4j_password,
        cfg.embed_dim,
    )
    .await
    .expect("Failed to initialize MCP handler");

    info!("Databases initialized successfully");

    // ------------------------------------------------------------------ //
    // Create MCP server with stdio transport                              //
    // ------------------------------------------------------------------ //

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "knot".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            title: Some("knot Codebase Index".into()),
            description: Some(
                "Semantic search and structural exploration of indexed Java/TypeScript codebases"
                    .into(),
            ),
            icons: vec![],
            website_url: Some("https://github.com/anomalyco/knot".into()),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        protocol_version: ProtocolVersion::V2025_11_25.into(),
        instructions: Some(
            "Use the three available tools to search and explore an indexed codebase:\n\
             1. search_hybrid_context — find entities by semantic meaning with dependencies\n\
             2. find_callers — reverse dependency lookup (impact analysis)\n\
             3. explore_file — inspect file structure and entity declarations"
                .into(),
        ),
        meta: None,
    };

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
