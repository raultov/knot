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

/// Injects custom CA certificates into the process environment for TLS connections.
///
/// This is required for `fastembed`/`hf-hub` to work through corporate SSL-inspecting proxies.
/// Must be called before any async runtime threads are spawned (i.e., early in `main()`).
///
/// # Safety
/// `std::env::set_var` is marked unsafe in Rust 2024 because concurrent modification
/// from multiple threads is a data race. This function is safe because:
/// - It is called exactly once, early in main(), before any Tokio threads exist.
/// - The tokio runtime is not yet running at this point.
#[inline(always)]
fn inject_custom_ca_certs(cert_path: &Option<String>) {
    if let Some(path) = cert_path {
        // SAFETY: This is safe because:
        // 1. Called before any threads exist (single-threaded main context)
        // 2. No other code can concurrently modify env vars at this point
        // 3. Tokio runtime hasn't been entered yet
        unsafe {
            std::env::set_var("SSL_CERT_FILE", path);
            std::env::set_var("SSL_CERT_DIR", path);
        }
        tracing::info!("Injected custom CA certificate path: {}", path);
    }
}

/// Build server details with configuration for the MCP server.
fn build_server_details() -> InitializeResult {
    InitializeResult {
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
    }
}

#[tokio::main]
async fn main() -> SdkResult<()> {
    // Logging must be initialized before anything else.
    utils::init_logging().expect("Failed to initialize logging");

    // Load configuration for MCP server (.env takes precedence over CLI args).
    let cfg = Config::load_mcp().expect("Failed to load configuration");

    // Inject custom CA certificates for fastembed/hf-hub model downloads if provided.
    // This must be called before any async/tokio threads are spawned.
    inject_custom_ca_certs(&cfg.custom_ca_certs);

    info!("knot MCP server starting");
    info!("Repository path : {}", cfg.repo_path);
    info!(
        "Qdrant          : {} / {}",
        cfg.qdrant_url, cfg.qdrant_collection
    );
    info!("Neo4j           : {}", cfg.neo4j_uri);

    // ------------------------------------------------------------------ //
    // Initialize MCP handler                                              //
    // ------------------------------------------------------------------ //

    let handler = if cfg.dry_run {
        info!("Running in dry-run mode (no database connections)");
        KnotMcpHandler::new_dry_run()
    } else {
        let h = KnotMcpHandler::new(
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
        h
    };

    // ------------------------------------------------------------------ //
    // Create MCP server with stdio transport                              //
    // ------------------------------------------------------------------ //

    let server_details = build_server_details();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_server_details_has_name() {
        let details = build_server_details();
        assert_eq!(details.server_info.name, "knot");
    }

    #[test]
    fn test_build_server_details_has_version() {
        let details = build_server_details();
        assert!(!details.server_info.version.is_empty());
        // Version should be a semantic version string like "0.4.3"
        assert!(details.server_info.version.contains('.'));
    }

    #[test]
    fn test_build_server_details_has_title() {
        let details = build_server_details();
        assert!(details.server_info.title.is_some());
        assert_eq!(
            details.server_info.title.as_ref().unwrap(),
            "knot Codebase Index"
        );
    }

    #[test]
    fn test_build_server_details_has_description() {
        let details = build_server_details();
        assert!(details.server_info.description.is_some());
        let desc = details.server_info.description.as_ref().unwrap();
        assert!(desc.contains("Semantic search"));
        assert!(desc.contains("Java/TypeScript"));
    }

    #[test]
    fn test_build_server_details_has_website() {
        let details = build_server_details();
        assert!(details.server_info.website_url.is_some());
        assert!(
            details
                .server_info
                .website_url
                .as_ref()
                .unwrap()
                .contains("github.com")
        );
    }

    #[test]
    fn test_build_server_details_has_tools_capability() {
        let details = build_server_details();
        assert!(details.capabilities.tools.is_some());
    }

    #[test]
    fn test_build_server_details_has_instructions() {
        let details = build_server_details();
        assert!(details.instructions.is_some());
        let instructions = details.instructions.as_ref().unwrap();
        assert!(instructions.contains("search_hybrid_context"));
        assert!(instructions.contains("find_callers"));
        assert!(instructions.contains("explore_file"));
    }

    #[test]
    fn test_build_server_details_protocol_version() {
        let details = build_server_details();
        // Just verify that protocol_version is set
        assert!(!details.protocol_version.is_empty());
    }
}
