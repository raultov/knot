//! Search Hybrid Context Tool
//!
//! Performs a hybrid search combining:
//! 1. Semantic search via Qdrant vector similarity (understands code meaning)
//! 2. Structural expansion via Neo4j graph relationships (understands architecture)
//!
//! **Key Capabilities:**
//! - **Semantic Code Search**: Find code by what it does (not just keywords)
//! - **Comment Search**: Search through docstrings and inline comments
//! - **Class/Interface Search**: Find specific class names or interface definitions
//! - **Method/Function Lookup**: Locate methods and functions by name or behavior
//! - **Architectural Pattern Search**: Discover design patterns and architectural structures
//! - **Dependency Context**: Get full dependency chains and architectural relationships
//! - **Multi-language Support**: Works with Java and TypeScript codebases

pub mod format;

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::*;

use crate::cli_tools;
use crate::cli_tools::DEFAULT_MAX_RESULTS;
use crate::mcp_handler::KnotMcpHandler;
use crate::mcp_tools::repo_scope_from_args;

/// Input contract for `search_hybrid_context`.
///
/// The `#[mcp_tool]` macro derives `SearchHybridContextTool::tool()` from this
/// declaration, so the JSON Schema advertised over MCP stays in lockstep with
/// the fields documented here.
#[mcp_tool(
    name = "search_hybrid_context",
    title = "Hybrid semantic + structural search",
    description = "Read-only semantic and structural code search combining vector embeddings with graph analysis. Use this for initial codebase discovery to find features by their meaning (e.g., 'user authentication'). \
                   Locates code based on natural language descriptions instead of exact keywords, returning relevant files, signatures, and documentation. \
                   \n\n⚠️ PREREQUISITE: This tool requires an active knot-mcp server with vector database (Qdrant) and graph database (Neo4j) initialized. \
                   \n\nBehavior & Return: Performs a read-only dual query against vector DB (for semantic similarity) and graph DB (for architectural relationships). \
                   Returns Markdown-formatted results with file paths, line numbers, code snippets, and cross-repository dependencies. No side effects. \
                   \n\nUsage: Use as your FIRST step when exploring unfamiliar code or discovering architectural patterns. Do NOT use this to find all usages of a specific function—use the 'find_callers' tool for that instead. \
                   \n\nRanking contract: results are kind-aware — function/method/class/struct definitions outrank markdown docs, test files, config properties and build-dependency entities for natural-language queries; callers and helpers appear as context attached to a definition, never as substitutes. The shared entry point of the highest-ranked helpers outranks those helpers a loose paraphrase surfaces. \
                   \n\nGeneric-verb guard: an entity merely named after a generic verb or noun (find/get/create/build/acquire/borrow/current/…) does not win on that name alone; the full name boost is paid only when the entity's container context (FQN) corroborates a second query token. \
                   \n\nRecall contract: entity embeds carry identifier tokens and the tokenized call names of the entity's body, so a paraphrase of what a definition does (even one with no doc comment) still surfaces it. Entities whose identifier shares a word with the query enter the candidate pool by token match alone. \
                   \n\nResult bound: 'max_results' is 1-100 (default 5) and is enforced — a larger request is clamped to 100 and the reply says so. There is no pagination: when the bound is not enough, narrow the scope with 'kinds' / 'path' / 'repo_name' or refine the query rather than raising the limit. \
                   \n\nParameter guidance: 'query' should be 2-5 words describing functionality. Increase 'max_results' to 10-20 for broad discovery, keep at 5 for focused search. Include 'repo_name' in your first query to avoid cross-repository pollution. \
                   \n\nSupports Java, Kotlin, C#, and TypeScript codebases.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(JsonSchema)]
