//! List Files Tool
//!
//! Read-only enumeration of the files an indexed repository carries with
//! their entity counts, optionally narrowed by a directory prefix or glob.
//!
//! **Key Capabilities:**
//! - **File Discovery**: answer 'which files does this repo have under
//!   src/hooks?' without prior path knowledge.
//! - **Directory/glob narrowing**: `src/api`, `src/**/*_test.rs`, …
//! - **Entity counts**: each file shows how many indexed entities it
//!   carries, so an agent can pick the dense ones first.
//!
//! **Usage Guidelines:**
//! - Use this BEFORE `search_hybrid_context` when you do not know the
//!   codebase layout, and pass the same prefix as the search's `path`
//!   parameter to scope searches to those files.
//! - Use `explore_file` afterwards for the anatomy of one relevant file.

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::*;

use crate::mcp_handler::KnotMcpHandler;
use crate::mcp_tools::repo_scope_from_args;

/// Input contract for `list_files`.
///
/// The `#[mcp_tool]` macro derives `ListFilesTool::tool()` from this
/// declaration, so the JSON Schema advertised over MCP stays in lockstep
/// with the fields documented here.
#[mcp_tool(
    name = "list_files",
    title = "List indexed repository files",
    description = "Read-only listing of the files an indexed repository carries, with entity counts and deterministic order. \
                   Answers 'list every file under src/hooks' or 'which files live in src/api/**' without prior path knowledge. \
                   \n\nUsage: Use this BEFORE 'search_hybrid_context' when you do not know the codebase layout; then pass the same prefix to the search's optional 'path' parameter to scope results to those files. \
                   Do NOT use this to enumerate entities — use 'explore_file' on a file from this listing for its anatomy. \
                   \n\nBehaviour & Return: Read-only query with no side effects. \
                   Returns a Markdown table with columns: REPOSITORY, FILE, ENTITIES, ordered by (repository, path). \
                   When nothing matches the prefix, returns 'No indexed files matched the given path'. \
                   \n\nParameter guidance: 'path' is optional. PREFERRED: a repo-relative directory prefix (e.g. 'src/api') matched on a path boundary, or a glob ('src/**/*_test.rs'). Absolute paths under the local checkout are accepted and normalized like 'explore_file'. Omit to list every indexed file (capped; the reply notes truncation). \
                   \n\nParameter guidance: 'repo_name' scopes the listing. Accepts a single repository name or a comma-separated list; include it when several indexed repositories may share path shapes. \
                   \n\nSupports all languages indexed by knot.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(JsonSchema)]
pub struct ListFilesTool {
    #[json_schema(
        description = "Optional repo-relative directory prefix (e.g. 'src/api', matched on a path boundary) or glob pattern (segments: `*` segment wildcard, `**` any depth, `?` one char). Absolute paths under the local checkout are accepted and normalized like 'explore_file'. Omit to list every indexed file.",
        min_length = 1,
        max_length = 500
    )]
    pub path: Option<String>,
    #[json_schema(
        description = "Optional but HIGHLY RECOMMENDED: repository scope. Accepts a single repository name (`'my-repo'`), a comma-separated list (`'repo-a,repo-b'`), or `'all'` (or `'*'`) to query every indexed repository. Include it when several indexed repositories may share path shapes.",
        min_length = 1,
        max_length = 255
    )]
    pub repo_name: Option<String>,
}

impl ListFilesTool {
    pub async fn handle(
        params: CallToolRequestParams,
        handler: &KnotMcpHandler,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        use crate::cli_tools;

        let args = params.arguments.unwrap_or_default();

        let path = args.get("path").and_then(|v| v.as_str());
        let repo = repo_scope_from_args(&args);

        if handler.graph_db.is_none() {
            return Err(CallToolError::from_message(
                "Server running in offline mode - graph database not available".to_string(),
            ));
        }

        let graph_db = handler.graph_db.as_ref().unwrap();

        let json_result = cli_tools::run_list_files(path, &repo, graph_db)
            .await
            .map_err(|e| CallToolError::from_message(format!("Query error: {e}")))?;

        let formatted = cli_tools::format_list_files_markdown(&json_result);

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

    #[test]
    fn test_tool_schema_has_optional_path_and_required_free() {
        let tool = ListFilesTool::tool();
        let props = tool.input_schema.properties.unwrap();

        assert!(props.contains_key("path"));
        assert!(props.contains_key("repo_name"));
        assert!(!tool.input_schema.required.contains(&"path".to_string()));
    }

    #[test]
    fn test_tool_has_valid_name_and_description() {
        let tool = ListFilesTool::tool();
        assert_eq!(tool.name, "list_files");
        assert!(tool.description.is_some());
        assert!(!tool.description.unwrap().is_empty());
    }
}
