//! Core search_hybrid_context logic shared between CLI and MCP
//!
//! Performs a hybrid search combining:
//! 1. Semantic search via Qdrant vector similarity (understands code meaning)
//! 2. Structural expansion via Neo4j graph relationships (understands architecture)

use serde_json::json;
use std::sync::{Arc, Mutex};

use crate::db::{
    graph::{GraphDb, QueryExt},
    vector::{VectorDb, VectorSearchExt},
};
use crate::pipeline::embed::Embedder;

/// Main search function called by both CLI and MCP
pub async fn run_search_hybrid_context(
    query: &str,
    max_results: usize,
    repo_name: Option<&str>,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    embedder: &Arc<Mutex<Embedder>>,
) -> anyhow::Result<String> {
    // Step 1: Embed the query using fastembed
    let vector = embedder
        .lock()
        .unwrap()
        .embed_query(query)
        .map_err(|e| anyhow::anyhow!("Embedding failed: {}", e))?;

    // Step 2: Search Qdrant for similar vectors
    let search_results = vector_db.search(&vector, max_results, repo_name).await?;

    if search_results.is_empty() {
        return Ok("No matching code found for your query.".to_string());
    }

    // Extract UUIDs and names from search results
    let uuids: Vec<String> = search_results
        .iter()
        .filter_map(|result| {
            result
                .get("uuid")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();

    let entity_names: Vec<String> = search_results
        .iter()
        .filter_map(|result| {
            result
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();

    // Step 3: Query Neo4j for detailed context and dependencies
    let context = graph_db
        .get_entities_with_dependencies(&uuids, repo_name)
        .await?;

    // Step 4: Enrich context with related entities (subclasses, implementers, references)
    let enriched_context = enrich_with_relationships(&context, &entity_names, graph_db, repo_name)
        .await
        .unwrap_or(context);

    // Step 5: Format results as Markdown
    let formatted = format_search_results(&enriched_context);

    Ok(formatted)
}

/// Enrich search results with related entities (subclasses, implementers, usages)
async fn enrich_with_relationships(
    context: &serde_json::Value,
    _entity_names: &[String],
    graph_db: &Arc<GraphDb>,
    repo_name: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let mut enriched = context.clone();

    // For each entity in the search results, find related entities
    if let Some(entities) = enriched.as_array_mut() {
        for entity in entities.iter_mut() {
            if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
                // Query for all references to this entity
                if let Ok(references) = graph_db.find_references(name, repo_name).await {
                    enrich_single_entity(entity, &references);
                }
            }
        }
    }

    Ok(enriched)
}

/// Extract subclass names from an extends relationship array.
fn extract_subclass_names(extends_arr: &[serde_json::Value]) -> Vec<String> {
    extends_arr
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

/// Extract implementer names from an implements relationship array.
fn extract_implementer_names(implements_arr: &[serde_json::Value]) -> Vec<String> {
    implements_arr
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

/// Format type usage samples from a references array (limited to 3 samples).
fn format_usage_samples(references_arr: &[serde_json::Value]) -> Vec<String> {
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

/// Format caller samples from a calls array (limited to 3 samples).
fn format_caller_samples(calls_arr: &[serde_json::Value]) -> Vec<String> {
    calls_arr
        .iter()
        .take(3)
        .map(|e| {
            let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let file = e.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{} in {}", name, file)
        })
        .collect()
}

/// Enrich a single entity with relationship data extracted from references.
/// This is a pure function that operates only on JSON data.
fn enrich_single_entity(entity: &mut serde_json::Value, references: &serde_json::Value) {
    // Add subclasses (EXTENDS)
    if let Some(extends_arr) = references.get("extends").and_then(|v| v.as_array())
        && !extends_arr.is_empty()
    {
        let subclasses = extract_subclass_names(extends_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("subclasses".to_string(), json!(subclasses));
        }
    }

    // Add implementers (IMPLEMENTS)
    if let Some(implements_arr) = references.get("implements").and_then(|v| v.as_array())
        && !implements_arr.is_empty()
    {
        let implementers = extract_implementer_names(implements_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("implementers".to_string(), json!(implementers));
        }
    }

    // Add type usages (REFERENCES)
    if let Some(references_arr) = references.get("references").and_then(|v| v.as_array())
        && !references_arr.is_empty()
    {
        let usage_count = references_arr.len();
        let samples = format_usage_samples(references_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("type_usage_count".to_string(), json!(usage_count));
            obj.insert("type_usage_samples".to_string(), json!(samples));
        }
    }

    // Add callers (CALLS)
    if let Some(calls_arr) = references.get("calls").and_then(|v| v.as_array())
        && !calls_arr.is_empty()
    {
        let caller_count = calls_arr.len();
        let samples = format_caller_samples(calls_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("caller_count".to_string(), json!(caller_count));
            obj.insert("caller_samples".to_string(), json!(samples));
        }
    }
}

/// Format search results as Markdown
fn format_search_results(context: &serde_json::Value) -> String {
    use crate::mcp_tools::search_hybrid_context::format::format_search_results as mcp_format;
    mcp_format(context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_subclass_names_empty() {
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
    fn test_format_usage_samples_limits_to_three() {
        let refs = vec![
            json!({"name": "usage1", "file_path": "file1.java"}),
            json!({"name": "usage2", "file_path": "file2.java"}),
            json!({"name": "usage3", "file_path": "file3.java"}),
            json!({"name": "usage4", "file_path": "file4.java"}),
            json!({"name": "usage5", "file_path": "file5.java"}),
        ];
        let samples = format_usage_samples(&refs);
        assert_eq!(samples.len(), 3);
        assert!(samples[0].contains("usage1"));
        assert!(samples[1].contains("usage2"));
        assert!(samples[2].contains("usage3"));
    }

    #[test]
    fn test_format_caller_samples() {
        let refs = vec![json!({"name": "caller1", "file_path": "caller.java"})];
        let samples = format_caller_samples(&refs);
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
}
