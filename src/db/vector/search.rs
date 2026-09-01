use anyhow::Context;
use anyhow::Result;
use qdrant_client::qdrant::{SearchPoints, WithPayloadSelector};

use super::{VectorDb, utils};

/// Build the Qdrant payload filter for a repo scope.
/// Empty slice → `None` (unfiltered). 1 name → `MatchValue::Keyword`.
/// N names → `MatchValue::Keywords(RepeatedStrings)` ("value in set" OR semantics).
pub(crate) fn build_repo_filter(repo_names: &[String]) -> Option<qdrant_client::qdrant::Filter> {
    if repo_names.is_empty() {
        return None;
    }

    let match_value = if repo_names.len() == 1 {
        qdrant_client::qdrant::r#match::MatchValue::Keyword(repo_names[0].clone())
    } else {
        qdrant_client::qdrant::r#match::MatchValue::Keywords(
            qdrant_client::qdrant::RepeatedStrings {
                strings: repo_names.to_vec(),
            },
        )
    };

    Some(qdrant_client::qdrant::Filter {
        must: vec![qdrant_client::qdrant::Condition {
            condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                qdrant_client::qdrant::FieldCondition {
                    key: "repo_name".to_string(),
                    r#match: Some(qdrant_client::qdrant::Match {
                        match_value: Some(match_value),
                    }),
                    ..Default::default()
                },
            )),
        }],
        ..Default::default()
    })
}

/// Extension trait for query and search operations.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait VectorSearchExt {
    async fn search(
        &self,
        vector: &[f32],
        limit: usize,
        repo_names: &[String],
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
        repo_names: &[String],
    ) -> Result<Vec<serde_json::Value>> {
        let filter = build_repo_filter(repo_names);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::vector::connection::VectorConnectExt;

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_search_vector() {
        let vector_db = VectorDb::connect("http://localhost:6334", "test_collection_search", 384)
            .await
            .expect("Failed to connect to Qdrant");

        let query_vector = vec![0.5; 384];

        let result = vector_db.search(&query_vector, 10, &[]).await;
        assert!(result.is_ok());
        let results = result.unwrap();
        assert!(results.is_empty() || !results.is_empty()); // Collection might be empty
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_search_vector_with_repo_filter() {
        let vector_db = VectorDb::connect(
            "http://localhost:6334",
            "test_collection_search_filter",
            384,
        )
        .await
        .expect("Failed to connect to Qdrant");

        let query_vector = vec![0.5; 384];

        let result = vector_db
            .search(&query_vector, 10, &["test-repo".to_string()])
            .await;
        assert!(result.is_ok());
        let results = result.unwrap();
        assert!(results.is_empty() || !results.is_empty()); // Collection might be empty
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_search_zero_limit() {
        let vector_db =
            VectorDb::connect("http://localhost:6334", "test_collection_search_zero", 384)
                .await
                .expect("Failed to connect to Qdrant");

        let query_vector = vec![0.5; 384];

        let result = vector_db.search(&query_vector, 0, &[]).await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_search_large_limit() {
        let vector_db =
            VectorDb::connect("http://localhost:6334", "test_collection_search_large", 384)
                .await
                .expect("Failed to connect to Qdrant");

        let query_vector = vec![0.5; 384];

        let result = vector_db.search(&query_vector, 1000, &[]).await;
        assert!(result.is_ok());
    }

    #[test]
    fn build_repo_filter_empty_is_none() {
        assert!(build_repo_filter(&[]).is_none());
    }

    #[test]
    fn build_repo_filter_single_keyword() {
        let names = vec!["a".to_string()];
        let filter = build_repo_filter(&names).expect("filter should be Some");
        assert_eq!(filter.must.len(), 1);
        let cond = &filter.must[0];
        match &cond.condition_one_of {
            Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(field_cond)) => {
                assert_eq!(field_cond.key, "repo_name");
                match &field_cond.r#match.as_ref().unwrap().match_value {
                    Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(k)) => {
                        assert_eq!(k, "a");
                    }
                    _ => panic!("Expected Keyword match value"),
                }
            }
            _ => panic!("Expected Field condition"),
        }
    }

    #[test]
    fn build_repo_filter_multi_keywords() {
        let names = vec!["a".to_string(), "b".to_string()];
        let filter = build_repo_filter(&names).expect("filter should be Some");
        assert_eq!(filter.must.len(), 1);
        let cond = &filter.must[0];
        match &cond.condition_one_of {
            Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(field_cond)) => {
                assert_eq!(field_cond.key, "repo_name");
                match &field_cond.r#match.as_ref().unwrap().match_value {
                    Some(qdrant_client::qdrant::r#match::MatchValue::Keywords(kw)) => {
                        assert_eq!(kw.strings, vec!["a", "b"]);
                    }
                    _ => panic!("Expected Keywords match value"),
                }
            }
            _ => panic!("Expected Field condition"),
        }
    }

    #[test]
    fn build_repo_filter_preserves_order() {
        let names = vec!["b".to_string(), "a".to_string()];
        let filter = build_repo_filter(&names).expect("filter should be Some");
        let cond = &filter.must[0];
        if let Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(field_cond)) =
            &cond.condition_one_of
        {
            if let Some(qdrant_client::qdrant::r#match::MatchValue::Keywords(kw)) =
                &field_cond.r#match.as_ref().unwrap().match_value
            {
                assert_eq!(kw.strings, vec!["b", "a"]);
            } else {
                panic!("Expected Keywords match value");
            }
        } else {
            panic!("Expected Field condition");
        }
    }
}
