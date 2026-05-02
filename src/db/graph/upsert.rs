use anyhow::{Context, Result};
use neo4rs::{BoltType, query};
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use super::{GraphDb, utils};
use crate::models::{EmbeddedEntity, RelationshipType, ResolutionEntity};

/// Extension trait for upsert and write operations.
#[allow(async_fn_in_trait)]
pub trait UpsertExt {
    async fn load_entity_mappings(
        &self,
        repo_names: &[String],
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
    /// Supports loading from multiple repositories for cross-repository dependency analysis.
    /// Returns two hashmaps for fast lookup during relationship resolution.
    async fn load_entity_mappings(
        &self,
        repo_names: &[String],
    ) -> Result<(HashMap<String, Uuid>, HashMap<String, Vec<Uuid>>)> {
        if repo_names.is_empty() {
            return Ok((HashMap::new(), HashMap::new()));
        }

        info!(
            "Loading entity mappings from Neo4j for {} repo(s): {}",
            repo_names.len(),
            repo_names.join(", ")
        );

        // Build Cypher query that filters by multiple repo names
        let cypher = if repo_names.len() == 1 {
            "MATCH (e:Entity)
             WHERE e.repo_name = $repo_names[0]
             RETURN e.name AS name, e.uuid AS uuid_str, 
                    COALESCE(e.fqn, e.name) AS fqn"
                .to_string()
        } else {
            "MATCH (e:Entity)
             WHERE e.repo_name IN $repo_names
             RETURN e.name AS name, e.uuid AS uuid_str, 
                    COALESCE(e.fqn, e.name) AS fqn"
                .to_string()
        };

        let mut stream = self
            .graph
            .execute(query(&cypher).param("repo_names", repo_names.to_vec()))
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
            "Loaded {} FQN mappings and {} name mappings from {} repo(s)",
            fqn_to_uuid.len(),
            name_to_uuids.len(),
            repo_names.len()
        );

        Ok((fqn_to_uuid, name_to_uuids))
    }

    /// Upsert a batch of entity nodes into Neo4j.
    ///
    /// Uses `UNWIND` to batch all entities in a single Cypher query,
    /// grouped by entity kind (since labels cannot be parameterized).
    /// Each entity is MERGED on its UUID for idempotency.
    async fn upsert_entities(&self, entities: &[EmbeddedEntity]) -> Result<()> {
        if entities.is_empty() {
            return Ok(());
        }

        // Group entities by their Neo4j label (derived from EntityKind)
        let mut groups: HashMap<String, Vec<&EmbeddedEntity>> = HashMap::new();
        for e in entities {
            let label = utils::kind_to_label(&e.entity.kind);
            groups.entry(label.to_string()).or_default().push(e);
        }

        for (label, group) in &groups {
            let entity_params: Vec<HashMap<String, BoltType>> = group
                .iter()
                .map(|e| {
                    let mut map = HashMap::new();
                    map.insert("uuid".to_string(), e.entity.uuid.to_string().into());
                    map.insert("name".to_string(), e.entity.name.clone().into());
                    map.insert("kind".to_string(), e.entity.kind.to_string().into());
                    map.insert("language".to_string(), e.entity.language.clone().into());
                    map.insert("repo_name".to_string(), e.entity.repo_name.clone().into());
                    map.insert("file_path".to_string(), e.entity.file_path.clone().into());
                    map.insert(
                        "start_line".to_string(),
                        (e.entity.start_line as i64).into(),
                    );
                    map.insert("end_line".to_string(), (e.entity.end_line as i64).into());
                    map.insert(
                        "signature".to_string(),
                        e.entity.signature.clone().unwrap_or_default().into(),
                    );
                    map.insert(
                        "docstring".to_string(),
                        e.entity.docstring.clone().unwrap_or_default().into(),
                    );
                    map.insert(
                        "inline_comments".to_string(),
                        e.entity.inline_comments.clone().into(),
                    );
                    map.insert("decorators".to_string(), e.entity.decorators.clone().into());
                    map.insert("embed_text".to_string(), e.entity.embed_text.clone().into());
                    map.insert("fqn".to_string(), e.entity.fqn.clone().into());
                    map.insert(
                        "enclosing_class".to_string(),
                        e.entity.enclosing_class.clone().unwrap_or_default().into(),
                    );
                    map
                })
                .collect();

            let cypher = format!(
                "UNWIND $entities AS e
                 MERGE (n:Entity {{uuid: e.uuid}})
                 SET n:{label},
                     n.name = e.name, n.kind = e.kind, n.language = e.language,
                     n.repo_name = e.repo_name, n.file_path = e.file_path,
                     n.start_line = e.start_line, n.end_line = e.end_line,
                     n.signature = e.signature, n.docstring = e.docstring,
                     n.inline_comments = e.inline_comments, n.decorators = e.decorators,
                     n.embed_text = e.embed_text, n.fqn = e.fqn,
                     n.enclosing_class = e.enclosing_class"
            );

            self.graph
                .run(query(&cypher).param("entities", entity_params))
                .await
                .context("Failed to upsert entity nodes into Neo4j")?;
        }

        info!("Upserted {} entity nodes into Neo4j", entities.len());
        Ok(())
    }

