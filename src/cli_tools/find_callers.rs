//! Core find_callers logic shared between CLI and MCP
//!
//! Performs comprehensive reverse dependency lookup: given an entity name,
//! finds all other entities that reference it through any relationship type
//! (CALLS, EXTENDS, IMPLEMENTS, REFERENCES).

use std::sync::Arc;

use crate::db::graph::{GraphDb, QueryExt};

use crate::cli_tools::json_target_name;

use crate::cli_tools::append_signature_if_present;
use crate::cli_tools::resolution::ResolutionView;

/// Main find_callers function called by both CLI and MCP
pub async fn run_find_callers(
    entity_name: &str,
    repo_name: Option<&str>,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<serde_json::Value> {
    let references = graph_db.find_references(entity_name, repo_name).await?;
    Ok(references)
}

pub fn format_references_result(entity_name: &str, references: &serde_json::Value) -> String {
    let mut output = format!("# References to `{}`\n\n", entity_name);
    output.push_str(&format_resolution_markdown(references));

    let rel_types = [
        ("calls", "Calls (function/method invocations)"),
        ("extends", "Extends (class inheritance)"),
        ("implements", "Implements (interface implementation)"),
        ("references", "References (type annotations/usages)"),
        ("overridden_by", "Overridden by (method implementations)"),
        ("overrides", "Overrides (declared supertype methods)"),
    ];

    let total_refs: usize = rel_types
        .iter()
        .filter_map(|(key, _)| references.get(key).and_then(|v| v.as_array()))
        .map(|arr| arr.len())
        .sum();

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

    for (key, label) in rel_types {
        if let Some(arr) = references.get(key).and_then(|v| v.as_array())
            && !arr.is_empty()
        {
            output.push_str(&format!("## {} ({})\n\n", label, arr.len()));
            output.push_str(&format_relationship_bucket(entity_name, arr));
        }
    }

    output
}

/// Render the `resolution` block (which targets the query resolved to, plus
/// fuzzy/truncation caveats) as Markdown. Empty when no block is present.
fn format_resolution_markdown(references: &serde_json::Value) -> String {
    let Some(view) = ResolutionView::from_references(references) else {
        return String::new();
    };

    let mut output = format!("{}:\n", view.summary());
    output.push_str(&view.target_bullets());
    output.push('\n');

    if view.is_fuzzy() {
        output.push_str(&format!(
            "> **Fuzzy match** — no entity matched `{}` exactly. The {} target(s) below were\n\
             > found by substring match and may be unrelated. Re-run with an exact name or a\n\
             > fully qualified name (e.g. `Namespace.Type.Member`) for precise results.\n\n",
            view.query(),
            view.count()
        ));
    }

    if view.is_truncated() {
        output.push_str(&format!(
            "> **Truncated** — {} targets matched; showing the first {} by FQN.\n\n",
            view.total_targets(),
            view.count()
        ));
    }

    output
}

/// Render one relationship bucket, grouping callers by resolved target when
/// the bucket spans more than a single target (homonym disambiguation).
fn format_relationship_bucket(entity_name: &str, arr: &[serde_json::Value]) -> String {
    use std::collections::HashMap;

    let mut grouped: HashMap<String, Vec<&serde_json::Value>> = HashMap::new();
    for entity in arr {
        grouped
            .entry(target_group_key(entity))
            .or_default()
            .push(entity);
    }

    let mut output = String::new();

    if grouped.len() == 1 {
        for entity in arr {
            output.push_str(&format_reference_entry(entity));
        }
        return output;
    }

    for (target_key, entities) in grouped {
        let first_entity = entities[0];
        // Prefer target_fqn when available — qualified identifiers
        // disambiguate homonyms (e.g., `WidgetA::new` vs `WidgetB::new`).
        let target_name = json_target_name(first_entity, entity_name);
        output.push_str(&format!(
            "### Target: `{}` at `{}`\n\n",
            target_name, target_key
        ));

        if let Some(target_sig) = first_entity
            .get("target_signature")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            output.push_str(&format!("Signature: `{}`\n\n", target_sig));
        }

        for entity in entities {
            output.push_str(&format_reference_entry(entity));
        }
    }

    output
}

/// Grouping key identifying the target a reference points at.
fn target_group_key(entity: &serde_json::Value) -> String {
    let Some(target_file) = entity.get("target_file_path").and_then(|v| v.as_str()) else {
        return "unknown".to_string();
    };

    match entity.get("target_start_line").and_then(|v| v.as_i64()) {
        Some(target_line) => format!("{}:{}", target_file, target_line),
        None => target_file.to_string(),
    }
}

