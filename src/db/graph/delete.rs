use anyhow::{Context, Result};
use neo4rs::query;
use tracing::warn;

use super::GraphDb;

/// Extension trait for deletion operations.
#[allow(async_fn_in_trait)]
pub trait DeleteExt {
    async fn delete_by_repo(&self, repo_name: &str) -> Result<()>;
    async fn delete_by_file_paths(&self, repo_name: &str, file_paths: &[String]) -> Result<()>;
}

impl DeleteExt for GraphDb {
    /// Delete all entity nodes (and their relationships) whose `repo_name`
    /// exactly matches the provided name. Called before a full re-index.
    async fn delete_by_repo(&self, repo_name: &str) -> Result<()> {
        warn!(
            "Deleting existing graph nodes for repo '{}' from Neo4j",
            repo_name
        );

        self.graph
            .run(
                query(
                    "MATCH (e:Entity)
                     WHERE e.repo_name = $repo_name
                     DETACH DELETE e",
                )
                .param("repo_name", repo_name),
            )
            .await
            .context("Failed to delete existing Neo4j nodes")?;

        Ok(())
    }

    /// Delete entity nodes for specific file paths (incremental mode).
    ///
    /// Called when files are modified or deleted to remove stale nodes
    /// before re-indexing only the changed files.
    async fn delete_by_file_paths(&self, repo_name: &str, file_paths: &[String]) -> Result<()> {
        if file_paths.is_empty() {
            return Ok(());
        }

        warn!(
            "Deleting {} file(s) from repo '{}' in Neo4j (incremental mode)",
            file_paths.len(),
            repo_name
        );

        self.graph
            .run(
                query(
                    "MATCH (e:Entity)
                     WHERE e.repo_name = $repo_name AND e.file_path IN $file_paths
                     DETACH DELETE e",
                )
                .param("repo_name", repo_name)
                .param("file_paths", file_paths),
            )
            .await
            .context("Failed to delete Neo4j nodes by file paths")?;

        Ok(())
    }
}
