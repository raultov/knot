//! Core CLI Tools Module
//!
//! Shared logic for CLI and MCP tools. These functions encapsulate
//! the business logic for searching, finding callers, and exploring files.
//! Both the CLI and MCP interfaces use these functions to avoid duplication.

pub mod deps;
pub mod explore_file;
pub mod find_callers;
pub mod formatters;
pub mod repos;
pub mod search_hybrid_context;
pub mod subgraph;

pub use deps::{format_deps_output, run_deps};
pub use explore_file::{format_file_entities, run_explore_file};
pub use find_callers::{format_reference_entry, format_references_result, run_find_callers};
pub use repos::{format_repos_output, run_list_repos};
pub use search_hybrid_context::run_search_hybrid_context;
pub use subgraph::{DEFAULT_MAX_NODES, run_get_subgraph};

// --- JSON helper functions shared across CLI formatters ---

/// Extract the `start_line` field from a JSON entity as a string, defaulting to `"-"`.
pub(crate) fn json_line_number(entity: &serde_json::Value) -> String {
    entity
        .get("start_line")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Resolve the target name for a reference entity.
///
/// Prefers `target_fqn` when available (qualified identifiers disambiguate
/// homonyms like `WidgetA::new` vs `WidgetB::new`), falling back to `target_name`
/// and finally to the provided `fallback`.
pub(crate) fn json_target_name(entity: &serde_json::Value, fallback: &str) -> String {
    entity
        .get("target_fqn")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            entity
                .get("target_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| fallback.to_string())
}

/// Extract the entities array from a JSON result.
///
/// Handles both the wrapped form `{"entities": [...]}` and the plain array form `[...]`.
pub(crate) fn json_entities_array(result: &serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(obj) = result.as_object() {
        obj.get("entities")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    } else if let Some(arr) = result.as_array() {
        arr.clone()
    } else {
        Vec::new()
    }
}

/// Append a signature line to `output` if the entity has a non-empty `signature` field.
pub(crate) fn append_signature_if_present(output: &mut String, entity: &serde_json::Value) {
    if let Some(signature) = entity
        .get("signature")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        output.push_str(&format!("  - Signature: `{}`\n", signature));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- json_line_number ---

    #[test]
    fn test_json_line_number_present() {
        let entity = json!({"start_line": 42});
        assert_eq!(json_line_number(&entity), "42");
    }

    #[test]
    fn test_json_line_number_missing() {
        let entity = json!({"name": "foo"});
        assert_eq!(json_line_number(&entity), "-");
    }

    #[test]
    fn test_json_line_number_zero() {
        let entity = json!({"start_line": 0});
        assert_eq!(json_line_number(&entity), "0");
    }

    // --- json_target_name ---

    #[test]
    fn test_json_target_name_prefers_fqn() {
        let entity = json!({"target_fqn": "crate::mod::Fn", "target_name": "Fn"});
        assert_eq!(json_target_name(&entity, "fallback"), "crate::mod::Fn");
    }

    #[test]
    fn test_json_target_name_falls_back_to_target_name() {
        let entity = json!({"target_name": "MyClass"});
        assert_eq!(json_target_name(&entity, "fallback"), "MyClass");
    }

    #[test]
    fn test_json_target_name_falls_back_to_fallback() {
        let entity = json!({});
        assert_eq!(
            json_target_name(&entity, "default_entity"),
            "default_entity"
        );
    }

    #[test]
    fn test_json_target_name_ignores_empty_fqn() {
        let entity = json!({"target_fqn": "", "target_name": "RealName"});
        assert_eq!(json_target_name(&entity, "fallback"), "RealName");
    }

    // --- json_entities_array ---

    #[test]
    fn test_json_entities_array_from_wrapped_object() {
        let result = json!({"entities": [{"name": "A"}, {"name": "B"}]});
        let arr = json_entities_array(&result);
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "A");
    }

    #[test]
    fn test_json_entities_array_from_plain_array() {
        let result = json!([{"name": "X"}, {"name": "Y"}]);
        let arr = json_entities_array(&result);
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[1]["name"], "Y");
    }

    #[test]
    fn test_json_entities_array_null_returns_empty() {
        let arr = json_entities_array(&serde_json::Value::Null);
        assert!(arr.is_empty());
    }

    #[test]
    fn test_json_entities_array_empty_object_returns_empty() {
        let arr = json_entities_array(&json!({"entities": []}));
        assert!(arr.is_empty());
    }

    // --- append_signature_if_present ---

    #[test]
    fn test_append_signature_with_value() {
        let mut output = String::new();
        let entity = json!({"signature": "pub fn foo()"});
        append_signature_if_present(&mut output, &entity);
        assert!(output.contains("pub fn foo()"));
        assert!(output.contains("Signature"));
    }

    #[test]
    fn test_append_signature_missing_field() {
        let mut output = String::new();
        let entity = json!({"name": "foo"});
        append_signature_if_present(&mut output, &entity);
        assert!(output.is_empty());
    }

    #[test]
    fn test_append_signature_empty_string() {
        let mut output = String::new();
        let entity = json!({"signature": ""});
        append_signature_if_present(&mut output, &entity);
        assert!(output.is_empty());
    }

    #[test]
    fn test_append_signature_whitespace_only() {
        let mut output = String::new();
        let entity = json!({"signature": "   "});
        append_signature_if_present(&mut output, &entity);
        assert!(!output.is_empty()); // "   " is not empty, just whitespace
    }
}
