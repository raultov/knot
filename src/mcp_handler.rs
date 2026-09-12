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
    list_repo_dependencies::ListRepoDependenciesTool, list_repositories::ListRepositoriesTool,
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
    #[expect(
        clippy::too_many_arguments,
        reason = "Server constructor needs many dependencies"
    )]
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

/// Error text returned when a dry-run handler is asked to execute a tool.
pub const DRY_RUN_MESSAGE: &str = "Server is running in dry-run mode. Database connections are not available. \
     This mode is used for protocol validation and quality checks only.";

impl KnotMcpHandler {
    /// The canonical MCP tool surface, independent of any transport or runtime.
    ///
    /// `knot-mcp` serves it over stdio; embedders such as knot-server serve the
    /// same list over HTTP. Keeping one list here is what makes those surfaces
    /// identical by construction.
    pub fn tools() -> Vec<Tool> {
        vec![
            SearchHybridContextTool::tool(),
            FindCallersTool::tool(),
            ExploreFileTool::tool(),
            ListRepoDependenciesTool::tool(),
            ListRepositoriesTool::tool(),
        ]
    }

    /// Execute a tool call without an `Arc<dyn McpServer>`.
    ///
    /// The SDK's `ServerHandler` demands a runtime handle that no tool reads;
    /// this entry point drops it so an embedder can drive the tools from its
    /// own transport. `handle_call_tool_request` delegates here, so both paths
    /// are the same code.
    pub async fn dispatch(
        &self,
        params: CallToolRequestParams,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        // In dry-run mode, return error for tool execution
        if self.dry_run {
            return Err(CallToolError::from_message(DRY_RUN_MESSAGE.to_string()));
        }

        match params.name.as_str() {
            "search_hybrid_context" => SearchHybridContextTool::handle(params, self).await,
            "find_callers" => FindCallersTool::handle(params, self).await,
            "explore_file" => ExploreFileTool::handle(params, self).await,
            "list_repo_dependencies" => ListRepoDependenciesTool::handle(params, self).await,
            "list_repositories" => ListRepositoriesTool::handle(params, self).await,
            _ => Err(CallToolError::unknown_tool(params.name)),
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
            tools: Self::tools(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        self.dispatch(params).await
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
            "Use the available tools to search and explore an indexed codebase:\n\
             1. search_hybrid_context — find entities by semantic meaning with dependencies\n\
             2. find_callers — reverse dependency lookup (impact analysis)\n\
             3. explore_file — inspect file structure and entity declarations\n\
             4. list_repositories — list all indexed repositories with optional name filtering\n\
             Repository Scope: repo_name supports a single repo name, comma-separated list ('repo-a,repo-b'), sentinel 'all'/'*', or JSON string array."
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
        assert!(instructions.contains("list_repositories"));
    }

    #[test]
    fn test_build_server_details_protocol_version() {
        let details = build_server_details();
        assert!(!details.protocol_version.is_empty());
    }

    // --- Runtime-free MCP tool surface (KnotMcpHandler::tools / ::dispatch) ---

    const EXPECTED_TOOLS: [&str; 5] = [
        "search_hybrid_context",
        "find_callers",
        "explore_file",
        "list_repo_dependencies",
        "list_repositories",
    ];

    /// Build `CallToolRequestParams` without `Default` (the SDK type does not
    /// implement it), so the other tests stay readable.
    fn call_params(name: &str) -> CallToolRequestParams {
        call_params_with_args(name, serde_json::Map::new())
    }

    fn call_params_with_args(
        name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> CallToolRequestParams {
        CallToolRequestParams {
            name: name.to_string(),
            arguments: Some(arguments),
            meta: None,
            task: None,
        }
    }

    /// Extract the source between two markers of this very file. Used by the
    /// drift-guard tests: the trait methods must delegate, never re-inline the
    /// tool table or the dispatch match.
    fn source_between(start: &str, end: &str) -> String {
        let src = include_str!("mcp_handler.rs");
        let start_idx = src
            .find(start)
            .unwrap_or_else(|| panic!("source marker not found: {start}"));
        let rest = &src[start_idx..];
        let end_idx = rest
            .find(end)
            .unwrap_or_else(|| panic!("source marker not found: {end}"));
        rest[..end_idx].to_string()
    }

    #[test]
    fn tools_returns_the_full_surface() {
        let names: Vec<String> = KnotMcpHandler::tools()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, EXPECTED_TOOLS);
    }

    #[test]
    fn list_tools_request_delegates_to_tools_fn() {
        let body = source_between(
            "async fn handle_list_tools_request",
            "async fn handle_call_tool_request",
        );
        assert!(
            body.contains("Self::tools()"),
            "handle_list_tools_request must delegate to Self::tools()"
        );
        assert!(
            !body.contains("Tool::tool()"),
            "handle_list_tools_request must not re-inline the tool list"
        );
    }

    #[test]
    fn every_tool_has_name_description_and_object_schema() {
        for tool in KnotMcpHandler::tools() {
            assert!(!tool.name.is_empty(), "empty tool name");
            let desc = tool.description.as_deref().unwrap_or_default();
            assert!(!desc.is_empty(), "{} has no description", tool.name);
            let schema =
                serde_json::to_value(&tool.input_schema).expect("input schema must serialize");
            assert_eq!(schema["type"], "object", "{}", tool.name);
        }
    }

    #[test]
    fn dry_run_message_is_descriptive() {
        assert!(DRY_RUN_MESSAGE.contains("dry-run mode"));
        assert!(DRY_RUN_MESSAGE.contains("Database connections are not available"));
    }

    #[tokio::test]
    async fn dispatch_refuses_in_dry_run_mode() {
        let handler = KnotMcpHandler::new_dry_run();
        let err = handler
            .dispatch(call_params("search_hybrid_context"))
            .await
            .expect_err("dry-run must refuse");
        assert_eq!(err.to_string(), DRY_RUN_MESSAGE);
    }

    #[tokio::test]
    async fn dispatch_refuses_every_tool_in_dry_run_mode() {
        let handler = KnotMcpHandler::new_dry_run();
        for name in EXPECTED_TOOLS {
            let err = handler
                .dispatch(call_params(name))
                .await
                .expect_err("dry-run must refuse");
            assert_eq!(err.to_string(), DRY_RUN_MESSAGE, "tool {name}");
        }
    }

    #[tokio::test]
    async fn dispatch_rejects_unknown_tool() {
        // dry_run = false so the unknown-tool arm is reached rather than the guard.
        let handler = KnotMcpHandler {
            vector_db: None,
            graph_db: None,
            embedder: None,
            dry_run: false,
        };
        let err = handler
            .dispatch(call_params("no_such_tool"))
            .await
            .expect_err("unknown tool must fail");
        assert!(err.to_string().contains("no_such_tool"));

        // The SDK runtime performs this conversion and delivers the result as a
        // successful JSON-RPC response with is_error = true; embedders must
        // replicate it, so pin it here.
        let result: CallToolResult = err.into();
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn offline_handler_reports_missing_databases_not_dry_run() {
        let handler = KnotMcpHandler {
            vector_db: None,
            graph_db: None,
            embedder: None,
            dry_run: false,
        };
        // The tool validates its arguments before the database guard, so a
        // well-formed call is needed to reach the offline branch.
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), "anything".into());
        let err = handler
            .dispatch(call_params_with_args("search_hybrid_context", args))
            .await
            .expect_err("offline must fail");
        let msg = err.to_string();
        assert_ne!(msg, DRY_RUN_MESSAGE);
        assert!(msg.contains("offline mode"), "unexpected message: {msg}");
    }

    #[test]
    fn call_tool_request_delegates_to_dispatch() {
        let body = source_between(
            "async fn handle_call_tool_request",
            "/// Build the MCP server details",
        );
        assert!(
            body.contains("self.dispatch("),
            "handle_call_tool_request must delegate to self.dispatch()"
        );
        assert!(
            !body.contains("SearchHybridContextTool::handle"),
            "handle_call_tool_request must not re-inline the dispatch match"
        );
    }
}
