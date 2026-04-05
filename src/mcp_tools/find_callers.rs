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
                "description": "Optional repository name to filter results to a specific codebase (e.g., 'my-java-repo'). Omit to search across all repositories.",
                "minLength": 1,
                "maxLength": 255
            }))
            .unwrap(),
        );

        Tool {
            name: "find_callers".to_string(),
            description: Some(
                "Comprehensive reverse dependency lookup: finds all code that references a specific entity through \
                 any relationship type (calls, inheritance, implementation, type usage). \
                 Use this to: detect dead code, understand impact analysis, refactor safely, discover inheritance chains, \
                 or track type usage. Returns results grouped by relationship type (CALLS, EXTENDS, IMPLEMENTS, REFERENCES). \
                 Works with Java and TypeScript. Supports optional repository filtering."
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

        let repo_name = args.get("repo_name").and_then(|v| v.as_str());

        // Query Neo4j for all reference types
        let references = handler
            .graph_db
            .find_references(entity_name, repo_name)
            .await
            .map_err(|e| CallToolError::from_message(format!("Graph query failed: {}", e)))?;

        let formatted = format_references_result(entity_name, &references);

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

fn format_references_result(entity_name: &str, references: &serde_json::Value) -> String {
    let mut output = format!("# References to `{}`\n\n", entity_name);

    // Count total references across all types
    let mut total_refs = 0;
    let rel_types = vec![
        ("calls", "Calls (function/method invocations)"),
        ("extends", "Extends (class inheritance)"),
        ("implements", "Implements (interface implementation)"),
        ("references", "References (type annotations/usages)"),
    ];

    for (key, _) in &rel_types {
        if let Some(arr) = references.get(key).and_then(|v| v.as_array()) {
            total_refs += arr.len();
        }
    }

    if total_refs == 0 {
        output.push_str(&format!(
            "No references found for `{}`. This entity may be unused.\n",
            entity_name
        ));
        return output;
    }

    output.push_str(&format!(
        "Found {} reference(s) across all relationship types:\n\n",
        total_refs
    ));

    // Format each relationship type
    for (key, label) in rel_types {
        if let Some(arr) = references.get(key).and_then(|v| v.as_array())
            && !arr.is_empty()
        {
            output.push_str(&format!("## {} ({})\n\n", label, arr.len()));
            for entity in arr {
                output.push_str(&format_reference_entry(entity));
            }
        }
    }

    output
}

fn format_reference_entry(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
            output.push_str(&format!("- **`{}`** ({})", name, kind));
        } else {
            output.push_str(&format!("- **`{}`**", name));
        }
    }

    if let Some(file_path) = entity.get("file_path").and_then(|v| v.as_str()) {
        if let Some(start_line) = entity.get("start_line").and_then(|v| v.as_i64()) {
            output.push_str(&format!(" at `{}:{}`", file_path, start_line));
        } else {
            output.push_str(&format!(" at `{}`", file_path));
        }
    }

    output.push('\n');

    if let Some(signature) = entity
        .get("signature")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        output.push_str(&format!("  - Signature: `{}`\n", signature));
    }

    output.push('\n');
    output
}
