//! Find References Tool
//!
//! Performs comprehensive reverse dependency lookup: given an entity name,
//! finds all other entities that reference it through any relationship type
//! (CALLS, EXTENDS, IMPLEMENTS, REFERENCES).
//!
//! **Key Capabilities:**
//! - **Dead Code Detection**: Identify truly unused methods/functions (zero incoming references)
//! - **Impact Analysis**: Understand "If I modify this class/interface, what breaks?"
//! - **Refactoring Safety**: Find all references before renaming or removing code
//! - **Inheritance Chain**: Discover all subclasses (EXTENDS) and implementers (IMPLEMENTS)
//! - **Type Usage**: Track all type annotations and usages (REFERENCES)
//! - **Call Graph Traversal**: Explore the full dependency chain of a method
//! - **Multi-language Support**: Works with Java and TypeScript codebases

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::*;

use crate::mcp_handler::KnotMcpHandler;
use crate::mcp_tools::repo_scope_from_args;

/// Input contract for `find_callers`.
///
/// The `#[mcp_tool]` macro derives `FindCallersTool::tool()` from this
/// declaration, so the JSON Schema advertised over MCP stays in lockstep with
/// the fields documented here.
#[mcp_tool(
    name = "find_callers",
    title = "Find callers (reverse dependencies)",
    description = "Read-only reverse dependency lookup. Use this to find all code that references, calls, extends, or implements a specific entity. \
                   Answers 'who uses this code?' by querying the graph database. Differs from search tools by providing exact dependency tracking. \
                   \n\nUsage: Use for impact analysis before refactoring or to detect dead code. Do NOT use this for semantic feature discovery—use 'search_hybrid_context' instead. \
                   \n\nMatching is precedence-based: exact FQN (containing '.' or '::') → FQN suffix (`Type.member`) → exact name → signature prefix (`accept(List`) → fuzzy substring. \
                   The first tier that matches wins, so an exact name never returns fuzzy noise. Pass a qualified name (`Namespace.Type.Member`) to disambiguate homonyms. \
                   Responses state which tier matched and flag fuzzy results explicitly. \
                   \n\nBehaviour & Return: Read-only graph traversal with no side effects. Returns Markdown grouped by relationship type (Calls, Extends, Implements, References, Overridden by, Overrides) with exact file paths and line numbers. \
                   Each caller entry and each resolved target states its repository as `(repo: name)`, so rows are attributable when multiple repositories are in scope. \
                   For JVM code (Java/Kotlin/Groovy) and C#, 'Overridden by' lists method implementations/overrides in subtypes and 'Overrides' lists the supertype methods a method implements/overrides. \
                   When the query resolves to more than one entity with that name (homonyms, e.g., 'find_nearest_entity_by_line' in orphans.rs vs rust.rs), results are grouped by target entity showing which specific target each caller references — even when only one of the homonyms has callers. \
                    Each caller entry includes: name, kind, file_path:line_number, and signature. When multiple targets exist, each group shows the target's location and signature. \
                    \n\nTruncation & completeness: the queried name is first resolved to concrete targets (capped at 25 by default). \
                    When more targets match than fit the cap, the response states 'Truncated — N targets matched; showing the first M by FQN' and \
                    'Counts below are partial — they cover only the M of N targets shown', so bucket counts are never mistaken for the complete impact set. \
                    Raise 'max_targets' (up to 500) to retrieve more targets when the notice reports truncation. \
                    \n\nParameter guidance: 'entity_name' supports exact names or signature fragments (e.g., 'handleRequest' or 'handle(Request'). Include 'repo_name' to filter results to the specific codebase being analyzed. \
                   \n\nSupports Java, Kotlin, C#, Rust, and TypeScript codebases.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(JsonSchema)]
pub struct FindCallersTool {
    #[json_schema(
        description = "The name of the function, method, or class to find callers for",
        min_length = 1,
        max_length = 255
    )]
    pub entity_name: String,
    #[json_schema(
        description = "Optional but HIGHLY RECOMMENDED: repository scope. Accepts a single repository name (`'my-repo'`), a comma-separated list (`'repo-a,repo-b'`), or `'all'` (or `'*'`) to query every indexed repository. If you know the repository you are working on, include it in your FIRST query to avoid mixed results from other indexed projects. Omit to search across all repositories.",
        min_length = 1,
        max_length = 255
    )]
    pub repo_name: Option<String>,
    #[json_schema(
        description = "Maximum number of resolved targets to include (default: 25, max: 500). Raise this when the response reports a truncated target list and you need the complete impact set.",
        minimum = 1,
        maximum = 500,
        default = 25
    )]
    pub max_targets: Option<i64>,
}

