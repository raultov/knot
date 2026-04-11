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
