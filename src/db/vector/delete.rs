use anyhow::{Context, Result};
use qdrant_client::qdrant::{DeletePointsBuilder, Filter};
use tracing::warn;

use super::{VectorDb, utils};

/// Filter selecting every point belonging to `repo_name`.
///
/// Uses an exact keyword match, mirroring the `e.repo_name = $repo_name`
/// predicate of the Neo4j delete path. A full-text match here would also select
/// repositories whose name merely shares a token with `repo_name`.
fn repo_filter(repo_name: &str) -> Filter {
    Filter::must([utils::exact_keyword_condition("repo_name", repo_name)])
}

/// Filter selecting every point of `repo_name` that came from `file_path`.
fn repo_file_filter(repo_name: &str, file_path: &str) -> Filter {
    Filter::must([
        utils::exact_keyword_condition("repo_name", repo_name),
        utils::exact_keyword_condition("file_path", file_path),
    ])
}

/// Extension trait for deletion operations.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait VectorDeleteExt {
    async fn delete_by_repo(&self, repo_name: &str) -> Result<()>;
    async fn delete_by_file_paths(&self, repo_name: &str, file_paths: &[String]) -> Result<()>;
}

impl VectorDeleteExt for VectorDb {
    /// Delete all points in the collection whose `repo_name` payload field
    /// exactly matches the provided name. Called before a full re-index to avoid orphans.
    async fn delete_by_repo(&self, repo_name: &str) -> Result<()> {
        warn!(
            "Deleting existing vectors for repo '{}' from collection '{}'",
            repo_name, self.collection
        );

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection).points(repo_filter(repo_name)),
            )
            .await
            .context("Failed to delete existing vectors")?;

        Ok(())
    }

    /// Delete points for specific file paths (incremental mode).
    ///
    /// Called when files are modified or deleted to remove stale vectors
    /// before re-indexing only the changed files.
    async fn delete_by_file_paths(&self, repo_name: &str, file_paths: &[String]) -> Result<()> {
        if file_paths.is_empty() {
            return Ok(());
        }

        warn!(
            "Deleting {} file(s) from repo '{}' in Qdrant (incremental mode)",
            file_paths.len(),
            repo_name
        );

        // Delete each file individually (simpler than complex OR filters)
        // This is acceptable for incremental mode where file counts are low
        for file_path in file_paths {
            self.client
                .delete_points(
                    DeletePointsBuilder::new(&self.collection)
                        .points(repo_file_filter(repo_name, file_path)),
                )
                .await
                .with_context(|| format!("Failed to delete vectors for file: {}", file_path))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::vector::connection::VectorConnectExt;

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_vector_db_delete_by_repo() {
        let vector_db = VectorDb::connect("http://localhost:6334", "test_collection_delete", 384)
            .await
            .expect("Failed to connect to Qdrant");

        let result = vector_db.delete_by_repo("nonexistent-test-repo").await;
        // Should not fail even if repo doesn't exist
        assert!(result.is_ok());
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_vector_db_delete_by_file_paths() {
        let vector_db =
            VectorDb::connect("http://localhost:6334", "test_collection_delete_files", 384)
                .await
                .expect("Failed to connect to Qdrant");

        let file_paths = vec![
            "/test/path/File1.java".to_string(),
            "/test/path/File2.java".to_string(),
        ];

        let result = vector_db
            .delete_by_file_paths("nonexistent-test-repo", &file_paths)
            .await;
        // Should not fail even if repo/files don't exist
        assert!(result.is_ok());
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_vector_db_delete_by_file_paths_empty() {
        let vector_db =
            VectorDb::connect("http://localhost:6334", "test_collection_delete_empty", 384)
                .await
                .expect("Failed to connect to Qdrant");

        let result = vector_db.delete_by_file_paths("test-repo", &[]).await;
        // Should return Ok immediately without querying
        assert!(result.is_ok());
    }

    /// Extract the `MatchValue` of the condition at `idx` of a `must` filter.
    fn match_value_at(
        filter: &Filter,
        idx: usize,
    ) -> (String, qdrant_client::qdrant::r#match::MatchValue) {
        let Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(field)) =
            &filter.must[idx].condition_one_of
        else {
            panic!("Expected a field condition at index {idx}");
        };
        let value = field
            .r#match
            .as_ref()
            .expect("condition must carry a match")
            .match_value
            .clone()
            .expect("match must carry a value");
        (field.key.clone(), value)
    }

    /// Regression: `delete_by_repo` used `Condition::matches_text`, a full-text
    /// match. Indexing `knot` then wiped the vectors of `knot-server` and
    /// `knot-site`, and indexing `job-watch` wiped `job-watch-ui`, leaving those
    /// repos present in Neo4j but unsearchable via the vector store.
    #[test]
    fn repo_filter_uses_exact_keyword_not_full_text() {
        let filter = repo_filter("knot");
        assert_eq!(filter.must.len(), 1);

        let (key, value) = match_value_at(&filter, 0);
        assert_eq!(key, "repo_name");
        match value {
            qdrant_client::qdrant::r#match::MatchValue::Keyword(k) => assert_eq!(k, "knot"),
            other => panic!("Expected an exact Keyword match, got {other:?}"),
        }
    }

    #[test]
    fn repo_file_filter_matches_both_fields_exactly() {
        let filter = repo_file_filter("job-watch", "src/main.rs");
        assert_eq!(filter.must.len(), 2);

        let (repo_key, repo_value) = match_value_at(&filter, 0);
        assert_eq!(repo_key, "repo_name");
        match repo_value {
            qdrant_client::qdrant::r#match::MatchValue::Keyword(k) => assert_eq!(k, "job-watch"),
            other => panic!("Expected an exact Keyword match, got {other:?}"),
        }

        let (path_key, path_value) = match_value_at(&filter, 1);
        assert_eq!(path_key, "file_path");
        match path_value {
            qdrant_client::qdrant::r#match::MatchValue::Keyword(k) => assert_eq!(k, "src/main.rs"),
            other => panic!("Expected an exact Keyword match, got {other:?}"),
        }
    }

    /// The delete filter must be byte-for-byte identical to the search filter,
    /// otherwise a repo can be deletable but not findable (or vice versa).
    #[test]
    fn repo_filter_agrees_with_search_repo_filter() {
        let names = vec!["knot-site".to_string()];
        let search_filter = crate::db::vector::search::build_search_filter(&names, &[], &[])
            .expect("non-empty scope yields a filter");
        assert_eq!(repo_filter("knot-site"), search_filter);
    }
}
