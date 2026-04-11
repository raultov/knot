use anyhow::Context;
use anyhow::Result;
use qdrant_client::qdrant::{SearchPoints, WithPayloadSelector};

use super::{VectorDb, utils};

/// Extension trait for query and search operations.
#[allow(async_fn_in_trait)]
pub trait VectorSearchExt {
    async fn search(
        &self,
        vector: &[f32],
        limit: usize,
        repo_name: Option<&str>,
    ) -> Result<Vec<serde_json::Value>>;
}

impl VectorSearchExt for VectorDb {
    /// Search for similar vectors in Qdrant.
    ///
    /// Returns the top N matching points with their payloads (metadata).
    async fn search(
        &self,
        vector: &[f32],
        limit: usize,
        repo_name: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        // Build search request with optional repo_name filter
        let filter = repo_name.map(|repo| qdrant_client::qdrant::Filter {
            must: vec![qdrant_client::qdrant::Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    qdrant_client::qdrant::FieldCondition {
                        key: "repo_name".to_string(),
                        r#match: Some(qdrant_client::qdrant::Match {
                            match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                repo.to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                )),
            }],
            ..Default::default()
        });

        let search_request = SearchPoints {
            collection_name: self.collection.clone(),
            vector: vector.to_vec(),
            limit: limit as u64,
            with_payload: Some(WithPayloadSelector {
                selector_options: Some(
                    qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
                ),
            }),
            filter,
            ..Default::default()
        };

        let search_result = self
            .client
            .search_points(search_request)
            .await
            .context("Failed to search Qdrant")?;

        let results = search_result
            .result
            .into_iter()
            .filter_map(|scored_point| {
                if !scored_point.payload.is_empty() {
                    let mut json_obj = serde_json::json!({});
                    for (key, value) in scored_point.payload {
                        json_obj[&key] = utils::qdrant_value_to_json(&value);
                    }
                    Some(json_obj)
                } else {
                    None
                }
            })
            .collect();

        Ok(results)
    }
}
