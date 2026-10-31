//! List Endpoints Tool
//!
//! Read-only catalogue of the HTTP endpoints the indexed repositories serve
//! (issue #33): `<METHOD> <path>` with the handler method each one routes
//! to, optionally narrowed by HTTP method and path.
//!
//! **Usage Guidelines:**
//! - Use this to answer "which endpoints does this service expose?" or
//!   "which handler serves `GET /api/owners/42`?".
//! - Then run `find_callers` on the handler for its callers inside the
//!   repository, or `explore_file` on its file.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::*;

use crate::mcp_handler::KnotMcpHandler;
use crate::mcp_tools::repo_scope_from_args;

/// Input contract for `list_endpoints`.
#[mcp_tool(
    name = "list_endpoints",
    title = "List HTTP endpoints",
    description = "Read-only catalogue of the HTTP endpoints the indexed repositories serve, each with the handler method it routes to. \
                   Answers 'which endpoints does this service expose?' and 'which handler serves GET /api/owners/42?'. \
                   \n\nCoverage: Java and Kotlin handlers annotated for JAX-RS (class/method @Path + @GET/@POST/@PUT/@DELETE/@PATCH/@HEAD/@OPTIONS) or Spring MVC (class @RequestMapping + @GetMapping/@PostMapping/@PutMapping/@DeleteMapping/@PatchMapping or @RequestMapping(method = ...)). \
                   Paths join the class-level and method-level paths, resolve string constants declared in the same file, and drop path-variable patterns ('{id: [0-9]+}' → '{id}'); a part that cannot be resolved is shown as '<expression>'. \
                   Paths are relative to the application: the context path (server.servlet.context-path, @ApplicationPath) is not applied to the listed paths. \
                   'ANY' marks a @RequestMapping without 'method'. Declarative HTTP clients (@FeignClient, @RegisterRestClient, @HttpExchange interfaces) are not endpoints: their methods, like RestTemplate / WebClient / RestClient call sites, are 'http_client_call' entities, linked to the endpoints they hit. \
                   \n\nUsage: Use this for API-surface questions; then run 'find_callers' on the handler FQN for its callers, or on an endpoint name ('GET /api/owners/{id}') for the handler it routes to. Every endpoint is also an 'http_endpoint' entity reachable from 'search_hybrid_context' and 'explore_file', and a handler's 'find_callers' lists the endpoints routing to it under 'HTTP routes' and the client calls hitting them under 'HTTP callers (heuristic)'. \
                   \n\nBehaviour & Return: Read-only query with no side effects. Returns a Markdown table with columns: Method, Path, Handler, Location, Repository, ordered by (repository, path, method). At most 1000 rows; the reply states when more matched. When nothing matches, returns 'No HTTP endpoints matched.'. \
                   \n\nParameter guidance: 'method' is optional and case-insensitive ('GET'); 'ANY' endpoints match every method. 'path' is optional and matches segment by segment as a prefix ('/api/owners' matches '/api/owners/{id}' but not '/api/ownership'); an endpoint's '{variable}' segment matches any concrete segment, so a concrete URL path finds its endpoint. 'repo_name' scopes the catalogue.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(JsonSchema)]
pub struct ListEndpointsTool {
    #[json_schema(
        description = "Optional HTTP method filter, case-insensitive (e.g. 'GET'). Endpoints mapped to every method ('ANY') always match.",
        min_length = 1,
        max_length = 16
    )]
    pub method: Option<String>,
    #[json_schema(
        description = "Optional path filter, matched segment by segment as a prefix (e.g. '/api/owners'). An endpoint's '{variable}' segment matches any concrete segment, so '/api/owners/42' finds 'GET /api/owners/{id}'.",
        min_length = 1,
        max_length = 500
    )]
    pub path: Option<String>,
    #[json_schema(
        description = "Optional but HIGHLY RECOMMENDED: repository scope. Accepts a single repository name (`'my-repo'`), a comma-separated list (`'repo-a,repo-b'`), or `'all'` (or `'*'`) to query every indexed repository.",
        min_length = 1,
        max_length = 255
    )]
    pub repo_name: Option<String>,
}

impl ListEndpointsTool {
    pub async fn handle(
        params: CallToolRequestParams,
        handler: &KnotMcpHandler,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        use crate::cli_tools;

        let args = params.arguments.unwrap_or_default();
        let method = args.get("method").and_then(|v| v.as_str());
        let path = args.get("path").and_then(|v| v.as_str());
        let repo = repo_scope_from_args(&args);

        let Some(graph_db) = handler.graph_db.as_ref() else {
            return Err(CallToolError::from_message(
                "Server running in offline mode - graph database not available".to_string(),
            ));
        };

        let json_result = cli_tools::run_list_endpoints(method, path, &repo, graph_db)
            .await
            .map_err(|e| CallToolError::from_message(format!("Query error: {e}")))?;

        Ok(CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent::new(
                cli_tools::format_list_endpoints_markdown(&json_result),
                None,
                None,
            ))],
            is_error: None,
            meta: None,
            structured_content: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_schema_has_optional_filters() {
        let tool = ListEndpointsTool::tool();
        assert_eq!(tool.name, "list_endpoints");
        let props = tool.input_schema.properties.unwrap();
        for key in ["method", "path", "repo_name"] {
            assert!(props.contains_key(key), "{key}");
            assert!(!tool.input_schema.required.contains(&key.to_string()));
        }
    }
}
