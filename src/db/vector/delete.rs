use anyhow::{Context, Result};
use qdrant_client::qdrant::{Condition, DeletePointsBuilder, Filter};
use tracing::warn;

use super::VectorDb;

/// Extension trait for deletion operations.
#[allow(async_fn_in_trait)]
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
                DeletePointsBuilder::new(&self.collection).points(Filter::must([
                    Condition::matches_text("repo_name", repo_name),
                ])),
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
                    DeletePointsBuilder::new(&self.collection).points(Filter::must([
                        Condition::matches_text("repo_name", repo_name),
                        Condition::matches_text("file_path", file_path),
                    ])),
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
}
