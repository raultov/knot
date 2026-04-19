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
    ) -> anyhow::Result<Self> {
        let vector_db = VectorDb::connect(qdrant_url, qdrant_collection, embed_dim).await?;
        let graph_db = GraphDb::connect(neo4j_uri, neo4j_user, neo4j_password).await?;
        let embedder = Embedder::init()?;

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
            _ => Err(CallToolError::unknown_tool(params.name)),
        }
    }
}
