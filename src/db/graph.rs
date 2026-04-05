//! Neo4j graph database client wrapper (via neo4rs).
//!
//! Responsibilities:
//! - Open and maintain an async Bolt connection pool to Neo4j.
//! - Delete all nodes/relationships associated with a repository path before
//!   a full re-index (prevents orphan nodes).
//! - Create entity nodes (:Method, :Class, :Interface, :Function).
//! - Create CALLS relationships between resolved entities.

use anyhow::{Context, Result};
use neo4rs::{Graph, query};
use tracing::{info, warn};

use crate::models::{EmbeddedEntity, EntityKind};

/// Thin wrapper around the neo4rs async connection pool.
pub struct GraphDb {
    graph: Graph,
}

impl GraphDb {
    /// Connect to Neo4j via Bolt and return a ready-to-use [`GraphDb`].
    pub async fn connect(uri: &str, user: &str, password: &str) -> Result<Self> {
        let graph = Graph::new(uri, user, password).context("Failed to connect to Neo4j")?;

        info!("Connected to Neo4j at {uri}");
        Ok(Self { graph })
    }

    /// Ensure necessary indexes exist for fast lookups by UUID, repo_name, and file_path.
    pub async fn ensure_indexes(&self) -> Result<()> {
        let stmts = [
            // UUID uniqueness constraint (covers Method, Class, Interface, Function)
            "CREATE CONSTRAINT entity_uuid_unique IF NOT EXISTS \
             FOR (e:Entity) REQUIRE e.uuid IS UNIQUE",
            // Index on repo_name for multi-repository isolation and fast filtering
            "CREATE INDEX entity_repo_name IF NOT EXISTS \
             FOR (e:Entity) ON (e.repo_name)",
            // Index on file_path for quick path-based lookups
            "CREATE INDEX entity_file_path IF NOT EXISTS \
             FOR (e:Entity) ON (e.file_path)",
        ];

        for stmt in &stmts {
            self.graph
                .run(query(stmt))
                .await
                .context("Failed to create Neo4j index/constraint")?;
        }

        info!("Neo4j indexes/constraints verified");
        Ok(())
    }

    /// Delete all entity nodes (and their relationships) whose `repo_name`
    /// exactly matches the provided name. Called before a full re-index.
    pub async fn delete_by_repo(&self, repo_name: &str) -> Result<()> {
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

    /// Upsert a batch of entity nodes into Neo4j.
    ///
    /// Uses `MERGE` on `uuid` so re-running the indexer is idempotent.
    pub async fn upsert_entities(&self, entities: &[EmbeddedEntity]) -> Result<()> {
        if entities.is_empty() {
            return Ok(());
        }

        for e in entities {
            let label = kind_to_label(&e.entity.kind);

            // MERGE on the shared :Entity label + uuid, then SET kind-specific label.
            let cypher = format!(
                "MERGE (n:Entity {{uuid: $uuid}})
                 SET n:{label}
                 SET n.name        = $name,
                     n.kind        = $kind,
                     n.language    = $language,
                     n.repo_name   = $repo_name,
                     n.file_path   = $file_path,
                     n.start_line  = $start_line,
                     n.signature   = $signature,
                     n.docstring   = $docstring,
                     n.inline_comments = $inline_comments,
                     n.decorators  = $decorators,
                     n.embed_text  = $embed_text"
            );

            self.graph
                .run(
                    query(&cypher)
                        .param("uuid", e.entity.uuid.to_string())
                        .param("name", e.entity.name.clone())
                        .param("kind", e.entity.kind.to_string())
                        .param("language", e.entity.language.clone())
                        .param("repo_name", e.entity.repo_name.clone())
                        .param("file_path", e.entity.file_path.clone())
                        .param("start_line", e.entity.start_line as i64)
                        .param("signature", e.entity.signature.clone().unwrap_or_default())
                        .param("docstring", e.entity.docstring.clone().unwrap_or_default())
                        .param("inline_comments", e.entity.inline_comments.clone())
                        .param("decorators", e.entity.decorators.clone())
                        .param("embed_text", e.entity.embed_text.clone()),
                )
                .await
                .context("Failed to upsert entity node into Neo4j")?;
        }

        info!("Upserted {} entity nodes into Neo4j", entities.len());
        Ok(())
    }

    /// Create typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES) for all resolved edges.
    pub async fn upsert_relationships(&self, entities: &[EmbeddedEntity]) -> Result<()> {
        let mut edge_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for e in entities {
            for (callee_uuid, rel_type) in &e.entity.relationships {
                let rel_label = rel_type.to_string();
                let cypher = format!(
                    "MATCH (caller:Entity {{uuid: $caller_uuid}})
                     MATCH (callee:Entity {{uuid: $callee_uuid}})
                     MERGE (caller)-[:{rel_label}]->(callee)"
                );

                self.graph
                    .run(
                        query(&cypher)
                            .param("caller_uuid", e.entity.uuid.to_string())
                            .param("callee_uuid", callee_uuid.to_string()),
                    )
                    .await
                    .context(format!(
                        "Failed to create {rel_label} relationship in Neo4j"
                    ))?;

                *edge_counts.entry(rel_label.clone()).or_insert(0) += 1;
            }
        }

        if !edge_counts.is_empty() {
            for (rel_type, count) in edge_counts {
                info!("Created {count} {rel_type} relationships in Neo4j");
            }
        }

        Ok(())
    }

    /// Legacy method for backward compatibility. Creates only CALLS relationships.
    /// New code should use `upsert_relationships()` instead.
    pub async fn upsert_calls(&self, entities: &[EmbeddedEntity]) -> Result<()> {
        let mut edge_count = 0usize;

        for e in entities {
            for callee_uuid in &e.entity.calls {
                self.graph
                    .run(
                        query(
                            "MATCH (caller:Entity {uuid: $caller_uuid})
                             MATCH (callee:Entity {uuid: $callee_uuid})
                             MERGE (caller)-[:CALLS]->(callee)",
                        )
                        .param("caller_uuid", e.entity.uuid.to_string())
                        .param("callee_uuid", callee_uuid.to_string()),
                    )
                    .await
                    .context("Failed to create CALLS relationship in Neo4j")?;

                edge_count += 1;
            }
        }

        if edge_count > 0 {
            info!("Created {edge_count} CALLS relationships in Neo4j");
        }

        Ok(())
    }

    /// Fetch entities by UUIDs along with their dependencies (outgoing CALLS relationships).
    pub async fn get_entities_with_dependencies(
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
    pub async fn find_references(
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
    pub async fn find_callers(
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
    pub async fn get_file_entities(
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

/// Map an [`EntityKind`] to its Neo4j node label string.
fn kind_to_label(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Class => "Class",
        EntityKind::Interface => "Interface",
        EntityKind::Method => "Method",
        EntityKind::Function => "Function",
        EntityKind::Constant => "Constant",
        EntityKind::Enum => "Enum",
    }
}