pub struct SearchHybridContextTool {
    #[json_schema(
        description = "Search query describing what you're looking for (e.g., 'user authentication', 'API error handling')",
        min_length = 1,
        max_length = 500
    )]
    pub query: String,
    #[json_schema(
        description = "Maximum number of results to return (default: 5, max: 100). Requests above 100 are clamped to 100 and the reply says so — there is no cursor or pagination; to look past the bound, narrow the search with 'kinds' / 'path' / 'repo_name' or refine the query.",
        minimum = 1,
        maximum = 100,
        default = 5
    )]
    pub max_results: Option<i64>,
    #[json_schema(
        description = "Optional but HIGHLY RECOMMENDED: repository scope. Accepts a single repository name (`'my-repo'`), a comma-separated list (`'repo-a,repo-b'`), or `'all'` (or `'*'`) to query every indexed repository. If you know the repository you are working on, include it in your FIRST query to avoid mixed results from other indexed projects. Omit to search across all repositories.",
        min_length = 1,
        max_length = 255
    )]
    pub repo_name: Option<String>,
    #[json_schema(
        description = "Optional entity-kind filter. Accepts exact wire-format kinds (`'rust_function'`, `'markdown_section'`, `'kotlin_class'`, …) or aliases: `'definition'` (all functions/methods/types), `'callable'`/`'function'`/`'method'` (all callable kinds), `'class'`/`'type'`/`'struct'` (all type kinds). Comma-separate for multiple values. Omit to search all kinds.",
        min_length = 1,
        max_length = 255
    )]
    pub kinds: Option<String>,
    #[json_schema(
        description = "Optional path filter. A repo-relative directory prefix ('src/api', matched on a path boundary so 'src/api-notes.md' never matches) or a glob ('src/**/*_test.rs'). Use 'list_files' first when you do not know the layout. Omit to search every file.",
        min_length = 1,
        max_length = 500
    )]
    pub path: Option<String>,
}

/// Read `max_results` from the call arguments and resolve it against the
/// advertised bound via the shared resolver
/// ([`cli_tools::resolve_max_results`]).
///
/// Absent parameter → [`DEFAULT_MAX_RESULTS`]; a negative request floors at
/// 1; a request above [`MAX_RESULTS_CEILING`] clamps to it — check
/// [`ResolvedLimit::was_clamped`] / [`ResolvedLimit::notice`] and tell the
/// caller, never silently.
pub(crate) fn parse_max_results(
    args: &serde_json::Map<String, serde_json::Value>,
) -> cli_tools::ResolvedLimit {
    let requested = args
        .get("max_results")
        .and_then(|v| v.as_i64())
        .map(|v| v.max(0) as usize);
    match requested {
        Some(n) => cli_tools::resolve_max_results(n),
        None => cli_tools::resolve_max_results(DEFAULT_MAX_RESULTS),
    }
}

impl SearchHybridContextTool {
    pub async fn handle(
        params: CallToolRequestParams,
        handler: &KnotMcpHandler,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        let args = params
            .arguments
            .ok_or_else(|| CallToolError::from_message("Missing arguments".to_string()))?;

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CallToolError::from_message("Missing 'query' parameter".to_string()))?;

        // Parse and enforce the advertised bound through the single shared
        // resolver so the handler can never drift from the schema.
        let limit = parse_max_results(&args);

        let repo = repo_scope_from_args(&args);
        let kinds = args.get("kinds").and_then(|v| v.as_str());
        let path = args.get("path").and_then(|v| v.as_str());

        // Check if in offline mode
        if let (None, None, None) = (&handler.vector_db, &handler.graph_db, &handler.embedder) {
            return Err(CallToolError::from_message(
                "Server running in offline mode - databases not available".to_string(),
            ));
        }

        // Extract references (must be done before await to avoid Send issues)
        let vector_db = handler
            .vector_db
            .as_ref()
            .ok_or_else(|| CallToolError::from_message("Vector DB not available".to_string()))?;
        let graph_db = handler
            .graph_db
            .as_ref()
            .ok_or_else(|| CallToolError::from_message("Graph DB not available".to_string()))?;
        let embedder = handler
            .embedder
            .as_ref()
            .ok_or_else(|| CallToolError::from_message("Embedder not available".to_string()))?;

        // Call the shared CLI tool logic
        let json_result = cli_tools::run_search_hybrid_context(
            query,
            limit.value,
            &repo,
            cli_tools::SearchFilters { kinds, path },
            &cli_tools::SearchContext {
                vector_db,
                graph_db,
                embedder,
            },
        )
        .await
        .map_err(|e| CallToolError::from_message(format!("Search failed: {}", e)))?;

