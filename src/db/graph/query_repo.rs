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

async fn collect_column_pairs(
    rows: &mut DetachedRowStream,
    column_a: &str,
    column_b: &str,
) -> Vec<(String, String)> {
    let mut result = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        if let (Ok(a), Ok(b)) = (row.get::<String>(column_a), row.get::<String>(column_b)) {
            result.push((a, b));
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
    async fn find_repo_dependents(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>>;
    async fn find_repository_by_artifact(
        &self,
        group_id: &str,
        artifact_id: &str,
        build_system: &str,
    ) -> Result<Option<String>>;
    async fn find_build_dependency_names(&self, repo_name: &str) -> Result<Vec<String>>;
    async fn find_repository_identity(&self, repo_name: &str) -> Result<Option<RepoIdentity>>;
    async fn find_dependency_candidates(
        &self,
        needle: &str,
        exclude_repo: &str,
    ) -> Result<Vec<(String, String)>>;
    /// The embedding-model markers of the requested repositories.
    ///
    /// An empty `repo_names` slice ⇒ every `:Repository` node. Repositories
    /// indexed before the marker existed come back with `None` fields (the
    /// startup guard treats them as legacy and infers the model from the
    /// collection dimension).
    async fn repo_embed_markers(
        &self,
        repo_names: &[String],
    ) -> Result<Vec<crate::startup_guard::RepoEmbedMarker>>;
    /// Shared traversal behind [`RepoQueryExt::find_repo_dependencies`] and
    /// [`RepoQueryExt::find_repo_dependents`]: the two directions differ only
    /// in the edge pattern.
    async fn traverse_depends_on(
        &self,
        repo_name: &str,
        max_depth: u32,
        reverse: bool,
    ) -> Result<Vec<String>>;
    async fn list_repositories(&self) -> Result<Vec<serde_json::Value>>;
}

/// Build-system identity recorded on a `:Repository` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    pub build_system: String,
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
}

impl RepoQueryExt for GraphDb {
    /// Find all repositories that this repo depends on (transitive, up to max_depth).
    async fn find_repo_dependencies(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>> {
        self.traverse_depends_on(repo_name, max_depth, false).await
    }

    /// Find all repositories that depend on this repo (reverse lookup,
    /// transitive, up to max_depth).
    async fn find_repo_dependents(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>> {
        self.traverse_depends_on(repo_name, max_depth, true).await
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

    /// All persisted `build_dependency` entity names for a repository,
    /// including those from manifests that the current incremental batch did
    /// not reparse. Used by cross-repo linking so a consumer whose manifest
    /// is unchanged still links newly indexed dependencies.
    async fn find_build_dependency_names(&self, repo_name: &str) -> Result<Vec<String>> {
        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (e:Entity {kind: 'build_dependency', repo_name: $repo_name})
                     RETURN DISTINCT e.name AS dep_name
                     ORDER BY dep_name",
                )
                .param("repo_name", repo_name),
            )
            .await
            .context("Failed to query persisted build dependencies")?;
        Ok(collect_column_strings(&mut rows, "dep_name").await)
    }

    /// The build-system identity recorded on a `:Repository` node, or `None`
    /// when no such repository node exists (never indexed).
    async fn find_repository_identity(&self, repo_name: &str) -> Result<Option<RepoIdentity>> {
        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (r:Repository {name: $repo_name})
                     RETURN coalesce(r.build_system, '') AS build_system,
                            coalesce(r.group_id, '')     AS group_id,
                            coalesce(r.artifact_id, '')  AS artifact_id,
                            coalesce(r.version, '')      AS version",
                )
                .param("repo_name", repo_name),
            )
            .await
            .context("Failed to query repository identity")?;
        if let Ok(Some(row)) = rows.next().await {
            let build_system = row.get::<String>("build_system").unwrap_or_default();
            let group_id = row.get::<String>("group_id").unwrap_or_default();
            let artifact_id = row.get::<String>("artifact_id").unwrap_or_default();
            let version = row.get::<String>("version").unwrap_or_default();
            return Ok(Some(RepoIdentity {
                build_system,
                group_id,
                artifact_id,
                version,
            }));
        }
        Ok(None)
    }

    /// Graph-wide candidates for the reverse dependency sweep: every
    /// `build_dependency` entity name that contains `needle`, together with
    /// the repository that declares it, excluding `exclude_repo`.
    ///
    /// `kind` is deliberately a `WHERE` filter rather than an inline map
    /// pattern so the planner selects a `NodeIndexContainsScan` on the
    /// `entity_name_text` TEXT index (verified with PROFILE: 1 total DB
    /// hit graph-wide). Do not inline it.
    async fn find_dependency_candidates(
        &self,
        needle: &str,
        exclude_repo: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (e:Entity)
                     WHERE e.name CONTAINS $needle
                       AND e.kind = 'build_dependency'
                       AND e.repo_name <> $exclude_repo
                     RETURN DISTINCT e.repo_name AS consumer, e.name AS dep_name
                     ORDER BY consumer, dep_name",
                )
                .param("needle", needle)
                .param("exclude_repo", exclude_repo),
            )
            .await
            .context("Failed to query dependency candidates for reverse sweep")?;
        Ok(collect_column_pairs(&mut rows, "consumer", "dep_name").await)
    }

    async fn repo_embed_markers(
        &self,
        repo_names: &[String],
    ) -> Result<Vec<crate::startup_guard::RepoEmbedMarker>> {
        // Empty name list ⇒ every repository node.
        let (where_clause, param) = if repo_names.is_empty() {
            ("", None)
        } else {
            (" WHERE r.name IN $repo_names", Some(repo_names.to_vec()))
        };
        let cypher = format!(
            "MATCH (r:Repository){where_clause}
             RETURN r.name AS repo_name,
                    coalesce(r.embed_model, '') AS embed_model,
                    coalesce(r.embed_dim, -1) AS embed_dim,
                    coalesce(r.qdrant_collection, '') AS qdrant_collection
             ORDER BY r.name"
        );
        let q = match &param {
            Some(names) => query(&cypher).param("repo_names", names.to_vec()),
            None => query(&cypher),
        };
        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query :Repository embed markers")?;

        let mut markers = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let name = row.get::<String>("repo_name").unwrap_or_default();
            let model = row.get::<String>("embed_model").unwrap_or_default();
            let dim = row.get::<i64>("embed_dim").unwrap_or(-1);
            let collection = row.get::<String>("qdrant_collection").unwrap_or_default();
            markers.push(crate::startup_guard::RepoEmbedMarker::from_row(
                name, model, dim, collection,
            ));
        }
        Ok(markers)
    }

    /// Traverse the `DEPENDS_ON` graph from `repo_name` up to `max_depth`
    /// hops, in the requested direction (forward = dependencies of the repo,
    /// reverse = repositories that depend on it).
    ///
    /// One shared builder keeps the forward and reverse queries in lockstep
    /// (same depth semantics, same self-exclusion, same ordering) — the two
    /// directions differ only in the edge pattern, so two handwritten
    /// builders would be a drift hazard and a near-duplicate pair.
    async fn traverse_depends_on(
        &self,
        repo_name: &str,
        max_depth: u32,
        reverse: bool,
    ) -> Result<Vec<String>> {
        let pattern = if reverse {
            format!(
                "MATCH (other:Repository)-[:DEPENDS_ON*1..{max_depth}]->(r:Repository {{name: $repo_name}})"
            )
        } else {
            format!(
                "MATCH (r:Repository {{name: $repo_name}})-[:DEPENDS_ON*1..{max_depth}]->(other:Repository)"
            )
        };
        let cypher = format!(
            "{pattern}
             WHERE other.name <> $repo_name
             RETURN DISTINCT other.name AS dep_name
             ORDER BY dep_name"
        );

        let mut rows = self
            .graph
            .execute(query(&cypher).param("repo_name", repo_name))
            .await
            .with_context(|| {
                format!(
                    "Failed to query {} of '{repo_name}' (depth {max_depth})",
                    if reverse {
                        "dependents"
                    } else {
                        "dependencies"
                    }
                )
            })?;

        let names = collect_column_strings(&mut rows, "dep_name").await;

        info!(
            "Found {} {} of '{repo_name}' (depth {max_depth})",
            names.len(),
            if reverse {
                "dependents"
            } else {
                "dependencies"
            }
        );
        Ok(names)
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
fn most_common_language(languages: &[String]) -> String {
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
