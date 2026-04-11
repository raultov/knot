use anyhow::{Context, Result};
use neo4rs::query;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use super::{GraphDb, utils};
use crate::models::{EmbeddedEntity, ResolutionEntity};

/// Extension trait for upsert and write operations.
#[allow(async_fn_in_trait)]
pub trait UpsertExt {
    async fn load_entity_mappings(
        &self,
        repo_name: &str,
    ) -> Result<(HashMap<String, Uuid>, HashMap<String, Vec<Uuid>>)>;
    async fn upsert_entities(&self, entities: &[EmbeddedEntity]) -> Result<()>;
    async fn upsert_relationships(&self, entities: &[ResolutionEntity]) -> Result<()>;
    async fn upsert_calls(&self, entities: &[EmbeddedEntity]) -> Result<()>;
}

impl UpsertExt for GraphDb {
    /// Load entity mappings (name, fqn -> uuid) for incremental indexing.
    ///
    /// This is called before resolving reference intents to hydrate the global
    /// context with entities from unchanged files that weren't re-parsed.
    /// Returns two hashmaps for fast lookup during relationship resolution.
    async fn load_entity_mappings(
        &self,
        repo_name: &str,
    ) -> Result<(HashMap<String, Uuid>, HashMap<String, Vec<Uuid>>)> {
        info!(
            "Loading entity mappings from Neo4j for repo '{}'",
            repo_name
        );

        let mut stream = self
            .graph
            .execute(
                query(
                    "MATCH (e:Entity)
                     WHERE e.repo_name = $repo_name
                     RETURN e.name AS name, e.uuid AS uuid_str, 
                            COALESCE(e.fqn, e.name) AS fqn",
                )
                .param("repo_name", repo_name),
            )
            .await
            .context("Failed to query entity mappings from Neo4j")?;

        let mut fqn_to_uuid: HashMap<String, Uuid> = HashMap::new();
        let mut name_to_uuids: HashMap<String, Vec<Uuid>> = HashMap::new();

        while let Some(row) = stream
            .next()
            .await
            .context("Failed to fetch row from Neo4j")?
        {
            let name: String = row.get("name").context("Missing 'name' field")?;
            let uuid_str: String = row.get("uuid_str").context("Missing 'uuid_str' field")?;
            let fqn: String = row.get("fqn").context("Missing 'fqn' field")?;

            let uuid = Uuid::parse_str(&uuid_str)
                .with_context(|| format!("Invalid UUID string: {}", uuid_str))?;

            // Populate fqn -> uuid mapping
            fqn_to_uuid.insert(fqn, uuid);

            // Populate name -> uuids mapping (multiple entities can have the same name)
            name_to_uuids.entry(name).or_default().push(uuid);
        }

        info!(
            "Loaded {} FQN mappings and {} name mappings from Neo4j",
            fqn_to_uuid.len(),
            name_to_uuids.len()
        );

        Ok((fqn_to_uuid, name_to_uuids))
    }

    /// Upsert a batch of entity nodes into Neo4j.
    ///
    /// Uses `MERGE` on `uuid` so re-running the indexer is idempotent.
    async fn upsert_entities(&self, entities: &[EmbeddedEntity]) -> Result<()> {
        if entities.is_empty() {
            return Ok(());
        }

        for e in entities {
            let label = utils::kind_to_label(&e.entity.kind);

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
    async fn upsert_relationships(&self, entities: &[ResolutionEntity]) -> Result<()> {
        let mut edge_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for e in entities {
            for (callee_uuid, rel_type) in &e.relationships {
                let rel_label = rel_type.to_string();
                let cypher = format!(
                    "MATCH (caller:Entity {{uuid: $caller_uuid}})
                     MATCH (callee:Entity {{uuid: $callee_uuid}})
                     MERGE (caller)-[:{rel_label}]->(callee)"
                );

                self.graph
                    .run(
                        query(&cypher)
                            .param("caller_uuid", e.uuid.to_string())
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
    async fn upsert_calls(&self, entities: &[EmbeddedEntity]) -> Result<()> {
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
}

#[cfg(test)]
mod tests {
    use super::super::GraphDb;
    use super::UpsertExt;
    use crate::db::graph::connection::ConnectExt;
    use crate::db::graph::test_utils::create_embedded_test_entity;
    use crate::models::{EntityKind, ResolutionEntity};

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_load_entity_mappings_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.load_entity_mappings("nonexistent-repo").await;
        assert!(result.is_ok());
        let (fqn_map, name_map) = result.unwrap();
        // Both maps should be empty for a nonexistent repo
        assert_eq!(fqn_map.len(), 0);
        assert_eq!(name_map.len(), 0);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_upsert_entities() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let entities = vec![
            create_embedded_test_entity("UpsertTest1", EntityKind::Class),
            create_embedded_test_entity("UpsertTest2", EntityKind::Method),
        ];

        let result = graph_db.upsert_entities(&entities).await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_upsert_entities_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.upsert_entities(&[]).await;
        // Should return Ok immediately without inserting anything
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_upsert_relationships() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let entities = [create_embedded_test_entity("RelTest1", EntityKind::Class)];
        let res_entities: Vec<ResolutionEntity> =
            entities.iter().map(ResolutionEntity::from).collect();

        let result = graph_db.upsert_relationships(&res_entities).await;
        // Should not fail even if relationships are empty
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_upsert_relationships_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.upsert_relationships(&[]).await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_upsert_calls() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let entities = vec![create_embedded_test_entity("CallTest1", EntityKind::Method)];

        let result = graph_db.upsert_calls(&entities).await;
        // Should not fail even if calls list is empty
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_upsert_calls_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.upsert_calls(&[]).await;
        assert!(result.is_ok());
    }
}
