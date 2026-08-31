use anyhow::{Context, Result};
use neo4rs::{DetachedRowStream, query};
use tracing::info;

use super::GraphDb;

async fn collect_dep_edges(rows: &mut DetachedRowStream) -> Result<Vec<(String, String)>> {
    let mut edges = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        let from = row.get::<String>("from_repo").unwrap_or_default();
        let to = row.get::<String>("to_repo").unwrap_or_default();
        if !from.is_empty() && !to.is_empty() {
            edges.push((from, to));
        }
    }
    Ok(edges)
}

async fn collect_call_coupling(rows: &mut DetachedRowStream) -> Result<Vec<(String, String, i64)>> {
    let mut pairs = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        let from = row.get::<String>("from_repo").unwrap_or_default();
        let to = row.get::<String>("to_repo").unwrap_or_default();
        let count = row.get::<i64>("call_count").unwrap_or(0);
        if !from.is_empty() && !to.is_empty() && count > 0 {
            pairs.push((from, to, count));
        }
    }
    Ok(pairs)
}

/// Extension trait for portfolio-level graph queries across all indexed repos.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait PortfolioQueryExt {
    async fn list_repositories_extended(&self) -> Result<Vec<serde_json::Value>>;
    async fn list_repo_dependency_edges(&self) -> Result<Vec<(String, String)>>;
    async fn list_cross_repo_call_coupling(&self) -> Result<Vec<(String, String, i64)>>;
    async fn list_repo_documentation(
        &self,
        repo_names: &[String],
    ) -> Result<Vec<serde_json::Value>>;
}

impl PortfolioQueryExt for GraphDb {
    async fn list_repositories_extended(&self) -> Result<Vec<serde_json::Value>> {
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
                        coalesce(r.group_id, '') AS group_id,
                        coalesce(r.artifact_id, '') AS artifact_id,
                        coalesce(r.version, '') AS version,
                        CASE WHEN r.indexed_at IS NULL THEN '' ELSE toString(r.indexed_at) END AS indexed_at,
                        languages
                 ORDER BY r.name",
            ))
            .await
            .context("Failed to query extended repository list from Neo4j")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let name = row.get::<String>("name").unwrap_or_default();
            let entity_count = row.get::<i64>("entity_count").unwrap_or(0);
            let file_count = row.get::<i64>("file_count").unwrap_or(0);
            let build_system = row.get::<String>("build_system").unwrap_or_default();
            let group_id = row.get::<String>("group_id").unwrap_or_default();
            let artifact_id = row.get::<String>("artifact_id").unwrap_or_default();
            let version = row.get::<String>("version").unwrap_or_default();
            let indexed_at = row.get::<String>("indexed_at").unwrap_or_default();
            let languages: Vec<String> = row.get::<Vec<String>>("languages").unwrap_or_default();
            let primary_language = super::query_repo::most_common_language(&languages);

            results.push(serde_json::json!({
                "name": name,
                "entity_count": entity_count,
                "file_count": file_count,
                "build_system": build_system,
                "group_id": group_id,
                "artifact_id": artifact_id,
                "version": version,
                "indexed_at": indexed_at,
                "primary_language": primary_language,
            }));
        }

        info!(
            "Listed {} repositories for portfolio analysis",
            results.len()
        );
        Ok(results)
    }

    async fn list_repo_dependency_edges(&self) -> Result<Vec<(String, String)>> {
        let mut rows = self
            .graph
            .execute(query(
                "MATCH (from:Repository)-[:DEPENDS_ON]->(to:Repository)
                 RETURN from.name AS from_repo, to.name AS to_repo
                 ORDER BY from_repo, to_repo",
            ))
            .await
            .context("Failed to query repository dependency edges")?;

        collect_dep_edges(&mut rows).await
    }

    async fn list_cross_repo_call_coupling(&self) -> Result<Vec<(String, String, i64)>> {
        let mut rows = self
            .graph
            .execute(query(
                "MATCH (a:Entity)-[:CALLS]->(b:Entity)
                 WHERE a.repo_name <> b.repo_name
                 RETURN a.repo_name AS from_repo, b.repo_name AS to_repo, count(*) AS call_count
                 ORDER BY call_count DESC",
            ))
            .await
            .context("Failed to query cross-repo call coupling")?;

        let pairs = collect_call_coupling(&mut rows).await?;
        info!("Found {} cross-repo call coupling pairs", pairs.len());
        Ok(pairs)
    }

    async fn list_repo_documentation(
        &self,
        repo_names: &[String],
    ) -> Result<Vec<serde_json::Value>> {
        if repo_names.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (e:Entity)
                     WHERE e.repo_name IN $repo_names
                       AND (
                         (toLower(e.file_path) ENDS WITH 'readme.md'
                          AND e.kind IN ['markdown_document', 'markdown_section'])
                         OR e.kind IN ['project_identity', 'cargo_package']
                         OR (e.kind = 'config_property'
                             AND (toLower(e.name) = 'description'
                                  OR toLower(e.fqn) CONTAINS 'description'))
                       )
                     RETURN e.repo_name AS repo_name,
                            e.kind AS kind,
                            e.file_path AS file_path,
                            e.name AS name,
                            coalesce(e.signature, '') AS signature,
                            coalesce(e.docstring, '') AS docstring,
                            coalesce(e.embed_text, '') AS embed_text,
                            coalesce(e.start_line, 0) AS start_line
                     ORDER BY e.repo_name, e.file_path, e.start_line",
                )
                .param("repo_names", repo_names.to_vec()),
            )
            .await
            .context("Failed to query repository documentation")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            results.push(serde_json::json!({
                "repo_name": row.get::<String>("repo_name").unwrap_or_default(),
                "kind": row.get::<String>("kind").unwrap_or_default(),
                "file_path": row.get::<String>("file_path").unwrap_or_default(),
                "name": row.get::<String>("name").unwrap_or_default(),
                "signature": row.get::<String>("signature").unwrap_or_default(),
                "docstring": row.get::<String>("docstring").unwrap_or_default(),
                "embed_text": row.get::<String>("embed_text").unwrap_or_default(),
                "start_line": row.get::<i64>("start_line").unwrap_or(0),
            }));
        }

        info!(
            "Fetched {} documentation entities for portfolio",
            results.len()
        );
        Ok(results)
    }
}
