use anyhow::{Context, Result};
use neo4rs::query;

use super::GraphDb;

/// Extension trait for query and read operations.
#[allow(async_fn_in_trait)]
pub trait QueryExt {
    async fn get_entities_with_dependencies(
        &self,
        uuids: &[String],
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value>;
    async fn find_references(
        &self,
        entity_name: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value>;
    async fn find_callers(
        &self,
        entity_name: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value>;
    async fn get_file_entities(
        &self,
        file_path: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value>;
    async fn find_entities_by_name_prefix(
        &self,
        prefix: &str,
        repo_name: Option<&str>,
        limit: usize,
    ) -> Result<serde_json::Value>;
    async fn get_file_outgoing_references(
        &self,
        file_path: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value>;
    /// Suffix-based fallback used by `explore_file` (§4 of
    /// `docs/specs/relative_file_paths.md`). `suffix_fragment` is the
    /// fragment after `WHERE e.file_path ` in the Cypher query (e.g.
    /// `ENDS WITH '/Cargo.toml'`). Returns a list of distinct
    /// `(file_path, repo_name)` pairs that match.
    async fn find_files_by_suffix(
        &self,
        suffix_fragment: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value>;
}

impl QueryExt for GraphDb {
    /// Fetch entities by UUIDs along with their dependencies (outgoing CALLS relationships).
    async fn get_entities_with_dependencies(
        &self,
        uuids: &[String],
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        if uuids.is_empty() {
            return Ok(serde_json::json!([]));
        }

        let mut results = Vec::new();

        for uuid in uuids {
            let query_str = if repo_name.is_some() {
                "MATCH (m:Entity) WHERE m.uuid = $uuid AND m.repo_name = $repo_name
                 OPTIONAL MATCH (m)-[:CALLS]->(dep:Entity)
                 RETURN m.name, m.kind, m.fqn, m.signature, m.docstring, m.file_path, 
                        m.start_line, collect(dep.name) as dependencies"
                    .to_string()
            } else {
                "MATCH (m:Entity) WHERE m.uuid = $uuid
                 OPTIONAL MATCH (m)-[:CALLS]->(dep:Entity)
                 RETURN m.name, m.kind, m.fqn, m.signature, m.docstring, m.file_path, 
                        m.start_line, collect(dep.name) as dependencies"
                    .to_string()
            };

            let mut q = query(&query_str).param("uuid", uuid.as_str());
            if let Some(repo) = repo_name {
                q = q.param("repo_name", repo);
            }

            let mut row = self
                .graph
                .execute(q)
                .await
                .context("Failed to query Neo4j for entity dependencies")?;

            if let Ok(Some(row_data)) = row.next().await {
                let name = row_data.get::<String>("m.name").ok();
                let kind = row_data.get::<String>("m.kind").ok();
                let fqn = row_data.get::<String>("m.fqn").ok();
                let signature = row_data.get::<String>("m.signature").ok();
                let docstring = row_data.get::<String>("m.docstring").ok();
                let file_path = row_data.get::<String>("m.file_path").ok();
                let start_line = row_data.get::<i64>("m.start_line").ok();
                let dependencies = row_data
                    .get::<Vec<String>>("dependencies")
                    .unwrap_or_default();

                let entity_json = serde_json::json!({
                    "uuid": uuid,
                    "name": name,
                    "kind": kind,
                    "fqn": fqn,
                    "signature": signature,
                    "docstring": docstring,
                    "file_path": file_path,
                    "start_line": start_line,
                    "dependencies": dependencies,
                });

                results.push(entity_json);
            }
        }

        Ok(serde_json::json!(results))
    }

    /// Find all entities that reference a given entity via any relationship type (CALLS, EXTENDS, IMPLEMENTS, REFERENCES).
    /// Returns results grouped by relationship type.
    async fn find_references(
        &self,
        entity_name: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut results = serde_json::json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": []
        });

        // Query for each relationship type
        let rel_types = vec![
            ("CALLS", "calls"),
            ("EXTENDS", "extends"),
            ("IMPLEMENTS", "implements"),
            ("REFERENCES", "references"),
        ];

        for (rel_label, result_key) in rel_types {
            let query_str = if repo_name.is_some() {
                format!(
                    "MATCH (entity:Entity)-[:{rel_label}]->(target:Entity)
                     WHERE target.repo_name = $repo_name
                       AND (target.name = $name
                        OR target.fqn = $name
                        OR target.fqn CONTAINS $name
                        OR (target.name + COALESCE(target.signature, '')) CONTAINS $name)
                     RETURN entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature,
                            target.name as target_name, target.fqn as target_fqn,
                            target.file_path as target_file_path,
                            target.start_line as target_start_line, target.signature as target_signature"
                )
            } else {
                format!(
                    "MATCH (entity:Entity)-[:{rel_label}]->(target:Entity)
                     WHERE target.name = $name
                        OR target.fqn = $name
                        OR target.fqn CONTAINS $name
                        OR (target.name + COALESCE(target.signature, '')) CONTAINS $name
                     RETURN entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature,
                            target.name as target_name, target.fqn as target_fqn,
                            target.file_path as target_file_path,
                            target.start_line as target_start_line, target.signature as target_signature"
                )
            };

            let mut q = query(&query_str).param("name", entity_name);
            if let Some(repo) = repo_name {
                q = q.param("repo_name", repo);
            }

            let mut rows = self.graph.execute(q).await.context(format!(
                "Failed to query Neo4j for {rel_label} relationships"
            ))?;

            let mut type_results = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                let entity_json = serde_json::json!({
                    "name": row.get::<String>("entity.name").ok(),
                    "kind": row.get::<String>("entity.kind").ok(),
                    "file_path": row.get::<String>("entity.file_path").ok(),
                    "start_line": row.get::<i64>("entity.start_line").ok(),
                    "signature": row.get::<String>("entity.signature").ok(),
                    "target_name": row.get::<String>("target_name").ok(),
                    "target_fqn": row.get::<String>("target_fqn").ok(),
                    "target_file_path": row.get::<String>("target_file_path").ok(),
                    "target_start_line": row.get::<i64>("target_start_line").ok(),
                    "target_signature": row.get::<String>("target_signature").ok(),
                });
                type_results.push(entity_json);
            }

            if let Some(arr) = results.get_mut(result_key) {
                *arr = serde_json::json!(type_results);
            }
        }

        Ok(results)
    }

    /// Find all entities that call a given entity (reverse dependency lookup).
    /// **Deprecated:** Use `find_references()` instead for comprehensive relationship tracking.
    async fn find_callers(
        &self,
        entity_name: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        let query_str = if repo_name.is_some() {
            "MATCH (caller:Entity)-[:CALLS]->(callee:Entity)
             WHERE callee.repo_name = $repo_name
               AND (callee.name = $name 
                OR callee.fqn = $name)
             RETURN caller.name, caller.kind, caller.file_path, caller.start_line, caller.signature"
                .to_string()
        } else {
            "MATCH (caller:Entity)-[:CALLS]->(callee:Entity)
             WHERE callee.name = $name 
                OR callee.fqn = $name
             RETURN caller.name, caller.kind, caller.file_path, caller.start_line, caller.signature"
                .to_string()
        };

        let mut q = query(&query_str).param("name", entity_name);
        if let Some(repo) = repo_name {
            q = q.param("repo_name", repo);
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for callers")?;

        while let Ok(Some(row)) = rows.next().await {
            let caller_json = serde_json::json!({
                "name": row.get::<String>("caller.name").ok(),
                "kind": row.get::<String>("caller.kind").ok(),
                "file_path": row.get::<String>("caller.file_path").ok(),
                "start_line": row.get::<i64>("caller.start_line").ok(),
                "signature": row.get::<String>("caller.signature").ok(),
            });
            results.push(caller_json);
        }

        Ok(serde_json::json!(results))
    }

    /// Get all entities within a specific file.
    async fn get_file_entities(
        &self,
        file_path: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        let query_str = if repo_name.is_some() {
            "MATCH (e:Entity {file_path: $file_path, repo_name: $repo_name})
             RETURN e.name, e.kind, e.signature, e.docstring, e.start_line, e.decorators
             ORDER BY e.start_line"
                .to_string()
        } else {
            "MATCH (e:Entity {file_path: $file_path})
             RETURN e.name, e.kind, e.signature, e.docstring, e.start_line, e.decorators
             ORDER BY e.start_line"
                .to_string()
        };

        let mut q = query(&query_str).param("file_path", file_path);
        if let Some(repo) = repo_name {
            q = q.param("repo_name", repo);
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for file entities")?;

        while let Ok(Some(row)) = rows.next().await {
            let decorators = row.get::<Vec<String>>("e.decorators").unwrap_or_default();

            let entity_json = serde_json::json!({
                "name": row.get::<String>("e.name").ok(),
                "kind": row.get::<String>("e.kind").ok(),
                "signature": row.get::<String>("e.signature").ok(),
                "docstring": row.get::<String>("e.docstring").ok(),
                "start_line": row.get::<i64>("e.start_line").ok(),
                "decorators": decorators,
            });
            results.push(entity_json);
        }

        Ok(serde_json::json!(results))
    }

    async fn find_entities_by_name_prefix(
        &self,
        prefix: &str,
        repo_name: Option<&str>,
        limit: usize,
    ) -> Result<serde_json::Value> {
        let query_str = if repo_name.is_some() {
            "MATCH (m:Entity)
             WHERE toLower(m.name) STARTS WITH toLower($prefix) AND m.repo_name = $repo_name
             OPTIONAL MATCH (m)-[:CALLS]->(dep:Entity)
             RETURN m.uuid AS uuid, m.name, m.kind, m.fqn, m.signature, m.docstring,
                    m.file_path, m.start_line, collect(dep.name) as dependencies
             ORDER BY CASE WHEN toLower(m.name) = toLower($prefix) THEN 0 ELSE 1 END,
                      size(m.name),
                      m.fqn,
                      m.uuid
             LIMIT $limit"
                .to_string()
        } else {
            "MATCH (m:Entity)
             WHERE toLower(m.name) STARTS WITH toLower($prefix)
             OPTIONAL MATCH (m)-[:CALLS]->(dep:Entity)
             RETURN m.uuid AS uuid, m.name, m.kind, m.fqn, m.signature, m.docstring,
                    m.file_path, m.start_line, collect(dep.name) as dependencies
             ORDER BY CASE WHEN toLower(m.name) = toLower($prefix) THEN 0 ELSE 1 END,
                      size(m.name),
                      m.fqn,
                      m.uuid
             LIMIT $limit"
                .to_string()
        };

        let mut q = query(&query_str)
            .param("prefix", prefix)
            .param("limit", limit as i64);
        if let Some(repo) = repo_name {
            q = q.param("repo_name", repo);
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for entities by name prefix")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let entity_json = serde_json::json!({
                "uuid": row.get::<String>("uuid").ok(),
                "name": row.get::<String>("m.name").ok(),
                "kind": row.get::<String>("m.kind").ok(),
                "fqn": row.get::<String>("m.fqn").ok(),
                "signature": row.get::<String>("m.signature").ok(),
                "docstring": row.get::<String>("m.docstring").ok(),
                "file_path": row.get::<String>("m.file_path").ok(),
                "start_line": row.get::<i64>("m.start_line").ok(),
                "dependencies": row.get::<Vec<String>>("dependencies").unwrap_or_default(),
            });
            results.push(entity_json);
        }

        Ok(serde_json::json!(results))
    }

    async fn get_file_outgoing_references(
        &self,
        file_path: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        let query_str = if repo_name.is_some() {
            "MATCH (src:Entity {file_path: $file_path, repo_name: $repo_name})
                  -[r:REFERENCES|CALLS|EXTENDS|IMPLEMENTS]->
                  (dst:Entity)
             WHERE dst.file_path <> $file_path OR dst.repo_name <> $repo_name
             RETURN type(r) AS rel,
                    dst.name AS name,
                    dst.kind AS kind,
                    dst.file_path AS file_path,
                    dst.start_line AS line
             ORDER BY rel, name"
                .to_string()
        } else {
            "MATCH (src:Entity {file_path: $file_path})
                  -[r:REFERENCES|CALLS|EXTENDS|IMPLEMENTS]->
                  (dst:Entity)
             WHERE dst.file_path <> $file_path
             RETURN type(r) AS rel,
                    dst.name AS name,
                    dst.kind AS kind,
                    dst.file_path AS file_path,
                    dst.start_line AS line
             ORDER BY rel, name"
                .to_string()
        };

        let mut q = query(&query_str).param("file_path", file_path);
        if let Some(repo) = repo_name {
            q = q.param("repo_name", repo);
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for file outgoing references")?;

        while let Ok(Some(row)) = rows.next().await {
            let entry = serde_json::json!({
                "rel": row.get::<String>("rel").ok(),
                "name": row.get::<String>("name").ok(),
                "kind": row.get::<String>("kind").ok(),
                "file_path": row.get::<String>("file_path").ok(),
                "line": row.get::<i64>("line").ok(),
            });
            results.push(entry);
        }

        Ok(serde_json::json!(results))
    }

    async fn find_files_by_suffix(
        &self,
        suffix_fragment: &str,
        repo_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        // `suffix_fragment` is the post-`WHERE` text, e.g.
        // "ENDS WITH '/src/lib.rs'". We hardcode the rest of the WHERE so
        // callers cannot inject arbitrary Cypher; the fragment is built by
        // `ends_with_suffix_query` which only ever interpolates a string
        // literal, so SQL/Cypher injection is not possible here.
        let query_str = if repo_name.is_some() {
            format!(
                "MATCH (e:Entity) \
                 WHERE e.file_path {suffix_fragment} AND e.repo_name = $repo_name \
                 RETURN DISTINCT e.file_path AS file_path, e.repo_name AS repo_name \
                 ORDER BY e.file_path LIMIT 50"
            )
        } else {
            format!(
                "MATCH (e:Entity) \
                 WHERE e.file_path {suffix_fragment} \
                 RETURN DISTINCT e.file_path AS file_path, e.repo_name AS repo_name \
                 ORDER BY e.file_path LIMIT 50"
            )
        };
        let mut q = query(&query_str);
        if let Some(repo) = repo_name {
            q = q.param("repo_name", repo);
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for files by suffix")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            results.push(serde_json::json!({
                "file_path": row.get::<String>("file_path").ok(),
                "repo_name": row.get::<String>("repo_name").ok(),
            }));
        }
        Ok(serde_json::json!(results))
    }
}

#[cfg(test)]
mod tests {
    use super::super::GraphDb;
    use super::QueryExt;
    use crate::db::graph::connection::ConnectExt;

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entities_with_dependencies_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.get_entities_with_dependencies(&[], None).await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entities_with_dependencies() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let uuids = vec!["550e8400-e29b-41d4-a716-446655440000".to_string()];
        let result = graph_db
            .get_entities_with_dependencies(&uuids, Some("test-repo"))
            .await;
        // Should not fail even if UUID doesn't exist
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_references() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.find_references("nonexistent_entity", None).await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_object());
        assert!(json.get("calls").is_some());
        assert!(json.get("extends").is_some());
        assert!(json.get("implements").is_some());
        assert!(json.get("references").is_some());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_references_with_repo() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .find_references("nonexistent_entity", Some("test-repo"))
            .await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_callers() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.find_callers("nonexistent_entity", None).await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_array());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_callers_with_repo() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .find_callers("nonexistent_entity", Some("test-repo"))
            .await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_file_entities() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_file_entities("/test/path/File.java", None)
            .await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_array());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_file_entities_with_repo() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_file_entities("/test/path/File.java", Some("test-repo"))
            .await;
        assert!(result.is_ok());
    }
}
