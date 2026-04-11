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

mod enrich;
mod format;

use rust_mcp_sdk::schema::*;
use serde_json::json;
use std::collections::HashMap;

use crate::{
    db::{graph::QueryExt, vector::VectorSearchExt},
    mcp_handler::KnotMcpHandler,
};

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
                "description": "Optional repository name to filter results to a specific codebase (e.g., 'my-java-repo'). Omit to search across all repositories.",
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
                 Works with Java and TypeScript codebases. Supports optional repository filtering."
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

        // Step 1: Embed the query using fastembed
        let vector = handler
            .embedder
            .lock()
            .unwrap()
            .embed_query(query)
            .map_err(|e| CallToolError::from_message(format!("Embedding failed: {}", e)))?;

        // Step 2: Search Qdrant for similar vectors
        let search_results = handler
            .vector_db
            .search(&vector, max_results, repo_name)
            .await
            .map_err(|e| CallToolError::from_message(format!("Vector search failed: {}", e)))?;

        if search_results.is_empty() {
            return Ok(CallToolResult {
                content: vec![ContentBlock::TextContent(TextContent::new(
                    "No matching code found for your query.".to_string(),
                    None,
                    None,
                ))],
                is_error: None,
                meta: None,
                structured_content: None,
            });
        }

        // Extract UUIDs and names from search results
        let uuids: Vec<String> = search_results
            .iter()
            .filter_map(|result| {
                result
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        let entity_names: Vec<String> = search_results
            .iter()
            .filter_map(|result| {
                result
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        // Step 3: Query Neo4j for detailed context and dependencies
        let context = handler
            .graph_db
            .get_entities_with_dependencies(&uuids, repo_name)
            .await
            .map_err(|e| CallToolError::from_message(format!("Graph query failed: {}", e)))?;

        // Step 4: Enrich context with related entities (subclasses, implementers, references)
        let enriched_context =
            enrich::enrich_with_relationships(&context, &entity_names, handler, repo_name)
                .await
                .unwrap_or(context);

        // Step 5: Format results as Markdown
        let formatted = format::format_search_results(&enriched_context);

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
