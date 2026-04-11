use crate::mcp_handler::KnotMcpHandler;
use serde_json::json;

/// Enrich search results with related entities (subclasses, implementers, usages)
pub(crate) async fn enrich_with_relationships(
    context: &serde_json::Value,
    _entity_names: &[String],
    handler: &KnotMcpHandler,
    repo_name: Option<&str>,
) -> std::result::Result<serde_json::Value, String> {
    use crate::db::graph::QueryExt;
    let mut enriched = context.clone();

    // For each entity in the search results, find related entities
    if let Some(entities) = enriched.as_array_mut() {
        for entity in entities.iter_mut() {
            if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
                // Query for all references to this entity
                if let Ok(references) = handler.graph_db.find_references(name, repo_name).await {
                    // Add subclasses (EXTENDS)
                    if let Some(extends_arr) = references.get("extends").and_then(|v| v.as_array())
                        && !extends_arr.is_empty()
                    {
                        let extends_names: Vec<String> = extends_arr
                            .iter()
                            .filter_map(|e| {
                                e.get("name").and_then(|v| v.as_str()).map(String::from)
                            })
                            .collect();
                        if let Some(obj) = entity.as_object_mut() {
                            obj.insert("subclasses".to_string(), json!(extends_names));
                        }
                    }

                    // Add implementers (IMPLEMENTS)
                    if let Some(implements_arr) =
                        references.get("implements").and_then(|v| v.as_array())
                        && !implements_arr.is_empty()
                    {
                        let implements_names: Vec<String> = implements_arr
                            .iter()
                            .filter_map(|e| {
                                e.get("name").and_then(|v| v.as_str()).map(String::from)
                            })
                            .collect();
                        if let Some(obj) = entity.as_object_mut() {
                            obj.insert("implementers".to_string(), json!(implements_names));
                        }
                    }

                    // Add type usages (REFERENCES)
                    if let Some(references_arr) =
                        references.get("references").and_then(|v| v.as_array())
                        && !references_arr.is_empty()
                    {
                        let usage_count = references_arr.len();
                        let sample_usages: Vec<String> = references_arr
                            .iter()
                            .take(3)
                            .map(|e| {
                                let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let file =
                                    e.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
                                format!("{} in {}", name, file)
                            })
                            .collect();
                        if let Some(obj) = entity.as_object_mut() {
                            obj.insert("type_usage_count".to_string(), json!(usage_count));
                            obj.insert("type_usage_samples".to_string(), json!(sample_usages));
                        }
                    }

                    // Add callers (CALLS) - limit to 3 samples
                    if let Some(calls_arr) = references.get("calls").and_then(|v| v.as_array())
                        && !calls_arr.is_empty()
                    {
                        let caller_count = calls_arr.len();
                        let sample_callers: Vec<String> = calls_arr
                            .iter()
                            .take(3)
                            .map(|e| {
                                let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let file =
                                    e.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
                                format!("{} in {}", name, file)
                            })
                            .collect();
                        if let Some(obj) = entity.as_object_mut() {
                            obj.insert("caller_count".to_string(), json!(caller_count));
                            obj.insert("caller_samples".to_string(), json!(sample_callers));
                        }
                    }
                }
            }
        }
    }

    Ok(enriched)
}
