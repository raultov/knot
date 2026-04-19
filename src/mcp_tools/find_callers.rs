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

use rust_mcp_sdk::schema::*;
use serde_json::json;
use std::collections::HashMap;

use crate::mcp_handler::KnotMcpHandler;

pub struct FindCallersTool;

impl FindCallersTool {
    pub fn tool() -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "entity_name".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "The name of the function, method, or class to find callers for",
                "minLength": 1,
                "maxLength": 255
            }))
            .unwrap(),
        );
        properties.insert(
            "repo_name".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Optional but HIGHLY RECOMMENDED: repository name to filter results to a specific codebase (e.g., 'my-java-repo'). If you know the repository you are working on, include this in your FIRST query to avoid mixed results from other indexed projects. Omit only to search across all repositories.",
                "minLength": 1,
                "maxLength": 255
            }))
            .unwrap(),
        );

        Tool {
            name: "find_callers".to_string(),
            description: Some(
                "Read-only reverse dependency lookup. Use this to find all code that references, calls, extends, or implements a specific entity. \
                 Answers 'who uses this code?' by querying the graph database. Differs from search tools by providing exact dependency tracking. \
                 \n\nUsage: Use for impact analysis before refactoring or to detect dead code. Do NOT use this for semantic feature discovery—use 'search_hybrid_context' instead. \
                 CRITICAL: For common method names (e.g., 'accept', 'process'), you MUST include a signature fragment (e.g., 'accept(List') to prevent thousands of irrelevant results. \
                 \n\nBehaviour & Return: Read-only graph traversal with no side effects. Returns Markdown grouped by relationship type (Calls, Extends, Implements, References) with exact file paths and line numbers. \
                 \n\nParameter guidance: 'entity_name' supports exact names or signature fragments (e.g., 'handleRequest' or 'handle(Request'). Include 'repo_name' to filter results to the specific codebase being analyzed. \
                 \n\nSupports Java, Kotlin, and TypeScript codebases."
                    .to_string(),
            ),
            input_schema: ToolInputSchema::new(
                vec!["entity_name".to_string()],
                Some(properties),
                None,
            ),
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

        let args = params
            .arguments
            .ok_or_else(|| CallToolError::from_message("Missing arguments".to_string()))?;

        let entity_name = args
            .get("entity_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CallToolError::from_message("Missing 'entity_name' parameter".to_string())
            })?;

        let repo_name = args.get("repo_name").and_then(|v| v.as_str());

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
        let formatted = cli_tools::run_find_callers(entity_name, repo_name, graph_db)
            .await
            .map_err(|e| CallToolError::from_message(format!("Find callers failed: {}", e)))?;

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
    }
}