        // Surface a clamp to the caller instead of silently adjusting the
        // advertised contract.
        let mut formatted = format::format_search_results(&json_result);
        if let Some(notice) = limit.notice() {
            formatted.push('\n');
            formatted.push_str(&notice);
        }

        Ok(CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent::new(
                formatted, None, None,
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
    use crate::cli_tools::MAX_RESULTS_CEILING;

    #[test]
    fn test_search_hybrid_context_tool_schema() {
        let tool = SearchHybridContextTool::tool();
        assert_eq!(tool.name, "search_hybrid_context");
        assert!(tool.description.is_some());

        let schema = tool.input_schema;
        assert!(schema.required.contains(&"query".to_string()));

        let props = schema.properties.unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("max_results"));
        assert!(props.contains_key("repo_name"));
        assert!(props.contains_key("kinds"));
    }

    #[test]
    fn schema_maximum_equals_enforced_ceiling() {
        // Drift guard: the JSON Schema `maximum` advertised over MCP must
        // equal the clamp the shared core actually enforces.
        let tool = SearchHybridContextTool::tool();
        let props = tool.input_schema.properties.unwrap();
        let max_prop = props.get("max_results").unwrap();
        let maximum = max_prop.get("maximum").unwrap().as_i64().unwrap();
        assert_eq!(maximum, MAX_RESULTS_CEILING as i64);
    }

    #[test]
    fn schema_default_equals_default_constant() {
        let tool = SearchHybridContextTool::tool();
        let props = tool.input_schema.properties.unwrap();
        let default = props
            .get("max_results")
            .unwrap()
            .get("default")
            .unwrap()
            .as_i64()
            .unwrap();
        assert_eq!(default, DEFAULT_MAX_RESULTS as i64);
    }

    #[test]
    fn description_documents_bound_and_absence_of_pagination() {
        let desc = SearchHybridContextTool::tool()
            .description
            .unwrap()
            .to_string();
        assert!(desc.contains("Result bound: 'max_results' is 1-100"));
        assert!(desc.contains("no pagination"));
    }

    #[test]
    fn parse_max_results_defaults_when_absent() {
        let args = serde_json::json!({"query": "x"});
        let map = args.as_object().unwrap();
        let limit = parse_max_results(map);
        assert_eq!(limit.value, DEFAULT_MAX_RESULTS);
        assert!(!limit.was_clamped());
    }

    #[test]
    fn parse_max_results_clamps_above_ceiling() {
        let args = serde_json::json!({"query": "x", "max_results": 1000});
        let limit = parse_max_results(args.as_object().unwrap());
        assert_eq!(limit.value, MAX_RESULTS_CEILING);
        assert!(limit.was_clamped());
        assert!(limit.notice().is_some());
    }

    #[test]
    fn parse_max_results_floors_negative_on_one() {
        let args = serde_json::json!({"query": "x", "max_results": -3});
        let limit = parse_max_results(args.as_object().unwrap());
        assert_eq!(limit.value, 1);
        assert!(limit.was_clamped());
    }

    #[test]
    fn parse_max_results_preserves_in_bound_request() {
        let args = serde_json::json!({"query": "x", "max_results": 20});
        let limit = parse_max_results(args.as_object().unwrap());
        assert_eq!(limit.value, 20);
        assert!(!limit.was_clamped());
    }

    #[test]
    fn test_search_schema_repo_name_documents_scope() {
        let tool = SearchHybridContextTool::tool();
        let props = tool.input_schema.properties.unwrap();
        let repo_prop = props.get("repo_name").unwrap();
        let desc = repo_prop.get("description").unwrap().as_str().unwrap();

        assert_eq!(
            desc,
            "Optional but HIGHLY RECOMMENDED: repository scope. Accepts a single repository name (`'my-repo'`), a comma-separated list (`'repo-a,repo-b'`), or `'all'` (or `'*'`) to query every indexed repository. If you know the repository you are working on, include it in your FIRST query to avoid mixed results from other indexed projects. Omit to search across all repositories."
        );
    }
}
