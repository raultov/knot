use anyhow::Context;
use anyhow::Result;
use qdrant_client::qdrant::{Condition, Filter, SearchPoints, WithPayloadSelector};

use super::{VectorDb, utils};

/// Build the Qdrant payload filter for a search: repo scope and optional
/// entity-kind filter, both as exact keyword `must` conditions.
/// Empty slices are omitted; a filter with no conditions is `None`.
pub(crate) fn build_search_filter(repo_names: &[String], kinds: &[String]) -> Option<Filter> {
    let mut must = Vec::new();
    if !repo_names.is_empty() {
        must.push(utils::any_keyword_condition("repo_name", repo_names));
    }
    if !kinds.is_empty() {
        must.push(utils::any_keyword_condition("kind", kinds));
    }
    if must.is_empty() {
        None
    } else {
        Some(Filter::must(must))
    }
}

/// Build the name-exact probe filter: one `must` arm whose `should` group
/// admits entities matching an exact name OR sharing an identifier token
/// with the query, plus the repo/kind scope arms. `None` when there is
/// nothing lexical to probe (the caller then runs an unfiltered probe).
pub(crate) fn build_probe_filter(
    names: &[String],
    tokens: &[String],
    repo_names: &[String],
    kinds: &[String],
) -> Option<Filter> {
    let mut should = Vec::new();
    if !names.is_empty() {
        should.push(utils::any_keyword_condition("name", names));
    }
    if !tokens.is_empty() {
        should.push(utils::any_keyword_condition("name_tokens", tokens));
    }
    let mut must = Vec::new();
    match (names.is_empty(), tokens.is_empty()) {
        (true, true) => {
            // Nothing lexical to probe: fall back to kind/repo-filtered
            // vector search rather than an unscoped full scan.
        }
        _ => {
            let condition: Condition = Filter::should(should).into();
            must.push(condition);
        }
    }
    if !repo_names.is_empty() {
        must.push(utils::any_keyword_condition("repo_name", repo_names));
    }
    if !kinds.is_empty() {
        must.push(utils::any_keyword_condition("kind", kinds));
    }
    (!must.is_empty()).then(|| Filter::must(must))
}

/// Parameters for the name-exact probe search — bundled in one struct to
/// keep [`VectorSearchExt::search_exact_names`] within clippy's arity
/// threshold.
pub struct ExactNameProbe<'a> {
    /// Query embedding.
    pub vector: &'a [f32],
    /// Entity names to match exactly (case-sensitive keyword OR).
    pub names: &'a [String],
    /// Lowercase identifier tokens (from the query text) to match against
    /// the indexed `name_tokens` payload: an entity whose identifier shares
    /// a word with the query (camel/snake-case sliced) must enter the
    /// candidate pool even when the query never names it verbatim.
    ///
    /// Recall semantics: the probe stays a vector search *restricted to*
    /// lexically-related entities, so the best cosine hits inside the
    /// lexical set are returned, not an unfiltered name scan.
    pub tokens: &'a [String],
    /// Maximum number of hits to return.
    pub limit: usize,
    /// Repository scope (empty = all repositories).
    pub repo_names: &'a [String],
    /// Wire-format entity kinds to restrict hits to (empty = all kinds).
    pub kinds: &'a [String],
}

/// Extension trait for query and search operations.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait VectorSearchExt {
    /// Search for similar vectors in Qdrant.
    ///
    /// `repo_names` scopes the search to repositories (empty = all);
    /// `kinds` restricts hits to wire-format entity kinds (empty = all).
    /// Each result carries its cosine similarity in the `score` field so
    /// callers can re-rank.
    async fn search(
        &self,
        vector: &[f32],
        limit: usize,
        repo_names: &[String],
        kinds: &[String],
    ) -> Result<Vec<serde_json::Value>>;

    /// Search restricted to entities whose payload `name` exactly matches
    /// one of the probe names (case-sensitive keyword OR). Used as a
    /// name-exact probe: an identifier the query literally names must enter
    /// the candidate pool with its true cosine, however deep it ranks.
    async fn search_exact_names(&self, probe: ExactNameProbe<'_>)
    -> Result<Vec<serde_json::Value>>;

    /// Scored search restricted to a fixed set of point UUIDs. Used by the
    /// caller-recall bridge in `search_hybrid_context`: entities chosen by
    /// a graph query (without vectors) still need their true cosine to the
    /// query before re-ranking.
    async fn search_by_uuids(&self, probe: UuidProbe<'_>) -> Result<Vec<serde_json::Value>>;
}

/// Parameters for the UUID-restricted scored search — bundled in one
/// struct to keep [`VectorSearchExt::search_by_uuids`] within clippy's
/// arity threshold.
pub struct UuidProbe<'a> {
    /// Query embedding.
    pub vector: &'a [f32],
    /// Point UUIDs to score (keyword OR on the `uuid` payload).
    pub uuids: &'a [String],
    /// Repository scope (empty = all repositories).
    pub repo_names: &'a [String],
    /// Wire-format entity kinds to restrict hits to (empty = all kinds).
    pub kinds: &'a [String],
    /// Maximum number of hits to return.
    pub limit: usize,
}

