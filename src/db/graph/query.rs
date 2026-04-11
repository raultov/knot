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
                 RETURN m.name, m.kind, m.signature, m.docstring, m.file_path, 
                        m.start_line, collect(dep.name) as dependencies"
                    .to_string()
            } else {
                "MATCH (m:Entity) WHERE m.uuid = $uuid
                 OPTIONAL MATCH (m)-[:CALLS]->(dep:Entity)
                 RETURN m.name, m.kind, m.signature, m.docstring, m.file_path, 
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
                    "MATCH (entity:Entity)-[:{rel_label}]->(target:Entity {{name: $name, repo_name: $repo_name}})
                     RETURN entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature"
                )
            } else {
                format!(
                    "MATCH (entity:Entity)-[:{rel_label}]->(target:Entity {{name: $name}})
                     RETURN entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature"
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
            "MATCH (caller:Entity)-[:CALLS]->(callee:Entity {name: $name, repo_name: $repo_name})
             RETURN caller.name, caller.kind, caller.file_path, caller.start_line, caller.signature"
                .to_string()
        } else {
            "MATCH (caller:Entity)-[:CALLS]->(callee:Entity {name: $name})
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
             RETURN e.name, e.kind, e.signature, e.docstring, e.start_line
             ORDER BY e.start_line"
                .to_string()
        } else {
            "MATCH (e:Entity {file_path: $file_path})
             RETURN e.name, e.kind, e.signature, e.docstring, e.start_line
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
            let entity_json = serde_json::json!({
                "name": row.get::<String>("e.name").ok(),
                "kind": row.get::<String>("e.kind").ok(),
                "signature": row.get::<String>("e.signature").ok(),
                "docstring": row.get::<String>("e.docstring").ok(),
                "start_line": row.get::<i64>("e.start_line").ok(),
            });
            results.push(entity_json);
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