pub fn format_reference_entry(entity: &serde_json::Value) -> String {
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

    append_signature_if_present(&mut output, entity);

    output.push('\n');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn test_format_references_result_multiple_targets_same_name() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "function",
                    "file_path": "src/parser/orphans.rs",
                    "start_line": 8,
                    "target_name": "find_nearest_entity_by_line",
                    "target_file_path": "src/parser/orphans.rs",
                    "target_start_line": 92,
                    "target_signature": "pub(crate) fn find_nearest_entity_by_line(entities: &[ParsedEntity], target_line: usize) -> usize"
                },
                {
                    "name": "caller2",
                    "kind": "function",
                    "file_path": "src/parser/languages/rust.rs",
                    "start_line": 258,
                    "target_name": "find_nearest_entity_by_line",
                    "target_file_path": "src/parser/languages/rust.rs",
                    "target_start_line": 445,
                    "target_signature": "fn find_nearest_entity_by_line(entities: &[ParsedEntity], line: usize) -> usize"
                },
                {
                    "name": "caller3",
                    "kind": "function",
                    "file_path": "src/parser/orphans.rs",
                    "start_line": 175,
                    "target_name": "find_nearest_entity_by_line",
                    "target_file_path": "src/parser/orphans.rs",
                    "target_start_line": 92,
                    "target_signature": "pub(crate) fn find_nearest_entity_by_line(entities: &[ParsedEntity], target_line: usize) -> usize"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });

        let formatted = format_references_result("find_nearest_entity_by_line", &references);

        assert!(formatted.contains("Found 3 reference(s)"));
        assert!(formatted.contains("### Target:"));
        assert!(formatted.contains("src/parser/orphans.rs:92"));
        assert!(formatted.contains("src/parser/languages/rust.rs:445"));
        assert!(formatted.contains("pub(crate) fn find_nearest_entity_by_line"));
        assert!(formatted.contains("fn find_nearest_entity_by_line"));
        assert!(formatted.contains("caller1"));
        assert!(formatted.contains("caller2"));
        assert!(formatted.contains("caller3"));
    }

    #[test]
    fn test_format_references_result_renders_override_buckets() {
        // Scenario Q3 — formatter renders the new OVERRIDES buckets.
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "overridden_by": [
                {"name": "Session.getUniqueId", "kind": "groovy_method",
                 "file_path": "Session.groovy", "start_line": 26}
            ],
            "overrides": [
                {"name": "ISession.getUniqueId", "kind": "groovy_method",
                 "file_path": "ISession.groovy", "start_line": 3}
            ]
        });
        let formatted = format_references_result("getUniqueId", &references);
        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(formatted.contains("Overridden by (method implementations)"));
        assert!(formatted.contains("Overrides (declared supertype methods)"));
        assert!(formatted.contains("Session.getUniqueId"));
        assert!(formatted.contains("ISession.getUniqueId"));
    }

    #[test]
    fn test_format_references_result_single_target_no_grouping() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "method",
                    "file_path": "file1.java",
                    "start_line": 10,
                    "target_name": "myMethod",
                    "target_file_path": "src/Handler.java",
                    "target_start_line": 42,
                    "target_signature": "public void myMethod()"
                },
                {
                    "name": "caller2",
                    "kind": "method",
                    "file_path": "file2.java",
                    "start_line": 20,
                    "target_name": "myMethod",
                    "target_file_path": "src/Handler.java",
                    "target_start_line": 42,
                    "target_signature": "public void myMethod()"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });

        let formatted = format_references_result("myMethod", &references);

        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(!formatted.contains("### Target:"));
        assert!(formatted.contains("caller1"));
        assert!(formatted.contains("caller2"));
    }

    #[test]
    fn test_format_renders_resolution_header() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "Off",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "targets": [
                    {
                        "uuid": "uuid-123",
                        "name": "Off",
                        "fqn": "OpenLogi.Core.Gestures.GestureOwner.Off",
                        "kind": "csharp_record",
                        "file_path": "src/OpenLogi.Core/Gestures/GestureOwner.cs",
                        "start_line": 15
                    }
                ]
            }
        });
        let formatted = format_references_result("Off", &references);
        assert!(formatted.contains("Resolved to 1 target by exact name match:"));
        assert!(formatted.contains("OpenLogi.Core.Gestures.GestureOwner.Off"));
        assert!(formatted.contains("csharp_record"));
        assert!(formatted.contains("src/OpenLogi.Core/Gestures/GestureOwner.cs:15"));
    }

    #[test]
    fn test_format_renders_fuzzy_warning() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "Offlin",
                "tier": "fuzzy",
                "fuzzy": true,
                "truncated": false,
                "targets": [
                    {
                        "uuid": "uuid-123",
                        "name": "OfflineSlot",
                        "fqn": "OpenLogi.Tests.Hid.InventoryDedupeTests.OfflineSlot",
                        "kind": "csharp_method",
                        "file_path": "src/OpenLogi.Tests/Hid/InventoryDedupeTests.cs",
                        "start_line": 20
                    }
                ]
            }
        });
        let formatted = format_references_result("Offlin", &references);
        assert!(formatted.contains("**Fuzzy match**"));
        assert!(formatted.contains("no entity matched `Offlin` exactly."));
    }

    #[test]
    fn test_format_renders_truncation_notice() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "DuplicateName",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": true,
                "total_targets": 112,
                "targets": [
                    {
                        "uuid": "uuid-123",
                        "name": "DuplicateName",
                        "fqn": "Some.Namespace.DuplicateName",
                        "kind": "class",
                        "file_path": "src/Duplicate.java",
                        "start_line": 10
                    }
                ]
            }
        });
        let formatted = format_references_result("DuplicateName", &references);
        assert!(
            formatted.contains("**Truncated** — 112 targets matched; showing the first 1 by FQN.")
        );
    }

    #[test]
    fn test_format_without_resolution_key_is_unchanged() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("myMethod", &references);
        let expected = "# References to `myMethod`\n\n\
                        Found 1 reference(s) across all relationship types:\n\n\
                        ## Calls (function/method invocations) (1)\n\n\
                        - **`caller1`** (method) at `file1.java:10`\n\n";
        assert_eq!(formatted, expected);
    }
}
