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

use crate::{db::graph::QueryExt, mcp_handler::KnotMcpHandler};

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
                "Comprehensive reverse dependency lookup: finds all code that references a specific entity through \
                 any relationship type (calls, inheritance, implementation, type usage). \
                 Use this to: detect dead code, understand impact analysis, refactor safely, discover inheritance chains, \
                 or track type usage. Returns results grouped by relationship type (CALLS, EXTENDS, IMPLEMENTS, REFERENCES). \
                 Works with Java, Kotlin and TypeScript. IMPORTANT: If you know the repository name, ALWAYS include the 'repo_name' parameter in your initial call to avoid mixed results from other indexed projects."
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

    #[test]
    fn test_format_references_result_empty() {
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("MyEntity", &references);
        assert!(formatted.contains("No references found"));
    }

    #[test]
    fn test_format_references_result_with_data() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "method",
                    "file_path": "file1.java",
                    "start_line": 10,
                    "signature": "void caller1()"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("MyEntity", &references);
        assert!(formatted.contains("caller1"));
        assert!(formatted.contains("file1.java:10"));
    }

    #[test]
    fn test_format_references_result_with_multiple_relationship_types() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [{"name": "ChildClass", "kind": "class", "file_path": "file2.java", "start_line": 20}],
            "implements": [{"name": "ImplClass", "kind": "class", "file_path": "file3.java", "start_line": 30}],
            "references": [{"name": "refUser", "kind": "method", "file_path": "file4.java", "start_line": 40}]
        });
        let formatted = format_references_result("MyEntity", &references);
        assert!(formatted.contains("Found 4 reference(s)"));
        assert!(formatted.contains("Calls (function/method invocations)"));
        assert!(formatted.contains("Extends (class inheritance)"));
        assert!(formatted.contains("Implements (interface implementation)"));
        assert!(formatted.contains("References (type annotations/usages)"));
    }

    #[test]
    fn test_format_reference_entry_complete() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "file_path": "src/Handler.java",
            "start_line": 42,
            "signature": "public void myMethod() throws Exception"
        });
        let formatted = format_reference_entry(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("method"));
        assert!(formatted.contains("src/Handler.java:42"));
        assert!(formatted.contains("public void myMethod() throws Exception"));
    }

    #[test]
    fn test_format_reference_entry_without_line_number() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "file_path": "src/Handler.java"
        });
        let formatted = format_reference_entry(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("src/Handler.java"));
        assert!(!formatted.contains(":"));
    }

    #[test]
    fn test_format_reference_entry_without_kind() {
        let entity = json!({
            "name": "UnknownEntity",
            "file_path": "src/Unknown.java",
            "start_line": 50
        });
        let formatted = format_reference_entry(&entity);
        assert!(formatted.contains("UnknownEntity"));
        assert!(formatted.contains("src/Unknown.java:50"));
    }

    #[test]
    fn test_format_references_result_only_extends() {
        let references = json!({
            "calls": [],
            "extends": [
                {"name": "ChildClass1", "kind": "class", "file_path": "file1.java", "start_line": 10},
                {"name": "ChildClass2", "kind": "class", "file_path": "file2.java", "start_line": 20}
            ],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("BaseClass", &references);
        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(formatted.contains("Extends (class inheritance) (2)"));
        assert!(!formatted.contains("Calls (function/method invocations)"));
    }

    #[test]
    fn test_format_references_result_dead_code() {
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("UnusedMethod", &references);
        assert!(formatted.contains("No references found"));
        assert!(formatted.contains("This entity may be unused"));
    }
}
