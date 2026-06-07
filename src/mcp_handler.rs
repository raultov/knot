//! MCP Server Handler for knot.
//!
//! Implements the ServerHandler trait from rust-mcp-sdk and coordinates
//! all MCP tools that provide semantic search and structural exploration
//! of the indexed codebase.
//!
//! Supports both online mode (with database connections) and offline/dry-run mode
//! (for quality checks and testing).

use async_trait::async_trait;
use rust_mcp_sdk::{McpServer, mcp_server::ServerHandler, schema::*};
use std::sync::{Arc, Mutex};

use crate::db::{
    graph::{ConnectExt, GraphDb},
    vector::{VectorConnectExt, VectorDb},
};
use crate::mcp_tools::{
    explore_file::ExploreFileTool, find_callers::FindCallersTool,
    list_repo_dependencies::ListRepoDependenciesTool,
    search_hybrid_context::SearchHybridContextTool,
};
use crate::pipeline::embed::Embedder;

/// Main handler for the knot MCP server.
///
/// Maintains optional connections to Qdrant (vector DB), Neo4j (graph DB),
/// and the fastembed model for runtime embeddings.
///
/// When running in dry-run mode (dry_run=true), these connections
/// are not initialized, allowing the server to respond to protocol requests
/// without database/model dependencies.
pub struct KnotMcpHandler {
    pub vector_db: Option<Arc<VectorDb>>,
    pub graph_db: Option<Arc<GraphDb>>,
    pub embedder: Option<Arc<Mutex<Embedder>>>,
    pub dry_run: bool,
}

impl KnotMcpHandler {
    /// Create a new handler with initialized database connections (online mode).
    pub async fn new(
        qdrant_url: &str,
        qdrant_collection: &str,
        neo4j_uri: &str,
        neo4j_user: &str,
        neo4j_password: &str,
        embed_dim: u64,
        cache_dir: std::path::PathBuf,
    ) -> anyhow::Result<Self> {
        let vector_db = VectorDb::connect(qdrant_url, qdrant_collection, embed_dim).await?;
        let graph_db = GraphDb::connect(neo4j_uri, neo4j_user, neo4j_password).await?;
        let embedder = Embedder::init(cache_dir)?;

        Ok(Self {
            vector_db: Some(Arc::new(vector_db)),
            graph_db: Some(Arc::new(graph_db)),
            embedder: Some(Arc::new(Mutex::new(embedder))),
            dry_run: false,
        })
    }

    /// Create a new handler in dry-run mode (for quality checks and testing).
    /// Skips database and model initialization entirely.
    pub fn new_dry_run() -> Self {
        Self {
            vector_db: None,
            graph_db: None,
            embedder: None,
            dry_run: true,
        }
    }
}

#[async_trait]
impl ServerHandler for KnotMcpHandler {
    async fn handle_list_tools_request(
        &self,
        _request: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> std::result::Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: vec![
                SearchHybridContextTool::tool(),
                FindCallersTool::tool(),
                ExploreFileTool::tool(),
                ListRepoDependenciesTool::tool(),
            ],
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        // In dry-run mode, return error for tool execution
        if self.dry_run {
            return Err(CallToolError::from_message(
                "Server is running in dry-run mode. Database connections are not available. \
                 This mode is used for protocol validation and quality checks only."
                    .to_string(),
            ));
        }

        match params.name.as_str() {
            "search_hybrid_context" => SearchHybridContextTool::handle(params, self).await,
            "find_callers" => FindCallersTool::handle(params, self).await,
            "explore_file" => ExploreFileTool::handle(params, self).await,
            "list_repo_dependencies" => ListRepoDependenciesTool::handle(params, self).await,
            _ => Err(CallToolError::unknown_tool(params.name)),
        }
    }
}

/// Build the MCP server details for the `initialize` handshake response.
pub fn build_server_details() -> InitializeResult {
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
        assert!(!details.protocol_version.is_empty());
    }
}
