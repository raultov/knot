//! Table output formatters for CLI.
//!
//! Formats search results, callers, and file entities as ASCII tables
//! using the comfy-table crate for human-readable console output.

use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};
use serde_json::Value;

use crate::cli_tools::resolution::ResolutionView;
use crate::cli_tools::{json_entities_array, json_line_number, json_target_name};

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
            let line = json_line_number(entity);
            let kind_color = match kind {
                "class" | "python_class" => Color::Yellow,
                "interface" => Color::Cyan,
                "method" | "function" | "python_method" | "python_function" => Color::Blue,
                "variable" | "field" => Color::Magenta,
                _ => Color::White,
            };

            let name_cell = match entity.get("repo_name").and_then(|v| v.as_str()) {
                Some(repo) => Cell::new(format!("{name} (repo: {repo})")),
                None => Cell::new(name),
            };

            table.add_row(vec![
                Cell::new(kind).fg(kind_color),
                name_cell,
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

    let rel_types = [
        ("calls", "Calls", Color::Blue),
        ("extends", "Extends", Color::Yellow),
        ("implements", "Implements", Color::Cyan),
        ("references", "References", Color::Magenta),
    ];

    for (key, label, label_color) in rel_types {
        let Some(arr) = references.get(key).and_then(|v| v.as_array()) else {
            continue;
        };
        for entity in arr {
            total_refs += 1;
            // Prefer target_fqn when available (qualified identifiers
            // disambiguate homonyms like `WidgetA::new` vs `WidgetB::new`).
            let target = json_target_name(entity, entity_name);
            let caller_name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("-");
            let caller_repo = entity
                .get("repo_name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let caller = match caller_repo {
                Some(repo) => format!("{caller_name} (repo: {repo})"),
                None => caller_name.to_string(),
            };
            // Reference repo attribution (v1.8.1), rule R3: the
            // Target column repeats once per row, so it is labeled only for
            // genuine cross-repo references to avoid doubling the noise.
            let target_repo = entity
                .get("target_repo_name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let target_cell = match target_repo {
                Some(target_repo) if Some(target_repo) != caller_repo => {
                    format!("{target} (repo: {target_repo})")
                }
                _ => target,
            };
            let file = entity
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let line = json_line_number(entity);

            table.add_row(vec![
                Cell::new(label).fg(label_color),
                Cell::new(target_cell).fg(Color::Magenta),
                Cell::new(caller),
                Cell::new(file),
                Cell::new(line).set_alignment(CellAlignment::Right),
            ]);
        }
    }

    if total_refs == 0 {
        let mut no_ref_msg = callers_resolution_detail(references);
        no_ref_msg.push_str(&format!(
            "No references found for `{}`. This entity may be unused.\n",
            entity_name
        ));
        return no_ref_msg;
    }

    let header = callers_resolution_header(entity_name, references, total_refs);
    header + &table.to_string()
}

/// Verbose resolution block shown when no references were found: the target
/// list is the only useful signal left, so it is spelled out in full.
fn callers_resolution_detail(references: &Value) -> String {
    let Some(view) = ResolutionView::from_references(references) else {
        return String::new();
    };

    let mut out = format!("{}:\n", view.summary());
    out.push_str(&view.target_bullets());
    out.push('\n');

    if view.is_fuzzy() {
        out.push_str(&format!(
            "WARNING: Fuzzy match — no entity matched `{}` exactly.\n\n",
            view.query()
        ));
    }

    out
}

/// One-line resolution header shown above the caller table.
fn callers_resolution_header(entity_name: &str, references: &Value, total_refs: usize) -> String {
    let Some(view) = ResolutionView::from_references(references) else {
        return format!("References to `{}` ({} total)\n", entity_name, total_refs);
    };

    let mut out = format!("{} ({} references total)\n", view.summary(), total_refs);

    if view.is_fuzzy() {
        out.push_str(&format!(
            "WARNING: Fuzzy match — no entity matched `{}` exactly.\n",
            view.query()
        ));
    }

    if view.is_truncated() {
        out.push_str(&format!(
            "Truncated — {} targets matched; showing first {} by FQN.\n",
            view.total_targets(),
            view.count()
        ));
    }

    out
}

pub fn format_explore_table(file_path: &str, result: &Value) -> String {
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

    let arr = json_entities_array(result);

    for entity in &arr {
        let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("-");
        let name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("-");
        let line = json_line_number(entity);

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

    if table.row_iter().count() == 0 {
        return format!("No entities found in `{}`.\n", file_path);
    }

    let count = arr.len();
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
    fn callers_table_labels_caller_repo() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1", "kind": "method",
                    "file_path": "file1.java", "start_line": 10,
                    "repo_name": "alpha", "target_repo_name": "alpha"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });
        let output = format_callers_table("MyEntity", &references);
        assert!(output.contains("caller1 (repo: alpha)"), "got {output}");
    }

    #[test]
    fn callers_table_labels_target_repo_when_different() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1", "kind": "method",
                    "file_path": "file1.java", "start_line": 10,
                    "target_fqn": "beta::SharedUtil::work",
                    "repo_name": "alpha", "target_repo_name": "beta"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });
        let output = format_callers_table("MyEntity", &references);
        assert!(
            output.contains("beta::SharedUtil::work (repo: beta)"),
            "got {output}"
        );
    }

    #[test]
    fn callers_table_omits_target_label_when_same_repo() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1", "kind": "method",
                    "file_path": "file1.java", "start_line": 10,
                    "repo_name": "alpha", "target_repo_name": "alpha"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });
        let output = format_callers_table("MyEntity", &references);
        // Rule R3: intra-repo reference — the Target cell stays bare.
        assert!(!output.contains("MyEntity (repo:"), "got {output}");
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
