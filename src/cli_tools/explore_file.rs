//! Core explore_file logic shared between CLI and MCP
//!
//! Lists all code entities (classes, methods, interfaces, functions)
//! within a specific source file, organized by type.

use std::path::Path;
use std::sync::Arc;

use crate::db::graph::{GraphDb, QueryExt};

/// Main explore_file function called by both CLI and MCP
pub async fn run_explore_file(
    file_path: &str,
    repo_name: Option<&str>,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<String> {
    // Normalize file path: attempt to canonicalize, fall back to original if file doesn't exist
    let normalized_path = if Path::new(file_path).exists() {
        std::fs::canonicalize(file_path)?
            .to_string_lossy()
            .to_string()
    } else {
        // If file doesn't exist in current filesystem, use path as-is
        // (it may exist in a different repo context or be an absolute path)
        file_path.to_string()
    };

    // Query Neo4j for all entities in the file
    let entities = graph_db
        .get_file_entities(&normalized_path, repo_name)
        .await?;

    let formatted = format_file_entities(file_path, &entities);

    Ok(formatted)
}

/// Format file entities as Markdown
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
        let mut rust_structs = Vec::new();
        let mut rust_enums = Vec::new();
        let mut rust_unions = Vec::new();
        let mut rust_traits = Vec::new();
        let mut rust_impls = Vec::new();
        let mut rust_functions = Vec::new();
        let mut rust_methods = Vec::new();
        let mut rust_macros = Vec::new();
        let mut rust_type_aliases = Vec::new();
        let mut rust_constants = Vec::new();
        let mut rust_statics = Vec::new();
        let mut rust_modules = Vec::new();

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
                    "rust_struct" => rust_structs.push(entity),
                    "rust_enum" => rust_enums.push(entity),
                    "rust_union" => rust_unions.push(entity),
                    "rust_trait" => rust_traits.push(entity),
                    "rust_impl" => rust_impls.push(entity),
                    "rust_function" => rust_functions.push(entity),
                    "rust_method" => rust_methods.push(entity),
                    "rust_macro_def" | "rust_macro_invoke" => rust_macros.push(entity),
                    "rust_type_alias" => rust_type_aliases.push(entity),
                    "rust_constant" => rust_constants.push(entity),
                    "rust_static" => rust_statics.push(entity),
                    "rust_module" => rust_modules.push(entity),
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

        // Rust entities
        if !rust_structs.is_empty() {
            output.push_str("## Structs (Rust)\n\n");
            for entity in rust_structs {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_enums.is_empty() {
            output.push_str("## Enums (Rust)\n\n");
            for entity in rust_enums {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_unions.is_empty() {
            output.push_str("## Unions (Rust)\n\n");
            for entity in rust_unions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_traits.is_empty() {
            output.push_str("## Traits (Rust)\n\n");
            for entity in rust_traits {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_impls.is_empty() {
            output.push_str("## Impl Blocks (Rust)\n\n");
            for entity in rust_impls {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_functions.is_empty() {
            output.push_str("## Functions (Rust)\n\n");
            for entity in rust_functions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_methods.is_empty() {
            output.push_str("## Methods (Rust)\n\n");
            for entity in rust_methods {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_macros.is_empty() {
            output.push_str("## Macros (Rust)\n\n");
            for entity in rust_macros {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_type_aliases.is_empty() {
            output.push_str("## Type Aliases (Rust)\n\n");
            for entity in rust_type_aliases {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_constants.is_empty() {
            output.push_str("## Constants (Rust)\n\n");
            for entity in rust_constants {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_statics.is_empty() {
            output.push_str("## Statics (Rust)\n\n");
            for entity in rust_statics {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_modules.is_empty() {
            output.push_str("## Modules (Rust)\n\n");
            for entity in rust_modules {
                output.push_str(&format_entity_summary(entity));
            }
        }
    } else {
        output.push_str("No entities found.\n");
    }

    output
}

/// Format entity summary as Markdown
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
    use serde_json::json;

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
