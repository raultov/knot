use std::collections::HashMap;

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
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait RepoQueryExt {
    async fn find_repo_dependencies(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>>;
    async fn find_repo_dependents(&self, repo_name: &str) -> Result<Vec<String>>;
    async fn find_repository_by_artifact(
        &self,
        group_id: &str,
        artifact_id: &str,
        build_system: &str,
    ) -> Result<Option<String>>;
    async fn list_repositories(&self) -> Result<Vec<serde_json::Value>>;
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

    /// List all indexed repositories with their entity count, file count, build
    /// system, and the most common language across their entities.
    ///
    /// The `:Repository` node carries build-system metadata (build_system,
    /// group_id, artifact_id, version) but **not** a `language` property.
    /// Language is therefore derived from the `language` property of the
    /// repository's entities by picking the most frequently occurring value.
    /// Entities are joined to repositories through the `repo_name` property
    /// (there is no explicit `BELONGS_TO` relationship in the graph).
    async fn list_repositories(&self) -> Result<Vec<serde_json::Value>> {
        let mut rows = self
            .graph
            .execute(query(
                "MATCH (r:Repository)
                 OPTIONAL MATCH (e:Entity) WHERE e.repo_name = r.name
                 WITH r,
                      [l IN collect(e.language) WHERE l IS NOT NULL] AS languages,
                      collect(DISTINCT e.file_path) AS files
                 RETURN r.name AS name,
                        size(languages) AS entity_count,
                        size([f IN files WHERE f IS NOT NULL]) AS file_count,
                        coalesce(r.build_system, '') AS build_system,
                        languages
                 ORDER BY r.name",
            ))
            .await
            .context("Failed to query repository list from Neo4j")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let name = row.get::<String>("name").unwrap_or_default();
            let entity_count = row.get::<i64>("entity_count").unwrap_or(0);
            let file_count = row.get::<i64>("file_count").unwrap_or(0);
            let build_system = row.get::<String>("build_system").unwrap_or_default();
            let languages: Vec<String> = row.get::<Vec<String>>("languages").unwrap_or_default();

            let primary_language = most_common_language(&languages);

            results.push(serde_json::json!({
                "name": name,
                "entity_count": entity_count,
                "file_count": file_count,
                "build_system": build_system,
                "primary_language": primary_language,
            }));
        }

        info!("Listed {} indexed repositories", results.len());
        Ok(results)
    }
}

/// Pick the most common non-empty string from `languages`.
///
/// Returns an empty string when the slice is empty, so callers can safely use
/// the result as a display value without further null handling.
pub(crate) fn most_common_language(languages: &[String]) -> String {
    if languages.is_empty() {
        return String::new();
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for lang in languages {
        if lang.is_empty() {
            continue;
        }
        *counts.entry(lang.as_str()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(lang, _)| lang.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::most_common_language;

    #[test]
    fn test_most_common_language_empty() {
        assert_eq!(most_common_language(&[]), "");
    }

    #[test]
    fn test_most_common_language_single() {
        assert_eq!(most_common_language(&["rust".to_string()]), "rust");
    }

    #[test]
    fn test_most_common_language_picks_winner() {
        let langs = vec![
            "rust".to_string(),
            "rust".to_string(),
            "java".to_string(),
            "kotlin".to_string(),
        ];
        assert_eq!(most_common_language(&langs), "rust");
    }

    #[test]
    fn test_most_common_language_skips_empty() {
        let langs = vec!["".to_string(), "rust".to_string(), "rust".to_string()];
        assert_eq!(most_common_language(&langs), "rust");
    }

    #[test]
    fn test_most_common_language_all_empty() {
        let langs = vec!["".to_string(), "".to_string()];
        assert_eq!(most_common_language(&langs), "");
    }

    #[test]
    fn test_most_common_language_tie_keeps_one_deterministically() {
        // When two languages tie, HashMap iteration order is not stable but
        // both are valid answers. Verify the result is one of the tied values.
        let langs = vec!["rust".to_string(), "java".to_string()];
        let result = most_common_language(&langs);
        assert!(result == "rust" || result == "java");
    }
}