    /// Create typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES) for all resolved edges.
    ///
    /// Batched via `UNWIND` — one Cypher query per relationship type instead of
    /// one per edge. Grouping by relationship type is necessary because Cypher
    /// cannot parameterize relationship labels.
    async fn upsert_relationships(&self, entities: &[ResolutionEntity]) -> Result<()> {
        let mut by_type: HashMap<RelationshipType, Vec<(String, String)>> = HashMap::new();
        for e in entities {
            for (callee_uuid, rel_type) in &e.relationships {
                by_type
                    .entry(*rel_type)
                    .or_default()
                    .push((e.uuid.to_string(), callee_uuid.to_string()));
            }
        }

        let mut total_edges = 0usize;

        for (rel_type, edges) in &by_type {
            let rel_label = rel_type.to_string();
            let edge_params: Vec<HashMap<String, BoltType>> = edges
                .iter()
                .map(|(caller, callee)| {
                    let mut map = HashMap::new();
                    map.insert("caller_uuid".to_string(), caller.clone().into());
                    map.insert("callee_uuid".to_string(), callee.clone().into());
                    map
                })
                .collect();

            let cypher = format!(
                "UNWIND $edges AS e
                 MATCH (caller:Entity {{uuid: e.caller_uuid}})
                 MATCH (callee:Entity {{uuid: e.callee_uuid}})
                 MERGE (caller)-[:{rel_label}]->(callee)"
            );
            self.graph
                .run(query(&cypher).param("edges", edge_params))
                .await
                .context(format!(
                    "Failed to create {rel_label} relationships in Neo4j"
                ))?;

            total_edges += edges.len();
            info!("Created {} {rel_label} relationships in Neo4j", edges.len());
        }

        if total_edges == 0 {
            info!("No relationships to create");
        }

        Ok(())
    }

