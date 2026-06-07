use anyhow::{Context, Result};
use neo4rs::{DetachedRowStream, query};
use tracing::info;

use super::GraphDb;

async fn collect_column_strings(rows: &mut DetachedRowStream, column: &str) -> Vec<String> {
    let mut result = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        if let Ok(val) = row.get::<String>(column) {
            result.push(val);
        }
    }
    result
}

/// Extension trait for repository dependency query operations.
#[allow(async_fn_in_trait)]
pub trait RepoQueryExt {
    async fn find_repo_dependencies(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>>;
    async fn find_repo_dependents(&self, repo_name: &str) -> Result<Vec<String>>;
    async fn find_repository_by_artifact(
        &self,
        group_id: &str,
        artifact_id: &str,
        build_system: &str,
    ) -> Result<Option<String>>;
}

impl RepoQueryExt for GraphDb {
    /// Find all repositories that this repo depends on (transitive, up to max_depth).
    async fn find_repo_dependencies(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>> {
        let cypher = format!(
            "MATCH (from:Repository {{name: $repo_name}})-[:DEPENDS_ON*1..{}]->(to:Repository)
             RETURN DISTINCT to.name AS dep_name",
            max_depth
        );

        let mut rows = self
            .graph
            .execute(query(&cypher).param("repo_name", repo_name))
            .await
            .context("Failed to query repository dependencies")?;

        let dependencies = collect_column_strings(&mut rows, "dep_name").await;

        info!(
            "Found {} repository dependencies for '{repo_name}' (depth {max_depth})",
            dependencies.len()
        );
        Ok(dependencies)
    }

    /// Find all repositories that depend on this repo (reverse lookup).
    async fn find_repo_dependents(&self, repo_name: &str) -> Result<Vec<String>> {
        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (dependent:Repository)-[:DEPENDS_ON]->(target:Repository {name: $repo_name})
                     RETURN DISTINCT dependent.name AS dep_name",
                )
                .param("repo_name", repo_name),
            )
            .await
            .context("Failed to query repository dependents")?;

        let dependents = collect_column_strings(&mut rows, "dep_name").await;

        info!(
            "Found {} repositories that depend on '{repo_name}'",
            dependents.len()
        );
        Ok(dependents)
    }

    /// Find a repository by its build system artifact identity.
    async fn find_repository_by_artifact(
        &self,
        group_id: &str,
        artifact_id: &str,
        build_system: &str,
    ) -> Result<Option<String>> {
        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (r:Repository)
                     WHERE r.build_system = $build_system
                       AND r.group_id = $group_id
                       AND r.artifact_id = $artifact_id
                     RETURN r.name AS repo_name",
                )
                .param("build_system", build_system)
                .param("group_id", group_id)
                .param("artifact_id", artifact_id),
            )
            .await
            .context("Failed to query repository by artifact identity")?;

        if let Ok(Some(row)) = rows.next().await
            && let Ok(name) = row.get::<String>("repo_name")
        {
            return Ok(Some(name));
        }

        Ok(None)
    }
}
