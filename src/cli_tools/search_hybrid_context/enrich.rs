//! Structural enrichment of search results via Neo4j graph relationships.
//!
//! Annotation-only: relationship data is attached to entities that are
//! already in the result list. Callers and helpers are shown as context of
//! a definition, never as substitute rows displacing it — the ranked order
//! is decided entirely in `rank` before enrichment runs.

use serde_json::json;
use std::sync::Arc;

use crate::db::graph::{DEFAULT_MAX_TARGETS, GraphDb, QueryExt};
use crate::models::RepoScope;

/// Enrich search results with related entities (subclasses, implementers, usages).
///
/// Annotation-only: relationship data is attached to entities that are
/// already in the result list. Callers and helpers are shown as context of a
/// definition, never as substitute rows displacing it.
pub(super) async fn enrich_with_relationships(
    context: &serde_json::Value,
    _entity_names: &[String],
    graph_db: &Arc<GraphDb>,
    repo: &RepoScope,
) -> anyhow::Result<serde_json::Value> {
    let mut enriched = context.clone();
    let repo_names = repo.filter_names();

    if let Some(entities) = enriched.as_array_mut() {
        for entity in entities.iter_mut() {
            if let Some(name) = entity.get("name").and_then(|v| v.as_str())
                && let Ok(references) = graph_db
                    // `kinds=all`: the call-site enrichment must keep seeing
                    // every reference edge regardless of the search's own
                    // kind filter semantics.
                    .find_references(name, &repo_names, DEFAULT_MAX_TARGETS, Some("all"))
                    .await
            {
                enrich_single_entity(entity, &references);
            }
        }
    }

    Ok(enriched)
}

pub(crate) fn extract_subclass_names(extends_arr: &[serde_json::Value]) -> Vec<String> {
    extends_arr
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

pub(crate) fn extract_implementer_names(implements_arr: &[serde_json::Value]) -> Vec<String> {
    implements_arr
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

pub(crate) fn format_reference_samples(references_arr: &[serde_json::Value]) -> Vec<String> {
    references_arr
        .iter()
        .take(3)
        .map(|e| {
            let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let file = e.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{} in {}", name, file)
        })
        .collect()
}

pub(crate) fn enrich_single_entity(entity: &mut serde_json::Value, references: &serde_json::Value) {
    if let Some(extends_arr) = references.get("extends").and_then(|v| v.as_array())
        && !extends_arr.is_empty()
    {
        let subclasses = extract_subclass_names(extends_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("subclasses".to_string(), json!(subclasses));
        }
    }

    if let Some(implements_arr) = references.get("implements").and_then(|v| v.as_array())
        && !implements_arr.is_empty()
    {
        let implementers = extract_implementer_names(implements_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("implementers".to_string(), json!(implementers));
        }
    }

    if let Some(references_arr) = references.get("references").and_then(|v| v.as_array())
        && !references_arr.is_empty()
    {
        let usage_count = references_arr.len();
        let samples = format_reference_samples(references_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("type_usage_count".to_string(), json!(usage_count));
            obj.insert("type_usage_samples".to_string(), json!(samples));
        }
    }

    if let Some(calls_arr) = references.get("calls").and_then(|v| v.as_array())
        && !calls_arr.is_empty()
    {
        let caller_count = calls_arr.len();
        let samples = format_reference_samples(calls_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("caller_count".to_string(), json!(caller_count));
            obj.insert("caller_samples".to_string(), json!(samples));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_subclass_names_empty() {
        let refs = vec![];
        let names = extract_subclass_names(&refs);
        assert_eq!(names.len(), 0);
    }

    #[test]
    fn test_extract_subclass_names_with_data() {
        let refs = vec![
            json!({"name": "ChildClass1", "kind": "class"}),
            json!({"name": "ChildClass2", "kind": "class"}),
        ];
        let names = extract_subclass_names(&refs);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"ChildClass1".to_string()));
        assert!(names.contains(&"ChildClass2".to_string()));
    }

    #[test]
    fn test_extract_implementer_names() {
        let refs = vec![
            json!({"name": "ImplClass1", "kind": "class"}),
            json!({"name": "ImplClass2", "kind": "class"}),
        ];
        let names = extract_implementer_names(&refs);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"ImplClass1".to_string()));
    }

    #[test]
    fn test_format_reference_samples_limits_to_three() {
        let refs = vec![
            json!({"name": "usage1", "file_path": "file1.java"}),
            json!({"name": "usage2", "file_path": "file2.java"}),
            json!({"name": "usage3", "file_path": "file3.java"}),
            json!({"name": "usage4", "file_path": "file4.java"}),
            json!({"name": "usage5", "file_path": "file5.java"}),
        ];
        let samples = format_reference_samples(&refs);
        assert_eq!(samples.len(), 3);
        assert!(samples[0].contains("usage1"));
        assert!(samples[1].contains("usage2"));
        assert!(samples[2].contains("usage3"));
    }

    #[test]
    fn test_format_reference_samples() {
        let refs = vec![json!({"name": "caller1", "file_path": "caller.java"})];
        let samples = format_reference_samples(&refs);
        assert_eq!(samples.len(), 1);
        assert!(samples[0].contains("caller1"));
        assert!(samples[0].contains("caller.java"));
    }

    #[test]
    fn test_enrich_single_entity_with_subclasses() {
        let mut entity = json!({"name": "MyClass", "kind": "class"});
        let references = json!({
            "extends": [
                {"name": "Child1", "kind": "class"},
                {"name": "Child2", "kind": "class"}
            ],
            "implements": [],
            "references": [],
            "calls": []
        });

        enrich_single_entity(&mut entity, &references);

        assert_eq!(
            entity.get("subclasses"),
            Some(&json!(vec!["Child1", "Child2"]))
        );
    }

    #[test]
    fn test_enrich_single_entity_with_all_relationships() {
        let mut entity = json!({"name": "MyInterface"});
        let references = json!({
            "extends": [{"name": "Child1"}],
            "implements": [{"name": "Impl1"}, {"name": "Impl2"}],
            "references": [
                {"name": "ref1", "file_path": "ref1.java"},
                {"name": "ref2", "file_path": "ref2.java"}
            ],
            "calls": [{"name": "caller1", "file_path": "caller.java"}]
        });

        enrich_single_entity(&mut entity, &references);

        assert!(entity.get("subclasses").is_some());
        assert!(entity.get("implementers").is_some());
        assert!(entity.get("type_usage_count").is_some());
        assert!(entity.get("caller_count").is_some());
    }

    #[test]
    fn test_enrich_single_entity_ignores_empty_arrays() {
        let mut entity = json!({"name": "MyClass"});
        let references = json!({
            "extends": [],
            "implements": [],
            "references": [],
            "calls": []
        });

        enrich_single_entity(&mut entity, &references);

        assert!(entity.get("subclasses").is_none());
        assert!(entity.get("implementers").is_none());
        assert!(entity.get("type_usage_count").is_none());
        assert!(entity.get("caller_count").is_none());
    }
}