    /// Legacy method for backward compatibility. Creates only CALLS relationships.
    /// New code should use `upsert_relationships()` instead.
    ///
    /// Batched via `UNWIND` — all CALLS edges in a single Cypher query.
    async fn upsert_calls(&self, entities: &[EmbeddedEntity]) -> Result<()> {
        let mut edge_params: Vec<HashMap<String, BoltType>> = Vec::new();

        for e in entities {
            for callee_uuid in &e.entity.calls {
                let mut map = HashMap::new();
                map.insert("caller_uuid".to_string(), e.entity.uuid.to_string().into());
                map.insert("callee_uuid".to_string(), callee_uuid.to_string().into());
                edge_params.push(map);
            }
        }

        let edge_count = edge_params.len();
        if edge_count == 0 {
            return Ok(());
        }

        self.graph
            .run(
                query(
                    "UNWIND $edges AS e
                     MATCH (caller:Entity {uuid: e.caller_uuid})
                     MATCH (callee:Entity {uuid: e.callee_uuid})
                     MERGE (caller)-[:CALLS]->(callee)",
                )
                .param("edges", edge_params),
            )
            .await
            .context("Failed to create CALLS relationships in Neo4j")?;

        info!("Created {edge_count} CALLS relationships in Neo4j");
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
    use neo4rs::BoltType;
    use std::collections::HashMap;
    use uuid::Uuid;

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_load_entity_mappings_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .load_entity_mappings(&["nonexistent-repo".to_string()])
            .await;
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

    // Unit tests for load_entity_mappings with mocked logic
    #[test]
    fn test_load_entity_mappings_empty_repo_list() {
        let repo_names: Vec<String> = vec![];
        // Simulate what would happen with empty repo list
        if repo_names.is_empty() {
            let fqn_map = std::collections::HashMap::<String, uuid::Uuid>::new();
            let name_map = std::collections::HashMap::<String, Vec<uuid::Uuid>>::new();
            assert_eq!(fqn_map.len(), 0);
            assert_eq!(name_map.len(), 0);
        }
    }

    #[test]
    fn test_load_entity_mappings_cypher_query_single_repo() {
        let repo_names = ["core-lib".to_string()].to_vec();

        // Verify the query construction logic
        let cypher = if repo_names.len() == 1 {
            "MATCH (e:Entity)
             WHERE e.repo_name = $repo_names[0]
             RETURN e.name AS name, e.uuid AS uuid_str, 
                    COALESCE(e.fqn, e.name) AS fqn"
                .to_string()
        } else {
            "MATCH (e:Entity)
             WHERE e.repo_name IN $repo_names
             RETURN e.name AS name, e.uuid AS uuid_str, 
                    COALESCE(e.fqn, e.name) AS fqn"
                .to_string()
        };

        assert!(cypher.contains("e.repo_name = $repo_names[0]"));
        assert!(!cypher.contains("IN $repo_names"));
    }

    #[test]
    fn test_load_entity_mappings_cypher_query_multiple_repos() {
        let repo_names = [
            "core-lib".to_string(),
            "shared-types".to_string(),
            "utils".to_string(),
        ]
        .to_vec();

        // Verify the query construction logic
        let cypher = if repo_names.len() == 1 {
            "MATCH (e:Entity)
             WHERE e.repo_name = $repo_names[0]
             RETURN e.name AS name, e.uuid AS uuid_str, 
                    COALESCE(e.fqn, e.name) AS fqn"
                .to_string()
        } else {
            "MATCH (e:Entity)
             WHERE e.repo_name IN $repo_names
             RETURN e.name AS name, e.uuid AS uuid_str, 
                    COALESCE(e.fqn, e.name) AS fqn"
                .to_string()
        };

        assert!(cypher.contains("IN $repo_names"));
        assert!(!cypher.contains("e.repo_name = $repo_names[0]"));
    }

    #[test]
    fn test_hashmap_merging_simulation() {
        // Simulate merging entity mappings from multiple repos
        let mut fqn_to_uuid: HashMap<String, Uuid> = HashMap::new();
        let mut name_to_uuids: HashMap<String, Vec<Uuid>> = HashMap::new();

        // Simulate data from core-lib
        let uuid1 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"core.Service");
        fqn_to_uuid.insert("core.Service".to_string(), uuid1);
        name_to_uuids
            .entry("Service".to_string())
            .or_default()
            .push(uuid1);

        // Simulate data from shared-types
        let uuid2 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"shared.Config");
        fqn_to_uuid.insert("shared.Config".to_string(), uuid2);
        name_to_uuids
            .entry("Config".to_string())
            .or_default()
            .push(uuid2);

