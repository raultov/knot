use serde_json::json;

pub fn format_search_results(context: &serde_json::Value) -> String {
    let mut output = String::from("# Search Results\n\n");

    if let Some(entities) = context.as_array() {
        for entity in entities {
            output.push_str(&format_entity(entity));
        }
    } else if let Some(obj) = context.as_object() {
        output.push_str(&format_entity(&json!(obj)));
    }

    if output.is_empty() || output == "# Search Results\n\n" {
        output.push_str("No results found.");
    }

    output
}

pub(crate) fn format_entity(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        let repo_suffix = entity
            .get("repo_name")
            .and_then(|v| v.as_str())
            .map(|r| format!(" (repo: {r})"))
            .unwrap_or_default();
        if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
            output.push_str(&format!("## `{}` ({}){}\n\n", name, kind, repo_suffix));
        } else {
            output.push_str(&format!("## `{}`{}\n\n", name, repo_suffix));
        }
    }

    if let Some(file_path) = entity.get("file_path").and_then(|v| v.as_str()) {
        output.push_str(&format!("**File:** `{}`\n\n", file_path));
    }

    // Show FQN when it differs from the entity name (e.g., package-qualified Java names)
    if let Some(fqn) = entity.get("fqn").and_then(|v| v.as_str()) {
        let name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if fqn != name && !fqn.is_empty() {
            output.push_str(&format!("**FQN:** `{}`\n\n", fqn));
        }
    }

    if let Some(signature) = entity.get("signature").and_then(|v| v.as_str()) {
        output.push_str(&format!("**Signature:**\n```\n{}\n```\n\n", signature));
    }

    if let Some(docstring) = entity
        .get("docstring")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        output.push_str(&format!("**Documentation:**\n{}\n\n", docstring));
    }

    // Show subclasses
    output.push_str(&format_string_list(
        entity,
        "subclasses",
        "**Subclasses (extends):**",
    ));

    // Show implementers
    output.push_str(&format_string_list(
        entity,
        "implementers",
        "**Implementers:**",
    ));

    // Show type usage summary
    if let Some(count) = entity.get("type_usage_count").and_then(|v| v.as_i64()) {
        output.push_str(&format!(
            "**Type Usage:** Referenced in {count} location(s)\n"
        ));
        append_samples(
            entity,
            "type_usage_samples",
            "Sample usages",
            count,
            &mut output,
        );
        output.push('\n');
    }

    // Show callers summary
    if let Some(count) = entity.get("caller_count").and_then(|v| v.as_i64()) {
        output.push_str(&format!("**Called by:** {count} location(s)\n"));
        append_samples(
            entity,
            "caller_samples",
            "Sample callers",
            count,
            &mut output,
        );
        output.push('\n');
    }

    // Show dependencies
    output.push_str(&format_string_list(entity, "dependencies", "**Calls:**"));

    output
}

/// Formats a non-empty string list as a bulleted section; empty output when
/// the key is absent or empty.
fn format_string_list(entity: &serde_json::Value, key: &str, title: &str) -> String {
    let Some(items) = entity
        .get(key)
        .and_then(|v| v.as_array())
        .filter(|d| !d.is_empty())
    else {
        return String::new();
    };
    let mut section = format!("{title}\n");
    for item in items {
        if let Some(name) = item.as_str() {
            section.push_str(&format!("- `{name}`\n"));
        }
    }
    section.push('\n');
    section
}

