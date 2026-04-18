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

use crate::{db::graph::QueryExt, mcp_handler::KnotMcpHandler};

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

        // Query Neo4j for all entities in the file
        let entities = handler
            .graph_db
            .get_file_entities(file_path, repo_name)
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
        let mut objects = Vec::new();
        let mut companions = Vec::new();
        let mut methods = Vec::new();
        let mut functions = Vec::new();
        let mut properties = Vec::new();

        for entity in entities_array {
            if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
                match kind {
                    "class" | "kotlin_class" => classes.push(entity),
                    "interface" | "kotlin_interface" => interfaces.push(entity),
                    "kotlin_object" => objects.push(entity),
                    "kotlin_companion" => companions.push(entity),
                    "method" | "kotlin_method" => methods.push(entity),
                    "function" | "kotlin_function" => functions.push(entity),
                    "kotlin_property" => properties.push(entity),
                    _ => {}
                }
            }
        }

        // Format in order: Classes, Interfaces, Objects, Companions, Methods, Functions, Properties
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

        if !objects.is_empty() {
            output.push_str("## Objects (Singletons)\n\n");
            for entity in objects {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !companions.is_empty() {
            output.push_str("## Companion Objects\n\n");
            for entity in companions {
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

        if !properties.is_empty() {
            output.push_str("## Properties\n\n");
            for entity in properties {
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

        if let Some(decorators_array) = entity.get("decorators").and_then(|v| v.as_array())
            && !decorators_array.is_empty()
        {
            let decorator_strs: Vec<String> = decorators_array
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
            if !decorator_strs.is_empty() {
                output.push_str(&format!("  - Decorators: {}\n", decorator_strs.join(", ")));
            }
        }

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

    #[test]
    fn test_format_file_entities_empty() {
        let entities = json!([]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("No entities found in this file"));
    }

    #[test]
    fn test_format_file_entities_single_class() {
        let entities = json!([
            {
                "name": "MyClass",
                "kind": "class",
                "start_line": 10,
                "signature": "public class MyClass"
            }
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("## Classes"));
        assert!(formatted.contains("MyClass"));
        assert!(formatted.contains("(line 10)"));
        assert!(formatted.contains("public class MyClass"));
    }

    #[test]
    fn test_format_file_entities_multiple_classes() {
        let entities = json!([
            {
                "name": "Class1",
                "kind": "class",
                "start_line": 10
            },
            {
                "name": "Class2",
                "kind": "class",
                "start_line": 50
            }
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("Found 2 entity/entities"));
        assert!(formatted.contains("Class1"));
        assert!(formatted.contains("Class2"));
    }

    #[test]
    fn test_format_file_entities_groups_by_kind() {
        let entities = json!([
            {"name": "MyClass", "kind": "class"},
            {"name": "MyInterface", "kind": "interface"},
            {"name": "myMethod", "kind": "method"},
            {"name": "myFunction", "kind": "function"}
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("## Classes"));
        assert!(formatted.contains("## Interfaces"));
        assert!(formatted.contains("## Methods"));
        assert!(formatted.contains("## Functions"));
    }

    #[test]
    fn test_format_entity_summary_with_signature() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "start_line": 20,
            "signature": "public void myMethod(String param)"
        });
        let formatted = format_entity_summary(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("(line 20)"));
        assert!(formatted.contains("public void myMethod(String param)"));
    }

    #[test]
    fn test_format_entity_summary_with_docstring() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "docstring": "First line of doc\nSecond line of doc"
        });
        let formatted = format_entity_summary(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("First line of doc"));
        assert!(!formatted.contains("Second line of doc"));
    }

    #[test]
    fn test_format_entity_summary_ignores_whitespace_docstring() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "docstring": "   \n  \t"
        });
        let formatted = format_entity_summary(&entity);
        assert!(!formatted.contains("- Doc:"));
    }

    #[test]
    fn test_format_entity_summary_without_optional_fields() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class"
        });
        let formatted = format_entity_summary(&entity);
        assert!(formatted.contains("MyClass"));
        assert!(!formatted.contains("(line"));
        assert!(!formatted.contains("Signature:"));
    }

    #[test]
    fn test_format_file_entities_unknown_kind() {
        let entities = json!([
            {
                "name": "UnknownEntity",
                "kind": "unknown_kind"
            }
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        // Unknown kinds should be silently ignored
        assert!(!formatted.contains("UnknownEntity"));
        assert!(formatted.contains("Found 1 entity/entities"));
    }

    #[test]
    fn test_format_file_entities_displays_file_path() {
        let entities = json!([
            {"name": "MyClass", "kind": "class"}
        ]);
        let formatted = format_file_entities("src/main/java/MyClass.java", &entities);
        assert!(formatted.contains("src/main/java/MyClass.java"));
    }
}
