//! Structural enrichment of search results via Neo4j graph relationships.
//!
//! Annotation-only: relationship data is attached to entities that are
//! already in the result list. Callers and helpers are shown as context of
//! a definition, never as substitute rows displacing it — the ranked order
//! is decided entirely in `rank` before enrichment runs.

use serde_json::json;
use std::sync::Arc;

use crate::db::graph::{GraphDb, QueryExt};
use crate::models::RepoScope;

/// Enrich search results with related entities (subclasses, implementers, usages).
///
/// Annotation-only: relationship data is attached to entities that are
/// already in the result list. Callers and helpers are shown as context of a
/// definition, never as substitute rows displacing it.
///
/// Each row is enriched from the edges of its own UUID — the same edges
/// `find_callers` reports for that exact entity — so same-named entities
/// elsewhere never contribute callers. One lookup per row: a batched lookup
/// would let an interface and its implementation, both in the result list,
/// hide each other's dispatched calls.
pub(super) async fn enrich_with_relationships(
    context: &serde_json::Value,
    graph_db: &Arc<GraphDb>,
    repo: &RepoScope,
) -> anyhow::Result<serde_json::Value> {
    let mut enriched = context.clone();
    let repo_names = repo.filter_names();

    if let Some(entities) = enriched.as_array_mut() {
        for entity in entities.iter_mut() {
            if let Some(uuid) = entity.get("uuid").and_then(|v| v.as_str())
                && let Ok(references) = graph_db
                    .find_references_for_targets(&[uuid.to_string()], &repo_names)
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

pub(crate) fn format_reference_samples<'a>(
    references: impl IntoIterator<Item = &'a serde_json::Value>,
) -> Vec<String> {
    references
        .into_iter()
        .take(3)
        .map(|e| {
            let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let file = e.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{} in {}", name, file)
        })
        .collect()
}

/// Whether a reference row lives in test code — the ranker's
/// [`super::rank::is_test_path`] rule plus the Rust `#[cfg(test)]` flag.
fn is_test_reference(row: &serde_json::Value) -> bool {
    let file = row.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
    super::rank::is_test_path(file)
        || row
            .get("is_test_context")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

/// Attach one relationship bucket to `entity` as `<prefix>_count` (every
/// row, so it always equals the `find_callers` count) and `<prefix>_samples`
/// (production rows only). Test rows are counted apart in
/// `<prefix>_test_count` and sampled in `<prefix>_test_samples`, so the
/// formatter can collapse them into `N in tests` (issue #50).
fn insert_reference_bucket(
    entity: &mut serde_json::Value,
    prefix: &str,
    rows: Option<&Vec<serde_json::Value>>,
) {
    let Some(rows) = rows.filter(|rows| !rows.is_empty()) else {
        return;
    };
    let (tests, production): (Vec<&serde_json::Value>, Vec<&serde_json::Value>) =
        rows.iter().partition(|row| is_test_reference(row));
    let Some(obj) = entity.as_object_mut() else {
        return;
    };
    obj.insert(format!("{prefix}_count"), json!(rows.len()));
    obj.insert(
        format!("{prefix}_samples"),
        json!(format_reference_samples(production)),
    );
    if !tests.is_empty() {
        obj.insert(format!("{prefix}_test_count"), json!(tests.len()));
        obj.insert(
            format!("{prefix}_test_samples"),
            json!(format_reference_samples(tests)),
        );
    }
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

    let bucket = |key: &str| references.get(key).and_then(|v| v.as_array());
    insert_reference_bucket(entity, "type_usage", bucket("references"));
    insert_reference_bucket(entity, "caller", bucket("calls"));
    // Calls bound to an override or to a supertype method reach this entity
    // only through dynamic dispatch: reported apart from direct callers,
    // exactly like the "Dispatched calls" group of `find_callers`.
    insert_reference_bucket(entity, "dispatched_caller", bucket("dispatched_calls"));
    insert_reference_bucket(entity, "http_caller", bucket("http_calls"));
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
    fn test_enrich_single_entity_reports_dispatched_calls_apart() {
        let mut entity = json!({"name": "persist"});
        let references = json!({
            "calls": [],
            "dispatched_calls": [
                {"name": "archiveViaConcreteType", "file_path": "ReceiptClient.java"}
            ]
        });

        enrich_single_entity(&mut entity, &references);

        assert!(entity.get("caller_count").is_none());
        assert_eq!(entity.get("dispatched_caller_count"), Some(&json!(1)));
        assert_eq!(
            entity.get("dispatched_caller_samples"),
            Some(&json!(["archiveViaConcreteType in ReceiptClient.java"]))
        );
    }

    #[test]
    fn test_enrich_single_entity_counts_only_given_rows() {
        // The buckets belong to one UUID: every row counts, none is filtered
        // or merged by name (issue #31).
        let mut entity = json!({"name": "computeValue"});
        let references = json!({
            "calls": [{"name": "adjust", "file_path": "StandardAdjuster.java"}],
            "dispatched_calls": []
        });

        enrich_single_entity(&mut entity, &references);

        assert_eq!(entity.get("caller_count"), Some(&json!(1)));
        assert_eq!(
            entity.get("caller_samples"),
            Some(&json!(["adjust in StandardAdjuster.java"]))
        );
        assert!(entity.get("dispatched_caller_count").is_none());
    }

    /// Issue #50: test callers stay in the count but are sampled apart.
    #[test]
    fn test_enrich_single_entity_samples_test_callers_apart() {
        let mut entity = json!({"name": "login"});
        let references = json!({
            "calls": [
                {"name": "test_login", "file_path": "tests/login_test.rs"},
                {"name": "handle", "file_path": "src/api.rs"},
                {"name": "inline_case", "file_path": "src/auth.rs", "is_test_context": true},
                {"name": "spec", "file_path": "src/login.spec.ts"}
            ]
        });

        enrich_single_entity(&mut entity, &references);

        assert_eq!(entity.get("caller_count"), Some(&json!(4)));
        assert_eq!(
            entity.get("caller_samples"),
            Some(&json!(["handle in src/api.rs"]))
        );
        assert_eq!(entity.get("caller_test_count"), Some(&json!(3)));
        assert_eq!(
            entity.get("caller_test_samples"),
            Some(&json!([
                "test_login in tests/login_test.rs",
                "inline_case in src/auth.rs",
                "spec in src/login.spec.ts"
            ]))
        );
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