impl FindCallersTool {
    pub async fn handle(
        params: CallToolRequestParams,
        handler: &KnotMcpHandler,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        use crate::cli_tools;

        let args = params
            .arguments
            .ok_or_else(|| CallToolError::from_message("Missing arguments".to_string()))?;

        let entity_name = args
            .get("entity_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CallToolError::from_message("Missing 'entity_name' parameter".to_string())
            })?;

        let repo = repo_scope_from_args(&args);

        let max_targets = args
            .get("max_targets")
            .and_then(|v| v.as_i64())
            .map(|v| v.max(1) as usize);

        // Check if in offline mode
        if handler.graph_db.is_none() {
            return Err(CallToolError::from_message(
                "Server running in offline mode - graph database not available".to_string(),
            ));
        }

        // Extract reference (must be done before await to avoid Send issues)
        let graph_db = handler
            .graph_db
            .as_ref()
            .ok_or_else(|| CallToolError::from_message("Graph DB not available".to_string()))?;

        // Call the shared CLI tool logic
        let json_result = cli_tools::run_find_callers(entity_name, &repo, graph_db, max_targets)
            .await
            .map_err(|e| CallToolError::from_message(format!("Find callers failed: {}", e)))?;

        let formatted = cli_tools::format_references_result(entity_name, &json_result);

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
    fn test_find_callers_tool_schema() {
        let tool = FindCallersTool::tool();
        assert_eq!(tool.name, "find_callers");

        let schema = tool.input_schema;
        assert!(schema.required.contains(&"entity_name".to_string()));

        let props = schema.properties.unwrap();
        assert!(props.contains_key("entity_name"));
        assert!(props.contains_key("repo_name"));
        // v1.10.0: opt-in full impact set.
        assert!(props.contains_key("max_targets"));
    }

    #[test]
    fn test_find_callers_max_targets_is_optional_with_defaults() {
        let tool = FindCallersTool::tool();
        let schema = tool.input_schema;
        // Optional parameter — omitting it keeps the 25-target default.
        assert!(!schema.required.contains(&"max_targets".to_string()));
        let props = schema.properties.unwrap();
        let max_prop = props.get("max_targets").unwrap();
        let desc = max_prop.get("description").unwrap().as_str().unwrap();
        assert_eq!(
            desc,
            "Maximum number of resolved targets to include (default: 25, max: 500). Raise this when the response reports a truncated target list and you need the complete impact set."
        );
        let maximum = max_prop.get("maximum").unwrap().as_i64().unwrap();
        // Drift guard: the advertised ceiling must equal the bound the DB
        // layer actually clamps against.
        assert_eq!(maximum, crate::db::graph::MAX_TARGETS_CEILING as i64);
    }

    #[test]
    fn test_find_callers_description_documents_truncation_contract() {
        let tool = FindCallersTool::tool();
        let desc = tool.description.unwrap();
        assert!(desc.contains("Truncation & completeness"));
        assert!(desc.contains("Counts below are partial"));
        assert!(desc.contains("Raise 'max_targets' (up to 500)"));
    }

    #[test]
    fn test_max_targets_defaults_to_none_when_absent() {
        // Mirrors the parse in `handle`: absent parameter → None → the shared
        // core applies DEFAULT_MAX_TARGETS (25).
        let args = serde_json::json!({"entity_name": "delete"});
        let max_targets = args
            .get("max_targets")
            .and_then(|v| v.as_i64())
            .map(|v| v.max(1) as usize);
        assert_eq!(max_targets, None);
    }

    #[test]
    fn test_max_targets_zero_is_bounded_from_below() {
        // A bogus `max_targets: 0` must not disable the cap (0 would mean
        // "no targets at all"); clamped to >= 1 here and to the ceiling in
        // the db layer.
        let args = serde_json::json!({"entity_name": "delete", "max_targets": 0});
        let max_targets = args
            .get("max_targets")
            .and_then(|v| v.as_i64())
            .map(|v| v.max(1) as usize);
        assert_eq!(max_targets, Some(1));
    }

    #[test]
    fn test_find_callers_schema_repo_name_documents_scope() {
        let tool = FindCallersTool::tool();
        let props = tool.input_schema.properties.unwrap();
        let repo_prop = props.get("repo_name").unwrap();
        let desc = repo_prop.get("description").unwrap().as_str().unwrap();

        assert_eq!(
            desc,
            "Optional but HIGHLY RECOMMENDED: repository scope. Accepts a single repository name (`'my-repo'`), a comma-separated list (`'repo-a,repo-b'`), or `'all'` (or `'*'`) to query every indexed repository. If you know the repository you are working on, include it in your FIRST query to avoid mixed results from other indexed projects. Omit to search across all repositories."
        );
    }

    #[test]
    fn test_find_callers_description_documents_tier_ladder() {
        let tool = FindCallersTool::tool();
        let desc = tool.description.unwrap();
        assert!(desc.contains("Matching is precedence-based: exact FQN"));
        assert!(!desc.contains("CRITICAL: For common method names"));
    }
}
