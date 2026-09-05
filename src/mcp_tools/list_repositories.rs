//! List Repositories Tool
//!
//! Lists all indexed repositories with optional case-insensitive name filtering.
//! Enables AI agents to discover which codebases are available for search and exploration.
//!
//! **Key Capabilities:**
//! - **List All Repositories**: Retrieve all indexed repositories with metadata
//!   (entity count, file count, build system, primary language).
//! - **Filter by Name**: Optionally filter repositories by a case-insensitive
//!   substring match on the repository name.
//!
//! **Usage Guidelines:**
//! - Use this tool as a starting point to understand what codebases are indexed.
//! - Use the `filter` parameter to quickly locate a specific repository when
//!   working with multiple indexed codebases.
//!
//! **Behavior & Return:**
//! - Read-only query with no side effects.
//! - Returns a Markdown table (for MCP) or JSON array (for CLI) of repositories.
//! - Each entry includes: name, entity_count, file_count, build_system, primary_language.
//!
//! **Parameter Guidance:**
//! - `filter` is optional. When provided, only repositories whose name contains
//!   the filter string (case-insensitive) are returned.

use rust_mcp_sdk::schema::*;
use serde_json::json;
use std::collections::HashMap;

use crate::mcp_handler::KnotMcpHandler;

pub struct ListRepositoriesTool;

impl ListRepositoriesTool {
    pub fn tool() -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "filter".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Optional filter to narrow down repositories by name (case-insensitive substring match). When provided, only repositories whose name contains this string are returned. Examples: 'auth' matches 'auth-service' and 'Auth-Lib', 'api' matches 'my-api'. Omit to list all indexed repositories.",
                "minLength": 1,
                "maxLength": 255
            }))
            .unwrap(),
        );

        Tool {
            name: "list_repositories".to_string(),
            description: Some(
                "Read-only listing of all indexed repositories with optional name filtering. \
                 Shows repository metadata including entity count, file count, build system, and primary language. \
                 Answers 'what codebases have I indexed?' and 'which repositories match this name?'. \
                 \n\nUsage: Use this tool FIRST to discover available codebases before searching or exploring. \
                 Once you know the repository name, switch to 'search_hybrid_context' for semantic search, \
                 'find_callers' for reverse dependency lookup, 'explore_file' for file anatomy, \
                 or 'list_repo_dependencies' for cross-repo dependency graphs. \
                 Do NOT use this tool to search for code entities — use 'search_hybrid_context' instead. \
                 \n\nBehaviour & Return: Read-only query with no side effects. \
                 Returns a Markdown table with columns: REPO, BUILD SYSTEM, LANGUAGE, FILES, ENTITIES. \
                 When no repositories match the filter, returns 'No repositories found.' \
                 \n\nParameter guidance: 'filter' is optional. When provided, only repositories whose name \
                 contains the filter string are returned (case-insensitive substring match). \
                 Omit to list all indexed repositories. \
                 \n\nSupports all languages and build systems indexed by knot."
                    .to_string(),
            ),
            input_schema: ToolInputSchema::new(vec![], Some(properties), None),
            annotations: None,
            execution: None,
            icons: vec![],
            meta: None,
            output_schema: None,
            title: None,
        }
    }

    pub async fn handle(
        params: CallToolRequestParams,
        handler: &KnotMcpHandler,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        use crate::cli_tools;
        use crate::config::OutputFormat;

        let args = params.arguments.unwrap_or_default();

        let filter = args.get("filter").and_then(|v| v.as_str());

        if handler.graph_db.is_none() {
            return Err(CallToolError::from_message(
                "Server running in offline mode - graph database not available".to_string(),
            ));
        }

        let graph_db = handler.graph_db.as_ref().unwrap();

        let json_result = cli_tools::run_list_repos(filter, graph_db)
            .await
            .map_err(|e| CallToolError::from_message(format!("Query error: {e}")))?;

        let formatted = cli_tools::format_repos_output(&json_result, OutputFormat::Markdown);

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
    fn test_tool_schema_has_optional_filter() {
        let tool = ListRepositoriesTool::tool();
        let props = tool.input_schema.properties.unwrap();

        assert!(props.contains_key("filter"));
        assert!(!tool.input_schema.required.contains(&"filter".to_string()));
    }

    #[test]
    fn test_tool_has_valid_name() {
        let tool = ListRepositoriesTool::tool();
        assert_eq!(tool.name, "list_repositories");
    }

    #[test]
    fn test_tool_has_description() {
        let tool = ListRepositoriesTool::tool();
        assert!(tool.description.is_some());
        assert!(!tool.description.unwrap().is_empty());
    }

    #[test]
    fn test_tool_has_input_schema() {
        let tool = ListRepositoriesTool::tool();
        assert!(tool.input_schema.properties.is_some());
    }
}
