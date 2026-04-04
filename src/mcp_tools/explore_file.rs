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

        Tool {
            name: "explore_file".to_string(),
            description: Some(
                "Explore the complete anatomy of a source file: all classes, interfaces, methods, and functions \
                 organized by type with signatures and documentation. Use this to understand file structure, \
                 navigate large modules, or get a quick overview before diving into details. Works with Java and TypeScript."
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
        let args = params
            .arguments
            .ok_or_else(|| CallToolError::from_message("Missing arguments".to_string()))?;

        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CallToolError::from_message("Missing 'file_path' parameter".to_string())
            })?;

        // Query Neo4j for all entities in the file
        let entities = handler
            .graph_db
            .get_file_entities(file_path)
            .await
            .map_err(|e| CallToolError::from_message(format!("Graph query failed: {}", e)))?;

        let formatted = format_file_entities(file_path, &entities);

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

fn format_file_entities(file_path: &str, entities: &serde_json::Value) -> String {
    let mut output = format!("# Entities in `{}`\n\n", file_path);

    if let Some(entities_array) = entities.as_array() {
        if entities_array.is_empty() {
            output.push_str("No entities found in this file.\n");
            return output;
        }

        output.push_str(&format!(
            "Found {} entity/entities:\n\n",
            entities_array.len()
        ));

        // Group entities by kind for better organization
        let mut classes = Vec::new();
        let mut interfaces = Vec::new();
        let mut methods = Vec::new();
        let mut functions = Vec::new();

        for entity in entities_array {
            if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
                match kind {
                    "class" => classes.push(entity),
                    "interface" => interfaces.push(entity),
                    "method" => methods.push(entity),
                    "function" => functions.push(entity),
                    _ => {}
                }
            }
        }

        // Format in order: Classes, Interfaces, Methods, Functions
        if !classes.is_empty() {
            output.push_str("## Classes\n\n");
            for entity in classes {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !interfaces.is_empty() {
            output.push_str("## Interfaces\n\n");
            for entity in interfaces {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !methods.is_empty() {
            output.push_str("## Methods\n\n");
            for entity in methods {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !functions.is_empty() {
            output.push_str("## Functions\n\n");
            for entity in functions {
                output.push_str(&format_entity_summary(entity));
            }
        }
    } else {
        output.push_str("No entities found.\n");
    }

    output
}

fn format_entity_summary(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        output.push_str(&format!("- **`{}`**", name));

        if let Some(start_line) = entity.get("start_line").and_then(|v| v.as_i64()) {
            output.push_str(&format!(" (line {})", start_line));
        }

        output.push('\n');

        if let Some(signature) = entity
            .get("signature")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            output.push_str(&format!("  - Signature: `{}`\n", signature));
        }

        if let Some(docstring) = entity
            .get("docstring")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            let doc_preview = docstring.lines().next().unwrap_or("");
            output.push_str(&format!("  - Doc: {}\n", doc_preview));
        }

        output.push('\n');
    }

    output
}
