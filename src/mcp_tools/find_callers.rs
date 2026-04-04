//! Find Callers Tool
//!
//! Performs a reverse dependency lookup: given an entity name,
//! finds all other entities that call it.
//!
//! **Key Capabilities:**
//! - **Dead Code Detection**: Identify truly unused methods/functions (zero incoming calls)
//! - **Impact Analysis**: Understand "If I modify this function, what else breaks?"
//! - **Refactoring Safety**: Find all references before renaming or removing code
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

        Tool {
            name: "find_callers".to_string(),
            description: Some(
                "Reverse dependency lookup: finds all code that calls a specific function, method, or class. \
                 Use this to: detect dead code (zero callers), understand impact analysis ('What breaks if I modify this?'), \
                 refactor safely by finding all references, or traverse the call graph. Works with Java and TypeScript."
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
        let args = params
            .arguments
            .ok_or_else(|| CallToolError::from_message("Missing arguments".to_string()))?;

        let entity_name = args
            .get("entity_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CallToolError::from_message("Missing 'entity_name' parameter".to_string())
            })?;

        // Query Neo4j for callers
        let callers = handler
            .graph_db
            .find_callers(entity_name)
            .await
            .map_err(|e| CallToolError::from_message(format!("Graph query failed: {}", e)))?;

        let formatted = format_callers_result(entity_name, &callers);

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

fn format_callers_result(entity_name: &str, callers: &serde_json::Value) -> String {
    let mut output = format!("# Callers of `{}`\n\n", entity_name);

    if let Some(callers_array) = callers.as_array() {
        if callers_array.is_empty() {
            output.push_str(&format!(
                "No callers found for `{}`. This entity may be unused.\n",
                entity_name
            ));
            return output;
        }

        output.push_str(&format!("Found {} caller(s):\n\n", callers_array.len()));

        for caller in callers_array {
            output.push_str(&format_caller_entry(caller));
        }
    } else {
        output.push_str("No callers found.\n");
    }

    output
}

fn format_caller_entry(caller: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = caller.get("name").and_then(|v| v.as_str()) {
        if let Some(kind) = caller.get("kind").and_then(|v| v.as_str()) {
            output.push_str(&format!("### `{}` ({})\n\n", name, kind));
        } else {
            output.push_str(&format!("### `{}`\n\n", name));
        }
    }

    if let Some(file_path) = caller.get("file_path").and_then(|v| v.as_str()) {
        if let Some(start_line) = caller.get("start_line").and_then(|v| v.as_i64()) {
            output.push_str(&format!("**Location:** `{}:{}`\n\n", file_path, start_line));
        } else {
            output.push_str(&format!("**Location:** `{}`\n\n", file_path));
        }
    }

    if let Some(signature) = caller
        .get("signature")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        output.push_str(&format!("**Signature:**\n```\n{}\n```\n\n", signature));
    }

    output
}
