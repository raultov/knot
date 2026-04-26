//! Explore File Tool
//!
//! Lists all code entities (classes, methods, interfaces, functions)
//! within a specific source file, organized by type.
//!
//! **Key Capabilities:**
//! - **File Anatomy Inspection**: See all classes, interfaces, methods, functions at a glance
//! - **Code Structure Navigation**: Quickly understand the structure without reading line-by-line
//! - **Method/Function Discovery**: Find all callable entities in a file with signatures
//! - **Documentation Preview**: See docstrings and inline comments for each entity
//! - **Multi-language Support**: Works with Java and TypeScript codebases
//! - **Architecture Overview**: Get a bird's-eye view of a module's structure

use rust_mcp_sdk::schema::*;
use serde_json::json;
use std::collections::HashMap;

use crate::mcp_handler::KnotMcpHandler;

pub struct ExploreFileTool;

impl ExploreFileTool {
    pub fn tool() -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "file_path".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Absolute path to the source file to explore",
                "minLength": 1
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
            name: "explore_file".to_string(),
            description: Some(
                "Read-only file anatomy inspection. Use this to list all classes, methods, and properties within a specific source file without reading its entire contents. \
                 Provides a structural bird's-eye view of a file, showing entity signatures and docstrings to quickly grasp a module's layout. \
                 \n\nUsage: Use AFTER identifying an interesting file via 'search_hybrid_context' to understand its available methods, or before modifying a file. Do NOT use this for searching across multiple files. \
                 \n\nBehaviour & Return: Read-only operation. Returns a Markdown-formatted outline of the file's entities, grouped by type (Classes, Methods, Interfaces), including line numbers for direct editor navigation. No side effects. \
                 \n\nParameter guidance: 'file_path' must be a relative or absolute path to a valid source file. Include 'repo_name' if the file path might be ambiguous across multiple indexed repositories. \
                 \n\nSupports Java, Kotlin, and TypeScript codebases."
                    .to_string(),
            ),
            input_schema: ToolInputSchema::new(
                vec!["file_path".to_string()],
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

        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CallToolError::from_message("Missing 'file_path' parameter".to_string())
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
        let (fp, json_result) = cli_tools::run_explore_file(file_path, repo_name, graph_db)
            .await
            .map_err(|e| CallToolError::from_message(format!("Explore file failed: {}", e)))?;

        let formatted = cli_tools::format_file_entities(&fp, &json_result);

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
    fn test_explore_file_tool_schema() {
        let tool = ExploreFileTool::tool();
        assert_eq!(tool.name, "explore_file");
        assert!(tool.description.is_some());

        let schema = tool.input_schema;
        assert!(schema.required.contains(&"file_path".to_string()));

        let props = schema.properties.unwrap();
        assert!(props.contains_key("file_path"));
        assert!(props.contains_key("repo_name"));
    }
}