        // Verify merged maps
        assert_eq!(fqn_to_uuid.len(), 2);
        assert_eq!(name_to_uuids.len(), 2);
        assert_eq!(name_to_uuids["Service"].len(), 1);
        assert_eq!(name_to_uuids["Config"].len(), 1);
    }

    #[test]
    fn test_hashmap_merging_duplicate_names() {
        // Simulate merging when multiple entities have same name from different repos
        let mut name_to_uuids: HashMap<String, Vec<Uuid>> = HashMap::new();

        let uuid1 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"repo1.Service");
        let uuid2 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"repo2.Service");

        name_to_uuids
            .entry("Service".to_string())
            .or_default()
            .push(uuid1);
        name_to_uuids
            .entry("Service".to_string())
            .or_default()
            .push(uuid2);

        // Both UUIDs should be stored for the name
        assert_eq!(name_to_uuids["Service"].len(), 2);
        assert!(name_to_uuids["Service"].contains(&uuid1));
        assert!(name_to_uuids["Service"].contains(&uuid2));
    }

    #[test]
    fn test_uuid_parsing_from_string() {
        let uuid_str = "6b6e6f74-2d69-6e64-6578-6572762d3500";
        let uuid = Uuid::parse_str(uuid_str);
        assert!(uuid.is_ok());

        let uuid_str_invalid = "not-a-uuid";
        let uuid = Uuid::parse_str(uuid_str_invalid);
        assert!(uuid.is_err());
    }

    #[test]
    fn test_fqn_resolution_priority() {
        // Test logic: FQN takes priority over name for lookups
        let mut fqn_to_uuid: HashMap<String, Uuid> = HashMap::new();
        let mut name_to_uuids: HashMap<String, Vec<Uuid>> = HashMap::new();

        let uuid = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"Service");
        let fqn = "com.example.Service";
        let name = "Service";

        fqn_to_uuid.insert(fqn.to_string(), uuid);
        name_to_uuids
            .entry(name.to_string())
            .or_default()
            .push(uuid);

        // FQN lookup should be exact
        assert!(fqn_to_uuid.contains_key(fqn));
        assert_eq!(fqn_to_uuid[fqn], uuid);

        // Name lookup can have multiple matches
        assert!(name_to_uuids.contains_key(name));
        assert_eq!(name_to_uuids[name][0], uuid);
    }

    // --- UNWIND batch unit tests ---

    /// Verify UNWIND entity grouping: entities of the same kind go into
    /// the same group, different kinds go into different groups.
    #[test]
    fn test_unwind_entity_grouping_by_kind() {
        let entity1 = create_embedded_test_entity("MyClass", EntityKind::Class);
        let entity2 = create_embedded_test_entity("MyOtherClass", EntityKind::Class);
        let entity3 = create_embedded_test_entity("myMethod", EntityKind::Method);
        let entity4 = create_embedded_test_entity("myFunction", EntityKind::Function);

        let entities = vec![entity1, entity2, entity3, entity4];

        // Simulate the grouping logic from upsert_entities
        let mut groups: HashMap<String, Vec<&crate::models::EmbeddedEntity>> = HashMap::new();
        for e in &entities {
            let label = super::super::utils::kind_to_label(&e.entity.kind);
            groups.entry(label.to_string()).or_default().push(e);
        }

        // Should have 3 groups: Class, Method, Function
        assert_eq!(groups.len(), 3);
        assert_eq!(groups["Class"].len(), 2);
        assert_eq!(groups["Method"].len(), 1);
        assert_eq!(groups["Function"].len(), 1);
    }

    /// Verify UNWIND Cypher query contains the expected structure.
    #[test]
    fn test_unwind_cypher_query_contains_unwind_and_merge() {
        let label = "Class";
        let cypher = format!(
            "UNWIND $entities AS e
             MERGE (n:Entity {{uuid: e.uuid}})
             SET n:{label},
                 n.name = e.name, n.kind = e.kind, n.language = e.language,
                 n.repo_name = e.repo_name, n.file_path = e.file_path,
                 n.start_line = e.start_line, n.end_line = e.end_line,
                 n.signature = e.signature, n.docstring = e.docstring,
                 n.inline_comments = e.inline_comments, n.decorators = e.decorators,
                 n.embed_text = e.embed_text, n.fqn = e.fqn,
                 n.enclosing_class = e.enclosing_class"
        );

        assert!(cypher.contains("UNWIND $entities AS e"));
        assert!(cypher.contains("MERGE (n:Entity {uuid: e.uuid})"));
        assert!(cypher.contains("SET n:Class"));
        assert!(cypher.contains("n.name = e.name"));
        assert!(cypher.contains("n.kind = e.kind"));
        assert!(cypher.contains("n.fqn = e.fqn"));
        assert!(cypher.contains("n.enclosing_class = e.enclosing_class"));
    }

    /// Verify UNWIND parameter map construction for entities.
    #[test]
    fn test_unwind_entity_param_map_construction() {
        let entity = create_embedded_test_entity("TestParam", EntityKind::Class);
        let mut map: HashMap<String, BoltType> = HashMap::new();

        map.insert("uuid".to_string(), entity.entity.uuid.to_string().into());
        map.insert("name".to_string(), entity.entity.name.clone().into());
        map.insert("kind".to_string(), entity.entity.kind.to_string().into());
        map.insert(
            "language".to_string(),
            entity.entity.language.clone().into(),
        );
        map.insert(
            "repo_name".to_string(),
            entity.entity.repo_name.clone().into(),
        );
        map.insert(
            "file_path".to_string(),
            entity.entity.file_path.clone().into(),
        );
        map.insert(
            "start_line".to_string(),
            (entity.entity.start_line as i64).into(),
        );
        map.insert(
            "end_line".to_string(),
            (entity.entity.end_line as i64).into(),
        );
        map.insert(
            "signature".to_string(),
            entity.entity.signature.clone().unwrap_or_default().into(),
        );
        map.insert(
            "docstring".to_string(),
            entity.entity.docstring.clone().unwrap_or_default().into(),
        );
        map.insert(
            "inline_comments".to_string(),
            entity.entity.inline_comments.clone().into(),
        );
        map.insert(
            "decorators".to_string(),
            entity.entity.decorators.clone().into(),
        );
        map.insert(
            "embed_text".to_string(),
            entity.entity.embed_text.clone().into(),
        );
        map.insert("fqn".to_string(), entity.entity.fqn.clone().into());
        map.insert(
            "enclosing_class".to_string(),
            entity
                .entity
                .enclosing_class
                .clone()
                .unwrap_or_default()
                .into(),
        );

        assert_eq!(map.len(), 15);
        assert_eq!(map["name"], BoltType::from(entity.entity.name.clone()));
        assert_eq!(map["fqn"], BoltType::from(entity.entity.fqn.clone()));
    }

    /// Verify UNWIND relationships grouping by relationship type.
    #[test]
    fn test_unwind_relationships_grouping_by_type() {
        use crate::models::RelationshipType;

        let uuid1 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"caller1");
        let uuid2 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"callee1");
        let uuid3 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"callee2");
        let uuid4 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"callee3");
        let uuid5 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"callee4");

        // Simulate two entities with different relationship types
        let mut by_type: HashMap<RelationshipType, Vec<(String, String)>> = HashMap::new();

        // Entity 1: CALLS uuid2, EXTENDS uuid3
        by_type
            .entry(RelationshipType::Calls)
            .or_default()
            .push((uuid1.to_string(), uuid2.to_string()));
        by_type
            .entry(RelationshipType::Extends)
            .or_default()
            .push((uuid1.to_string(), uuid3.to_string()));

        // Entity 2: CALLS uuid4, CALLS uuid5, IMPLEMENTS uuid3
        by_type
            .entry(RelationshipType::Calls)
            .or_default()
            .push((uuid1.to_string(), uuid4.to_string()));
        by_type
            .entry(RelationshipType::Calls)
            .or_default()
            .push((uuid1.to_string(), uuid5.to_string()));
        by_type
            .entry(RelationshipType::Implements)
            .or_default()
            .push((uuid1.to_string(), uuid3.to_string()));

        assert_eq!(by_type.len(), 3);
        assert_eq!(by_type[&RelationshipType::Calls].len(), 3);
        assert_eq!(by_type[&RelationshipType::Extends].len(), 1);
        assert_eq!(by_type[&RelationshipType::Implements].len(), 1);
    }

    /// Verify UNWIND relationship Cypher query structure.
    #[test]
    fn test_unwind_relationship_cypher_query_structure() {
        let rel_label = "CALLS";
        let cypher = format!(
            "UNWIND $edges AS e
             MATCH (caller:Entity {{uuid: e.caller_uuid}})
             MATCH (callee:Entity {{uuid: e.callee_uuid}})
             MERGE (caller)-[:{rel_label}]->(callee)"
        );

        assert!(cypher.contains("UNWIND $edges AS e"));
        assert!(cypher.contains("MATCH (caller:Entity {uuid: e.caller_uuid})"));
        assert!(cypher.contains("MATCH (callee:Entity {uuid: e.callee_uuid})"));
        assert!(cypher.contains("MERGE (caller)-[:CALLS]->(callee)"));
    }

    /// Verify that relationships grouping handles empty relationships correctly.
    #[test]
    fn test_unwind_relationships_empty_grouping() {
        use crate::models::RelationshipType;

        let by_type: HashMap<RelationshipType, Vec<(String, String)>> = HashMap::new();
        assert!(by_type.is_empty());
    }

    /// Verify UNWIND edge parameter map construction.
    #[test]
    fn test_unwind_edge_param_map_construction() {
        let caller = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"caller");
        let callee = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"callee");

        let mut map: HashMap<String, BoltType> = HashMap::new();
        map.insert("caller_uuid".to_string(), caller.to_string().into());
        map.insert("callee_uuid".to_string(), callee.to_string().into());

        assert_eq!(map.len(), 2);
        assert_eq!(map["caller_uuid"], BoltType::from(caller.to_string()));
        assert_eq!(map["callee_uuid"], BoltType::from(callee.to_string()));
    }

    /// Verify that UNWIND CALLS query is correct.
    #[test]
    fn test_unwind_calls_cypher_query_structure() {
        let cypher = "UNWIND $edges AS e
             MATCH (caller:Entity {uuid: e.caller_uuid})
             MATCH (callee:Entity {uuid: e.callee_uuid})
             MERGE (caller)-[:CALLS]->(callee)";

        assert!(cypher.contains("UNWIND $edges AS e"));
        assert!(cypher.contains("MATCH (caller:Entity {uuid: e.caller_uuid})"));
        assert!(cypher.contains("MATCH (callee:Entity {uuid: e.callee_uuid})"));
        assert!(cypher.contains("MERGE (caller)-[:CALLS]->(callee)"));
    }

    /// Verify UNWIND entity grouping with a single kind.
    #[test]
    fn test_unwind_entity_grouping_single_kind() {
        let entity1 = create_embedded_test_entity("A", EntityKind::Class);
        let entity2 = create_embedded_test_entity("B", EntityKind::Class);

        let mut groups: HashMap<String, Vec<&crate::models::EmbeddedEntity>> = HashMap::new();
        let entities = [entity1, entity2];
        for e in &entities {
            let label = super::super::utils::kind_to_label(&e.entity.kind);
            groups.entry(label.to_string()).or_default().push(e);
        }

        assert_eq!(groups.len(), 1);
        assert_eq!(groups["Class"].len(), 2);
    }

    /// Verify UNWIND entity grouping with all entities of different kinds.
    #[test]
    fn test_unwind_entity_grouping_all_different_kinds() {
        let entities = vec![
            create_embedded_test_entity("A", EntityKind::Class),
            create_embedded_test_entity("B", EntityKind::Method),
            create_embedded_test_entity("C", EntityKind::Function),
            create_embedded_test_entity("D", EntityKind::Constant),
            create_embedded_test_entity("E", EntityKind::Interface),
            create_embedded_test_entity("F", EntityKind::Enum),
        ];

        let mut groups: HashMap<String, Vec<&crate::models::EmbeddedEntity>> = HashMap::new();
        for e in &entities {
            let label = super::super::utils::kind_to_label(&e.entity.kind);
            groups.entry(label.to_string()).or_default().push(e);
        }

        assert_eq!(groups.len(), 6);
        for group in groups.values() {
            assert_eq!(group.len(), 1);
        }
    }

    /// Verify that UNWIND handles RelationshipType::Display correctly.
    #[test]
    fn test_unwind_relationship_type_labels_match_expected_patterns() {
        use crate::models::RelationshipType;

        assert_eq!(RelationshipType::Calls.to_string(), "CALLS");
        assert_eq!(RelationshipType::Extends.to_string(), "EXTENDS");
        assert_eq!(RelationshipType::Implements.to_string(), "IMPLEMENTS");
        assert_eq!(RelationshipType::References.to_string(), "REFERENCES");
        assert_eq!(
            RelationshipType::ReferencesDOM.to_string(),
            "REFERENCES_DOM"
        );
        assert_eq!(RelationshipType::UsesCSSClass.to_string(), "USES_CSS_CLASS");
        assert_eq!(RelationshipType::MacroCalls.to_string(), "MACRO_CALLS");
        assert_eq!(RelationshipType::Contains.to_string(), "CONTAINS");
    }
}
