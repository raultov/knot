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

pub mod enrich;
pub mod format;

use rust_mcp_sdk::schema::*;
use serde_json::json;
use std::collections::HashMap;

use crate::mcp_handler::KnotMcpHandler;

pub struct SearchHybridContextTool;

impl SearchHybridContextTool {
    pub fn tool() -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "query".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Search query describing what you're looking for (e.g., 'user authentication', 'API error handling')",
                "minLength": 1,
                "maxLength": 500
            }))
            .unwrap(),
        );
        properties.insert(
            "max_results".to_string(),
            serde_json::from_value(json!({
                "type": "integer",
                "description": "Maximum number of results to return (default: 5)",
                "minimum": 1,
                "maximum": 20,
                "default": 5
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
            name: "search_hybrid_context".to_string(),
            description: Some(
                "Hybrid semantic + structural search combining AI understanding with code architecture. \
                 Search by meaning ('user authentication'), class/method names, docstrings, comments, or architectural patterns. \
                 Returns full context including signatures, documentation, inline comments, and dependencies. \
                 Works with Java, Kotlin and TypeScript codebases. IMPORTANT: If you know the repository name, ALWAYS include the 'repo_name' parameter in your initial call to avoid mixed results from other indexed projects."
                    .to_string(),
            ),
            input_schema: ToolInputSchema::new(vec!["query".to_string()], Some(properties), None),
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

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CallToolError::from_message("Missing 'query' parameter".to_string()))?;

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_i64())
            .unwrap_or(5) as usize;

        let repo_name = args.get("repo_name").and_then(|v| v.as_str());

        // Call the shared CLI tool logic
        let formatted = cli_tools::run_search_hybrid_context(
            query,
            max_results,
            repo_name,
            &handler.vector_db,
            &handler.graph_db,
            &handler.embedder,
        )
        .await
        .map_err(|e| CallToolError::from_message(format!("Search failed: {}", e)))?;

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
    }
}