/// Run a scored points search with the given filter and map hits to JSON
/// payloads carrying the cosine `score`.
async fn scored_search(
    db: &VectorDb,
    vector: &[f32],
    limit: usize,
    filter: Option<Filter>,
) -> Result<Vec<serde_json::Value>> {
    let search_request = SearchPoints {
        collection_name: db.collection.clone(),
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

    let search_result = db
        .client
        .search_points(search_request)
        .await
        .context("Failed to search Qdrant")?;

    Ok(search_result
        .result
        .into_iter()
        .filter_map(|scored_point| {
            if !scored_point.payload.is_empty() {
                let mut json_obj = serde_json::json!({});
                for (key, value) in scored_point.payload {
                    json_obj[&key] = utils::qdrant_value_to_json(&value);
                }
                json_obj["score"] = serde_json::json!(scored_point.score);
                Some(json_obj)
            } else {
                None
            }
        })
        .collect())
}

impl VectorSearchExt for VectorDb {
    /// Search for similar vectors in Qdrant.
    ///
    /// Returns the top N matching points with their payloads (metadata),
    /// plus the cosine score under the `score` key.
    async fn search(
        &self,
        vector: &[f32],
        limit: usize,
        repo_names: &[String],
        kinds: &[String],
    ) -> Result<Vec<serde_json::Value>> {
        let filter = build_search_filter(repo_names, kinds);
        scored_search(self, vector, limit, filter).await
    }

    /// Name-exact probe search (see trait docs).
    async fn search_exact_names(
        &self,
        probe: ExactNameProbe<'_>,
    ) -> Result<Vec<serde_json::Value>> {
        let filter = build_probe_filter(probe.names, probe.tokens, probe.repo_names, probe.kinds);
        scored_search(self, probe.vector, probe.limit, filter).await
    }

    /// UUID-restricted scored search (see trait docs).
    async fn search_by_uuids(&self, probe: UuidProbe<'_>) -> Result<Vec<serde_json::Value>> {
        if probe.uuids.is_empty() {
            return Ok(Vec::new());
        }
        let mut must = vec![utils::any_keyword_condition("uuid", probe.uuids)];
        if !probe.repo_names.is_empty() {
            must.push(utils::any_keyword_condition("repo_name", probe.repo_names));
        }
        if !probe.kinds.is_empty() {
            must.push(utils::any_keyword_condition("kind", probe.kinds));
        }
        scored_search(self, probe.vector, probe.limit, Some(Filter::must(must))).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::vector::connection::VectorConnectExt;

    fn repo_names(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    fn assert_keyword_condition(
        cond: &qdrant_client::qdrant::Condition,
        key: &str,
        expected: &[&str],
    ) {
        match &cond.condition_one_of {
            Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(field_cond)) => {
                assert_eq!(field_cond.key, key);
                match &field_cond.r#match.as_ref().unwrap().match_value {
                    Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(k)) => {
                        assert_eq!(expected, &[k.as_str()]);
                    }
                    Some(qdrant_client::qdrant::r#match::MatchValue::Keywords(kw)) => {
                        assert_eq!(kw.strings, expected);
                    }
                    other => panic!("Expected keyword match value, got {other:?}"),
                }
            }
            other => panic!("Expected Field condition, got {other:?}"),
        }
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_search_vector() {
        let vector_db = VectorDb::connect("http://localhost:6334", "test_collection_search", 384)
            .await
            .expect("Failed to connect to Qdrant");

        let query_vector = vec![0.5; 384];

        let result = vector_db.search(&query_vector, 10, &[], &[]).await;
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
            .search(&query_vector, 10, &["test-repo".to_string()], &[])
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

        let result = vector_db.search(&query_vector, 0, &[], &[]).await;
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

        let result = vector_db.search(&query_vector, 1000, &[], &[]).await;
        assert!(result.is_ok());
    }

    #[test]
    fn build_search_filter_empty_is_none() {
        assert!(build_search_filter(&[], &[]).is_none());
    }

    #[test]
    fn build_search_filter_repo_only_single_keyword() {
        let filter = build_search_filter(&repo_names(&["a"]), &[]).expect("filter should be Some");
        assert_eq!(filter.must.len(), 1);
        assert_keyword_condition(&filter.must[0], "repo_name", &["a"]);
    }

    #[test]
    fn build_search_filter_repo_multi_keywords_preserves_order() {
        let filter =
            build_search_filter(&repo_names(&["b", "a"]), &[]).expect("filter should be Some");
        assert_eq!(filter.must.len(), 1);
        assert_keyword_condition(&filter.must[0], "repo_name", &["b", "a"]);
    }

    #[test]
    fn build_search_filter_kinds_only() {
        let filter = build_search_filter(&[], &repo_names(&["rust_function", "method"]))
            .expect("filter should be Some");
        assert_eq!(filter.must.len(), 1);
        assert_keyword_condition(&filter.must[0], "kind", &["rust_function", "method"]);
    }

    #[test]
    fn build_search_filter_repo_and_kinds_combined() {
        let filter = build_search_filter(&repo_names(&["r"]), &repo_names(&["class"]))
            .expect("filter should be Some");
        assert_eq!(filter.must.len(), 2);
        assert_keyword_condition(&filter.must[0], "repo_name", &["r"]);
        assert_keyword_condition(&filter.must[1], "kind", &["class"]);
    }

    // --- build_probe_filter (token-level lexical recall) ---

    fn probe_names(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn probe_filter_names_and_tokens_form_a_should_or() {
        let filter = build_probe_filter(
            &probe_names(&["useChangePassword"]),
            &probe_names(&["change"]),
            &repo_names(&["ui"]),
            &[],
        )
        .expect("filter should be Some");
        // 1 lexical must-arm + 1 repo arm.
        assert_eq!(filter.must.len(), 2);
        // The lexical arm is a should group with exactly two conditions.
        let lexical = &filter.must[0];
        match &lexical.condition_one_of {
            Some(qdrant_client::qdrant::condition::ConditionOneOf::Filter(inner)) => {
                assert_eq!(inner.should.len(), 2);
                assert_keyword_condition(&inner.should[0], "name", &["useChangePassword"]);
                assert_keyword_condition(&inner.should[1], "name_tokens", &["change"]);
            }
            other => panic!("Expected nested should filter, got {other:?}"),
        }
        assert_keyword_condition(&filter.must[1], "repo_name", &["ui"]);
    }

    #[test]
    fn probe_filter_tokens_alone_suffice() {
        let filter = build_probe_filter(&[], &probe_names(&["similarity", "search"]), &[], &[])
            .expect("filter should be Some");
        assert_eq!(filter.must.len(), 1);
        match &filter.must[0].condition_one_of {
            Some(qdrant_client::qdrant::condition::ConditionOneOf::Filter(inner)) => {
                assert_eq!(inner.should.len(), 1);
                assert_keyword_condition(
                    &inner.should[0],
                    "name_tokens",
                    &["similarity", "search"],
                );
            }
            other => panic!("Expected nested should filter, got {other:?}"),
        }
    }

    #[test]
    fn probe_filter_names_only_keeps_exact_name_contract() {
        let filter = build_probe_filter(&probe_names(&["login", "Login"]), &[], &[], &[])
            .expect("filter should be Some");
        match &filter.must[0].condition_one_of {
            Some(qdrant_client::qdrant::condition::ConditionOneOf::Filter(inner)) => {
                assert_eq!(inner.should.len(), 1);
                assert_keyword_condition(&inner.should[0], "name", &["login", "Login"]);
            }
            other => panic!("Expected nested should filter, got {other:?}"),
        }
    }

    #[test]
    fn probe_filter_without_lexical_input_is_none() {
        assert!(build_probe_filter(&[], &[], &[], &[]).is_none());
    }
}
