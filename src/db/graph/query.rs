use anyhow::{Context, Result};
use neo4rs::query;
use std::collections::HashMap;
use tracing::info;

use super::GraphDb;
use crate::models::SubgraphDirection;

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
    async fn find_repo_dependencies(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>>;
    async fn find_repo_dependents(&self, repo_name: &str) -> Result<Vec<String>>;
    async fn find_repository_by_artifact(
        &self,
        group_id: &str,
        artifact_id: &str,
        build_system: &str,
    ) -> Result<Option<String>>;
    #[allow(clippy::too_many_arguments)]
    async fn get_entity_subgraph(
        &self,
        entity_name: &str,
        repo_name: &str,
        depth: u32,
        relationships: &[&str],
        direction: SubgraphDirection,
        max_nodes: usize,
        entity_uuid: Option<&str>,
        visible_kinds: Option<&[&str]>,
    ) -> Result<crate::models::SubgraphResult>;
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
                            target.name as target_name, target.file_path as target_file_path,
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
                            target.name as target_name, target.file_path as target_file_path,
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

    /// Find all repositories that this repo depends on (transitive, up to max_depth).
    async fn find_repo_dependencies(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>> {
        let mut dependencies = Vec::new();

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

        while let Ok(Some(row)) = rows.next().await {
            if let Ok(dep_name) = row.get::<String>("dep_name") {
                dependencies.push(dep_name);
            }
        }

        info!(
            "Found {} repository dependencies for '{repo_name}' (depth {max_depth})",
            dependencies.len()
        );
        Ok(dependencies)
    }

    /// Find all repositories that depend on this repo (reverse lookup).
    async fn find_repo_dependents(&self, repo_name: &str) -> Result<Vec<String>> {
        let mut dependents = Vec::new();

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

        while let Ok(Some(row)) = rows.next().await {
            if let Ok(dep_name) = row.get::<String>("dep_name") {
                dependents.push(dep_name);
            }
        }

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

    #[allow(clippy::too_many_arguments)]
    async fn get_entity_subgraph(
        &self,
        entity_name: &str,
        repo_name: &str,
        depth: u32,
        relationships: &[&str],
        direction: SubgraphDirection,
        max_nodes: usize,
        entity_uuid: Option<&str>,
        visible_kinds: Option<&[&str]>,
    ) -> Result<crate::models::SubgraphResult> {
        use crate::models::{SubgraphEdge, SubgraphNode, SubgraphResult};

        let valid_rels: &[&str] = &[
            "CALLS",
            "EXTENDS",
            "IMPLEMENTS",
            "REFERENCES",
            "REFERENCES_DOM",
            "USES_CSS_CLASS",
            "IMPORTS_SCRIPT",
            "IMPORTS_STYLESHEET",
            "MACRO_CALLS",
            "CONTAINS",
            "GENERIC_BOUND",
            "DEPENDS_ON",
        ];

        for rel in relationships {
            if !valid_rels.contains(rel) {
                anyhow::bail!(
                    "Invalid relationship type '{rel}'. Valid types are: {}",
                    valid_rels.join(", ")
                );
            }
        }

        let depth = depth.clamp(1, 5);

        // --- 1. Find the root entity ---
        let (root_q, root_match_clause) = if let Some(uuid) = entity_uuid {
            let q = query(
                "MATCH (root:Entity {uuid: $uuid, repo_name: $repo_name})
                 RETURN root.uuid, root.name, root.kind, root.fqn,
                        root.signature, root.docstring, root.file_path, root.start_line
                 LIMIT 1",
            )
            .param("uuid", uuid)
            .param("repo_name", repo_name);
            (q, "uuid: $uuid".to_string())
        } else {
            let q = query(
                "MATCH (root:Entity {name: $name, repo_name: $repo_name})
                 RETURN root.uuid, root.name, root.kind, root.fqn,
                        root.signature, root.docstring, root.file_path, root.start_line
                 LIMIT 1",
            )
            .param("name", entity_name)
            .param("repo_name", repo_name);
            (q, "name: $name".to_string())
        };

        let mut rows = self
            .graph
            .execute(root_q)
            .await
            .context("Failed to query root entity")?;

        let root_node = if let Ok(Some(row)) = rows.next().await {
            SubgraphNode {
                uuid: row
                    .get::<String>("root.uuid")
                    .context("root.uuid is required")?,
                name: row
                    .get::<String>("root.name")
                    .context("root.name is required")?,
                kind: row.get::<String>("root.kind").ok(),
                fqn: row.get::<String>("root.fqn").ok(),
                signature: row.get::<String>("root.signature").ok(),
                docstring: row.get::<String>("root.docstring").ok(),
                file_path: row.get::<String>("root.file_path").ok(),
                start_line: row.get::<i64>("root.start_line").ok(),
            }
        } else {
            // Root not found — return empty result
            return Ok(SubgraphResult {
                root_id: None,
                nodes: vec![],
                edges: vec![],
                truncated: false,
                total_nodes_found: 0,
            });
        };

        let root_uuid = root_node.uuid.clone();

        // --- 2. Collect nearby nodes ---
        let mut all_nodes: HashMap<String, SubgraphNode> = HashMap::new();
        all_nodes.insert(root_uuid.clone(), root_node);

        let rel_filter = relationships.join("|");
        let direction_arrow = match direction {
            SubgraphDirection::Outgoing => format!("-[:{rel_filter}*1..{depth}]->"),
            SubgraphDirection::Incoming => format!("<-[:{rel_filter}*1..{depth}]-"),
            SubgraphDirection::Both => format!("-[:{rel_filter}*1..{depth}]-"),
        };

        let kind_filter = if let Some(kinds) = visible_kinds
            && !kinds.is_empty()
        {
            let quoted: Vec<String> = kinds.iter().map(|k| format!("'{}'", k)).collect();
            format!("\n   AND related.kind IN [{}]", quoted.join(", "))
        } else {
            String::new()
        };

        let cypher = format!(
            "MATCH (root:Entity {{{root_match}, repo_name: $repo_name}}){arrow}(related:Entity)
             WHERE related.repo_name = $repo_name{kind_filter}
             RETURN DISTINCT related.uuid, related.name, related.kind, related.fqn,
                    related.signature, related.docstring, related.file_path, related.start_line",
            root_match = root_match_clause,
            arrow = direction_arrow,
            kind_filter = kind_filter,
        );

        let traversal_q = if let Some(uuid) = entity_uuid {
            query(&cypher)
                .param("uuid", uuid)
                .param("repo_name", repo_name)
        } else {
            query(&cypher)
                .param("name", entity_name)
                .param("repo_name", repo_name)
        };
        let mut rows = self
            .graph
            .execute(traversal_q)
            .await
            .context("Failed to traverse relationships")?;

        while let Ok(Some(row)) = rows.next().await {
            let uuid = match row.get::<String>("related.uuid") {
                Ok(u) => u,
                Err(_) => continue,
            };
            all_nodes.entry(uuid.clone()).or_insert(SubgraphNode {
                uuid,
                name: row.get::<String>("related.name").ok().unwrap_or_default(),
                kind: row.get::<String>("related.kind").ok(),
                fqn: row.get::<String>("related.fqn").ok(),
                signature: row.get::<String>("related.signature").ok(),
                docstring: row.get::<String>("related.docstring").ok(),
                file_path: row.get::<String>("related.file_path").ok(),
                start_line: row.get::<i64>("related.start_line").ok(),
            });
        }

        let total_nodes_found = all_nodes.len();
        let truncated = total_nodes_found > max_nodes;

        let mut nodes: Vec<SubgraphNode> = all_nodes.into_values().collect();
        if truncated && nodes.len() > max_nodes {
            nodes.truncate(max_nodes);
        }

        // --- 3. Extract edges between collected nodes ---
        let mut edges: Vec<SubgraphEdge> = Vec::new();

        if nodes.len() > 1 {
            let uuids: Vec<String> = nodes.iter().map(|n| n.uuid.clone()).collect();

            let edge_q = if let Some(kinds) = visible_kinds
                && !kinds.is_empty()
            {
                let visible_list: Vec<String> = kinds.iter().map(|k| format!("'{}'", k)).collect();
                let visible_kind_list = visible_list.join(", ");
                let rel_filter = relationships.join("|");

                let edge_cypher = format!(
                    "MATCH (a:Entity)-[r]->(b:Entity)
                     WHERE a.uuid IN $uuids AND b.uuid IN $uuids
                     RETURN DISTINCT a.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship
                     UNION
                     MATCH (m1:Entity {{repo_name: $repo_name}})-[r:{rel_filter}]->(m2:Entity {{repo_name: $repo_name}})
                     WHERE NOT m1.kind IN [{visible_kind_list}]
                       AND NOT m2.kind IN [{visible_kind_list}]
                       AND m1.enclosing_class <> '' AND m2.enclosing_class <> ''
                     MATCH (c1:Entity {{name: m1.enclosing_class, repo_name: $repo_name}})
                     MATCH (c2:Entity {{name: m2.enclosing_class, repo_name: $repo_name}})
                     WHERE c1.uuid IN $uuids AND c2.uuid IN $uuids AND c1.uuid <> c2.uuid
                     RETURN DISTINCT c1.uuid AS source_uuid, c2.uuid AS target_uuid, type(r) AS relationship
                     UNION
                     MATCH (m1:Entity {{repo_name: $repo_name}})-[r:{rel_filter}]->(b:Entity {{repo_name: $repo_name}})
                     WHERE NOT m1.kind IN [{visible_kind_list}]
                       AND b.kind IN [{visible_kind_list}]
                       AND m1.enclosing_class <> ''
                     MATCH (c1:Entity {{name: m1.enclosing_class, repo_name: $repo_name}})
                     WHERE c1.uuid IN $uuids AND b.uuid IN $uuids AND c1.uuid <> b.uuid
                     RETURN DISTINCT c1.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship
                     UNION
                     MATCH (a:Entity {{repo_name: $repo_name}})-[r:{rel_filter}]->(m2:Entity {{repo_name: $repo_name}})
                     WHERE a.kind IN [{visible_kind_list}]
                       AND NOT m2.kind IN [{visible_kind_list}]
                       AND m2.enclosing_class <> ''
                     MATCH (c2:Entity {{name: m2.enclosing_class, repo_name: $repo_name}})
                     WHERE a.uuid IN $uuids AND c2.uuid IN $uuids AND a.uuid <> c2.uuid
                     RETURN DISTINCT a.uuid AS source_uuid, c2.uuid AS target_uuid, type(r) AS relationship",
                    rel_filter = rel_filter,
                    visible_kind_list = visible_kind_list,
                );
                query(&edge_cypher)
                    .param("uuids", uuids)
                    .param("repo_name", repo_name)
            } else {
                query(
                    "MATCH (a:Entity)-[r]->(b:Entity)
                     WHERE a.uuid IN $uuids AND b.uuid IN $uuids
                     RETURN a.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship",
                )
                .param("uuids", uuids)
            };

            let mut rows = self
                .graph
                .execute(edge_q)
                .await
                .context("Failed to query subgraph edges")?;

            while let Ok(Some(row)) = rows.next().await {
                if let (Ok(source_uuid), Ok(target_uuid), Ok(relationship)) = (
                    row.get::<String>("source_uuid"),
                    row.get::<String>("target_uuid"),
                    row.get::<String>("relationship"),
                ) {
                    edges.push(SubgraphEdge {
                        source_uuid,
                        target_uuid,
                        relationship,
                    });
                }
            }
        }

        Ok(SubgraphResult {
            root_id: Some(root_uuid),
            nodes,
            edges,
            truncated,
            total_nodes_found,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::GraphDb;
    use super::QueryExt;
    use crate::db::graph::connection::ConnectExt;
    use crate::models::SubgraphDirection;

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

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_not_found() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(
                "nonexistent_entity",
                "test-repo",
                3,
                &["CALLS"],
                SubgraphDirection::Both,
                500,
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        assert!(sub.nodes.is_empty());
        assert!(sub.edges.is_empty());
        assert!(!sub.truncated);
        assert_eq!(sub.total_nodes_found, 0);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_valid_entity() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        // This test expects at least one entity named "TestClass" in the graph.
        // If no such entity exists, the result will be empty which is also valid.
        let result = graph_db
            .get_entity_subgraph(
                "TestClass",
                "test-repo",
                2,
                &["CALLS", "EXTENDS"],
                SubgraphDirection::Both,
                500,
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        // Should not fail regardless of whether the entity exists
        assert!(!sub.truncated);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_invalid_relationship() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(
                "TestClass",
                "test-repo",
                2,
                &["INVALID_REL"],
                SubgraphDirection::Both,
                500,
                None,
                None,
            )
            .await;
        assert!(result.is_err());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_outgoing_only() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(
                "TestClass",
                "test-repo",
                1,
                &["CALLS"],
                SubgraphDirection::Outgoing,
                500,
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_multiple_relationships() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(
                "TestClass",
                "test-repo",
                2,
                &["CALLS", "EXTENDS", "IMPLEMENTS", "REFERENCES"],
                SubgraphDirection::Both,
                500,
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        // With multiple relationship types, result should not be an error
        assert!(!sub.truncated || sub.total_nodes_found > 0);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_truncation() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(
                "TestClass",
                "test-repo",
                5,
                &["CALLS", "EXTENDS", "IMPLEMENTS", "REFERENCES"],
                SubgraphDirection::Both,
                2, // Very low max_nodes to trigger truncation
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        // Nodes should not exceed the max_nodes limit
        if sub.total_nodes_found > 2 {
            assert!(sub.truncated);
            assert!(sub.nodes.len() <= 2);
        }
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_with_visible_kinds() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(
                "TestClass",
                "test-repo",
                3,
                &["CALLS", "EXTENDS", "IMPLEMENTS"],
                SubgraphDirection::Both,
                500,
                None,
                Some(&["class", "interface"]),
            )
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        // All returned nodes should match the visible kinds
        if !sub.nodes.is_empty() {
            for node in &sub.nodes {
                let kind = node.kind.as_deref().unwrap_or("");
                assert!(
                    kind == "class" || kind == "interface",
                    "Expected node kind in ['class', 'interface'], got '{kind}'"
                );
            }
        }
    }
}
