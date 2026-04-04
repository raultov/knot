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
                "description": "Optional repository name to filter results to a specific codebase (e.g., 'shelob-java'). Omit to search across all repositories.",
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

        // Extract UUIDs from search results
        let uuids: Vec<String> = search_results
            .iter()
            .filter_map(|result| {
                result
                    .get("uuid")
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

        // Step 4: Format results as Markdown
        let formatted = format_search_results(&context);

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

fn format_search_results(context: &serde_json::Value) -> String {
    let mut output = String::from("# Search Results\n\n");

    if let Some(entities) = context.as_array() {
        for entity in entities {
            output.push_str(&format_entity(entity));
        }
    } else if let Some(obj) = context.as_object() {
        output.push_str(&format_entity(&json!(obj)));
    }

    if output.is_empty() || output == "# Search Results\n\n" {
        output.push_str("No results found.");
    }

    output
}

fn format_entity(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
            output.push_str(&format!("## `{}` ({}) \n\n", name, kind));
        } else {
            output.push_str(&format!("## `{}`\n\n", name));
        }
    }

    if let Some(file_path) = entity.get("file_path").and_then(|v| v.as_str()) {
        output.push_str(&format!("**File:** `{}`\n\n", file_path));
    }

    if let Some(signature) = entity.get("signature").and_then(|v| v.as_str()) {
        output.push_str(&format!("**Signature:**\n```\n{}\n```\n\n", signature));
    }

    if let Some(docstring) = entity
        .get("docstring")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        output.push_str(&format!("**Documentation:**\n{}\n\n", docstring));
    }

    if let Some(deps) = entity
        .get("dependencies")
        .and_then(|v| v.as_array())
        .filter(|d| !d.is_empty())
    {
        output.push_str("**Calls:**\n");
        for dep in deps {
            if let Some(dep_name) = dep.as_str() {
                output.push_str(&format!("- `{}`\n", dep_name));
            }
        }
        output.push('\n');
    }

    output
}
