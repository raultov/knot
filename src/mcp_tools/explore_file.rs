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
                "Explore the complete anatomy of a source file: all classes, interfaces, methods, and functions \
                 organized by type with signatures and documentation. Use this to understand file structure, \
                 navigate large modules, or get a quick overview before diving into details. Works with Java, Kotlin and TypeScript. \
                 IMPORTANT: If you know the repository name, ALWAYS include the 'repo_name' parameter in your initial call to avoid mixed results from other indexed projects."
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

        // Call the shared CLI tool logic
        let formatted = cli_tools::run_explore_file(file_path, repo_name, &handler.graph_db)
            .await
            .map_err(|e| CallToolError::from_message(format!("Explore file failed: {}", e)))?;

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
