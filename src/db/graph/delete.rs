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

#[cfg(test)]
mod tests {
    use super::super::GraphDb;
    use super::DeleteExt;
    use crate::db::graph::connection::ConnectExt;

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_graph_db_delete_by_repo() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.delete_by_repo("nonexistent-test-repo").await;
        // Should not fail even if repo doesn't exist
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_graph_db_delete_by_file_paths() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let file_paths = vec![
            "/test/path/File1.java".to_string(),
            "/test/path/File2.java".to_string(),
        ];

        let result = graph_db
            .delete_by_file_paths("nonexistent-test-repo", &file_paths)
            .await;
        // Should not fail even if repo/files don't exist
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_graph_db_delete_by_file_paths_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.delete_by_file_paths("test-repo", &[]).await;
        // Should return Ok immediately without querying
        assert!(result.is_ok());
    }
}
