//! Table output formatters for CLI.
//!
//! Formats search results, callers, and file entities as ASCII tables
//! using the comfy-table crate for human-readable console output.

use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};
use serde_json::Value;

pub fn format_search_table(results: &Value) -> String {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::DynamicFullWidth);
    table.set_header(vec![
        Cell::new("Kind")
            .set_alignment(CellAlignment::Center)
            .fg(Color::Cyan),
        Cell::new("Name").fg(Color::Green),
        Cell::new("File").fg(Color::White),
        Cell::new("Line")
            .set_alignment(CellAlignment::Right)
            .fg(Color::Yellow),
    ]);

    if let Some(arr) = results.as_array() {
        for entity in arr {
            let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("-");
            let name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("-");
            let file = entity
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let line = entity
                .get("start_line")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());

            let kind_color = match kind {
                "class" | "python_class" => Color::Yellow,
                "interface" => Color::Cyan,
                "method" | "function" | "python_method" | "python_function" => Color::Blue,
                "variable" | "field" => Color::Magenta,
                _ => Color::White,
            };

            table.add_row(vec![
                Cell::new(kind).fg(kind_color),
                Cell::new(name),
                Cell::new(file),
                Cell::new(line).set_alignment(CellAlignment::Right),
            ]);
        }
    }

    if table.row_iter().count() == 0 {
        return "No matching code found for your query.".to_string();
    }

    table.to_string()
}

pub fn format_callers_table(entity_name: &str, references: &Value) -> String {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::DynamicFullWidth);
    table.set_header(vec![
        Cell::new("Relationship")
            .set_alignment(CellAlignment::Center)
            .fg(Color::Cyan),
        Cell::new("Target").fg(Color::Magenta),
        Cell::new("Caller").fg(Color::Green),
        Cell::new("File").fg(Color::White),
        Cell::new("Line")
            .set_alignment(CellAlignment::Right)
            .fg(Color::Yellow),
    ]);

    let mut total_refs = 0;

    let rel_types = vec![
        ("calls", "Calls"),
        ("extends", "Extends"),
        ("implements", "Implements"),
        ("references", "References"),
    ];

    for (key, label) in rel_types {
        if let Some(arr) = references.get(key).and_then(|v| v.as_array()) {
            let label_color = match key {
                "calls" => Color::Blue,
                "extends" => Color::Yellow,
                "implements" => Color::Cyan,
                "references" => Color::Magenta,
                _ => Color::White,
            };
            for entity in arr {
                total_refs += 1;
                let target = entity
                    .get("target_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(entity_name);
                let caller = entity.get("name").and_then(|v| v.as_str()).unwrap_or("-");
                let file = entity
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let line = entity
                    .get("start_line")
                    .and_then(|v| v.as_i64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());

                table.add_row(vec![
                    Cell::new(label).fg(label_color),
                    Cell::new(target).fg(Color::Magenta),
                    Cell::new(caller),
                    Cell::new(file),
                    Cell::new(line).set_alignment(CellAlignment::Right),
                ]);
            }
        }
    }

    if total_refs == 0 {
        return format!(
            "No references found for `{}`. This entity may be unused.\n",
            entity_name
        );
    }

    let header = format!("References to `{}` ({} total)\n", entity_name, total_refs);
    header + &table.to_string()
}

pub fn format_explore_table(file_path: &str, entities: &Value) -> String {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::DynamicFullWidth);
    table.set_header(vec![
        Cell::new("Type")
            .set_alignment(CellAlignment::Center)
            .fg(Color::Cyan),
        Cell::new("Name").fg(Color::Green),
        Cell::new("Line")
            .set_alignment(CellAlignment::Right)
            .fg(Color::Yellow),
        Cell::new("Signature / Doc").fg(Color::White),
    ]);

    if let Some(arr) = entities.as_array() {
        for entity in arr {
            let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("-");
            let name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("-");
            let line = entity
                .get("start_line")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());

            let sig_or_doc = entity
                .get("signature")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| {
                    entity
                        .get("docstring")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.lines().next().unwrap_or("").to_string())
                })
                .unwrap_or_else(|| "-".to_string());

            let kind_color = match kind {
                "class" | "python_class" => Color::Yellow,
                "interface" => Color::Cyan,
                "method" | "function" | "python_method" | "python_function" => Color::Blue,
                "variable" | "field" => Color::Magenta,
                _ => Color::White,
            };

            table.add_row(vec![
                Cell::new(kind).fg(kind_color),
                Cell::new(name),
                Cell::new(line).set_alignment(CellAlignment::Right),
                Cell::new(sig_or_doc),
            ]);
        }
    }

    if table.row_iter().count() == 0 {
        return format!("No entities found in `{}`.\n", file_path);
    }

    let count = entities.as_array().map(|a| a.len()).unwrap_or(0);
    let header = format!("Entities in `{}` ({} found)\n", file_path, count);
    header + &table.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_search_table_empty() {
        let results = json!([]);
        let output = format_search_table(&results);
        assert!(output.contains("No matching code found"));
    }

    #[test]
    fn test_format_search_table_with_results() {
        let results = json!([
            {
                "name": "MyClass",
                "kind": "class",
                "file_path": "src/main.java",
                "start_line": 10
            }
        ]);
        let output = format_search_table(&results);
        assert!(output.contains("MyClass"));
        assert!(output.contains("class"));
        assert!(output.contains("src/main.java"));
        assert!(output.contains("10"));
    }

    #[test]
    fn test_format_search_table_multiple_results() {
        let results = json!([
            {"name": "Class1", "kind": "class", "file_path": "file1.java", "start_line": 1},
            {"name": "Class2", "kind": "interface", "file_path": "file2.java", "start_line": 20}
        ]);
        let output = format_search_table(&results);
        assert!(output.contains("Class1"));
        assert!(output.contains("Class2"));
    }

    #[test]
    fn test_format_callers_table_empty() {
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": []
        });
        let output = format_callers_table("MyEntity", &references);
        assert!(output.contains("No references found"));
        assert!(output.contains("MyEntity"));
    }

    #[test]
    fn test_format_callers_table_with_references() {
        let references = json!({
            "calls": [
                {"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}
            ],
            "extends": [],
            "implements": [],
            "references": []
        });
        let output = format_callers_table("MyEntity", &references);
        assert!(output.contains("References to `MyEntity`"));
        assert!(output.contains("caller1"));
        assert!(output.contains("Calls"));
    }

    #[test]
    fn test_format_explore_table_empty() {
        let entities = json!([]);
        let output = format_explore_table("test.java", &entities);
        assert!(output.contains("No entities found"));
    }

    #[test]
    fn test_format_explore_table_with_entities() {
        let entities = json!([
            {
                "name": "MyClass",
                "kind": "class",
                "start_line": 10,
                "signature": "public class MyClass"
            }
        ]);
        let output = format_explore_table("test.java", &entities);
        assert!(output.contains("Entities in `test.java`"));
        assert!(output.contains("MyClass"));
        assert!(output.contains("class"));
    }

    #[test]
    fn test_format_explore_table_with_docstring() {
        let entities = json!([
            {
                "name": "myMethod",
                "kind": "method",
                "start_line": 20,
                "docstring": "This is a method"
            }
        ]);
        let output = format_explore_table("test.java", &entities);
        assert!(output.contains("myMethod"));
        assert!(output.contains("This is a method"));
    }
}
