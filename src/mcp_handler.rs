//! MCP Server Handler for knot.
//!
//! Implements the ServerHandler trait from rust-mcp-sdk and coordinates
//! all MCP tools that provide semantic search and structural exploration
//! of the indexed codebase.

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
/// Maintains connections to Qdrant (vector DB), Neo4j (graph DB),
/// and the fastembed model for runtime embeddings.
pub struct KnotMcpHandler {
    pub vector_db: Arc<VectorDb>,
    pub graph_db: Arc<GraphDb>,
    pub embedder: Arc<Mutex<Embedder>>,
}

impl KnotMcpHandler {
    /// Create a new handler with initialized database connections.
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
            vector_db: Arc::new(vector_db),
            graph_db: Arc::new(graph_db),
            embedder: Arc::new(Mutex::new(embedder)),
        })
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
        match params.name.as_str() {
            "search_hybrid_context" => SearchHybridContextTool::handle(params, self).await,
            "find_callers" => FindCallersTool::handle(params, self).await,
            "explore_file" => ExploreFileTool::handle(params, self).await,
            _ => Err(CallToolError::unknown_tool(params.name)),
        }
    }
}