/// Appends a sample list block ("Sample usages:" / "Sample callers:") when
/// the key holds a non-empty array.
///
/// `header` carries the block title WITHOUT the trailing colon (added here).
/// `total` is the true reference count the samples were drawn from. When the
/// sample list is shorter than the total, the header is quantified —
/// `Sample callers — showing 2 of 21 (truncated):` — so a sample can never be
/// mistaken for the complete set. When the sample covers everything the
/// header stays unchanged.
fn append_samples(
    entity: &serde_json::Value,
    key: &str,
    header: &str,
    total: i64,
    output: &mut String,
) {
    let Some(samples) = entity
        .get(key)
        .and_then(|v| v.as_array())
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let shown = samples.len() as i64;
    if shown < total {
        output.push_str(&format!(
            "{header} — showing {shown} of {total} (truncated):\n"
        ));
    } else {
        output.push_str(&format!("{header}:\n"));
    }
    for sample in samples {
        if let Some(s) = sample.as_str() {
            output.push_str(&format!("- {s}\n"));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_search_results_empty_array() {
        let results = json!([]);
        let formatted = format_search_results(&results);
        assert!(formatted.contains("No results found"));
    }

    #[test]
    fn test_format_search_results_empty_string() {
        let results = json!("# Search Results\n\n");
        let formatted = format_search_results(&results);
        assert!(formatted.contains("No results found"));
    }

    #[test]
    fn test_format_entity_with_name_and_kind() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("MyClass"));
        assert!(formatted.contains("class"));
    }

    #[test]
    fn test_format_entity_with_file_path() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "file_path": "src/main/MyClass.java"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("src/main/MyClass.java"));
        assert!(formatted.contains("**File:**"));
    }

    #[test]
    fn test_format_entity_with_signature() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "signature": "public void myMethod(String param)"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("public void myMethod(String param)"));
        assert!(formatted.contains("**Signature:**"));
    }

    #[test]
    fn test_format_entity_with_docstring() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "docstring": "This is a test class\nwith multiple lines"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("This is a test class"));
        assert!(formatted.contains("**Documentation:**"));
    }

    #[test]
    fn test_format_entity_ignores_empty_docstring() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "docstring": "   \n  "
        });
        let formatted = format_entity(&entity);
        assert!(!formatted.contains("**Documentation:**"));
    }

    #[test]
    fn test_format_entity_with_subclasses() {
        let entity = json!({
            "name": "BaseClass",
            "kind": "class",
            "subclasses": ["ChildClass1", "ChildClass2"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Subclasses (extends):**"));
        assert!(formatted.contains("ChildClass1"));
        assert!(formatted.contains("ChildClass2"));
    }

    #[test]
    fn test_format_entity_ignores_empty_subclasses() {
        let entity = json!({
            "name": "BaseClass",
            "kind": "class",
            "subclasses": []
        });
        let formatted = format_entity(&entity);
        assert!(!formatted.contains("**Subclasses"));
    }

    #[test]
    fn test_format_entity_with_implementers() {
        let entity = json!({
            "name": "MyInterface",
            "kind": "interface",
            "implementers": ["Impl1", "Impl2", "Impl3"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Implementers:**"));
        assert!(formatted.contains("Impl1"));
        assert!(formatted.contains("Impl2"));
        assert!(formatted.contains("Impl3"));
    }

    #[test]
    fn test_format_entity_with_type_usage() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "type_usage_count": 5,
            "type_usage_samples": ["usage1 in file1.java", "usage2 in file2.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Type Usage:** Referenced in 5 location(s)"));
        assert!(formatted.contains("Sample usages — showing 2 of 5 (truncated):"));
        assert!(formatted.contains("usage1 in file1.java"));
    }

    #[test]
    fn test_format_entity_ignores_empty_usage_samples() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "type_usage_count": 5,
            "type_usage_samples": []
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Type Usage:** Referenced in 5 location(s)"));
        assert!(!formatted.contains("Sample usages:"));
    }

    #[test]
    fn test_format_entity_with_callers() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "caller_count": 3,
            "caller_samples": ["caller1 in file1.java", "caller2 in file2.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Called by:** 3 location(s)"));
        // 2 samples out of 3 callers is a truncated sample — the notice must
        // say so explicitly (v1.10.0).
        assert!(formatted.contains("Sample callers — showing 2 of 3 (truncated):"));
        assert!(formatted.contains("caller1 in file1.java"));
    }

    #[test]
    fn test_format_entity_caller_sample_covering_everything_has_no_notice() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "caller_count": 2,
            "caller_samples": ["caller1 in file1.java", "caller2 in file2.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("Sample callers:"));
        assert!(!formatted.contains("truncated"));
    }

    #[test]
    fn test_format_entity_many_callers_quantifies_sample() {
        // The bug report scenario: a ubiquitous symbol with 21 callers whose
        // 3-string sample read as if it were the complete set.
        let entity = json!({
            "name": "delete",
            "kind": "method",
            "caller_count": 21,
            "caller_samples": [
                "caller1 in a.java", "caller2 in b.java", "caller3 in c.java"
            ]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Called by:** 21 location(s)"));
        assert!(formatted.contains("Sample callers — showing 3 of 21 (truncated):"));
    }

    #[test]
    fn test_format_entity_type_usage_quantifies_sample() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "type_usage_count": 21,
            "type_usage_samples": ["u1 in f1.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("Sample usages — showing 1 of 21 (truncated):"));
    }

    #[test]
    fn test_format_entity_with_dependencies() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "dependencies": ["dep1", "dep2", "dep3"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Calls:**"));
        assert!(formatted.contains("dep1"));
        assert!(formatted.contains("dep2"));
    }

    #[test]
    fn test_format_entity_ignores_empty_dependencies() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "dependencies": []
        });
        let formatted = format_entity(&entity);
        assert!(!formatted.contains("**Calls:**"));
    }

    #[test]
    fn test_format_search_results_multiple_entities() {
        let results = json!([
            {"name": "Class1", "kind": "class", "file_path": "file1.java"},
            {"name": "Class2", "kind": "class", "file_path": "file2.java"}
        ]);
        let formatted = format_search_results(&results);
        assert!(formatted.contains("Class1"));
        assert!(formatted.contains("Class2"));
        assert!(formatted.contains("file1.java"));
        assert!(formatted.contains("file2.java"));
    }

    #[test]
    fn test_format_entity_without_kind() {
        let entity = json!({
            "name": "UnknownEntity"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("UnknownEntity"));
        // Should not have the kind in parentheses
        assert!(!formatted.contains("UnknownEntity ()"));
    }
}
