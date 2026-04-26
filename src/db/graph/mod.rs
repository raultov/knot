//! Neo4j graph database client wrapper (via neo4rs).
//!
//! Responsibilities:
//! - Open and maintain an async Bolt connection pool to Neo4j.
//! - Delete all nodes/relationships associated with a repository path before
//!   a full re-index (prevents orphan nodes).
//! - Create entity nodes (:Method, :Class, :Interface, :Function).
//! - Create CALLS relationships between resolved entities.

mod connection;
mod delete;
mod query;
mod upsert;
mod utils;

use neo4rs::Graph;

/// Thin wrapper around the neo4rs async connection pool.
pub struct GraphDb {
    pub(crate) graph: Graph,
}

// Re-export traits so they're available with GraphDb imports
pub use connection::ConnectExt;
pub use delete::DeleteExt;
pub use query::QueryExt;
pub use upsert::UpsertExt;

#[cfg(test)]
pub(crate) mod test_utils {
    use crate::models::{EmbeddedEntity, EntityKind, ParsedEntity};
    use uuid::Uuid;

    /// Helper function to create a test entity with realistic data
    pub fn create_test_entity(name: &str, kind: EntityKind, uuid: Uuid) -> ParsedEntity {
        ParsedEntity {
            uuid,
            name: name.to_string(),
            kind,
            fqn: format!("com.example.test.{}", name),
            signature: Some("public void test()".to_string()),
            docstring: Some("Test entity for unit testing".to_string()),
            inline_comments: vec!["Test comment".to_string()],
            decorators: vec!["@Test".to_string()],
            language: "java".to_string(),
            file_path: "/test/path/Test.java".to_string(),
            start_line: 10,
            end_line: 20,
            enclosing_class: Some("TestClass".to_string()),
            repo_name: "test-repo".to_string(),
            reference_intents: vec![],
            calls: vec![],
            relationships: vec![],
            embed_text: "test entity content".to_string(),
            rust_attributes: None,
            impl_trait: None,
            impl_target: None,
            generics: None,
            lifetimes: None,
        }
    }

    /// Helper function to create an embedded test entity
    pub fn create_embedded_test_entity(name: &str, kind: EntityKind) -> EmbeddedEntity {
        let uuid = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, name.as_bytes());
        EmbeddedEntity {
            entity: create_test_entity(name, kind, uuid),
            vector: vec![0.1; 384], // 384-dimensional vector with all 0.1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use crate::models::EntityKind;
    use uuid::Uuid;

    #[test]
    fn test_entity_kind_display() {
        let kind = EntityKind::Class;
        assert_eq!(kind.to_string(), "class");

        let kind = EntityKind::Method;
        assert_eq!(kind.to_string(), "method");

        let kind = EntityKind::Function;
        assert_eq!(kind.to_string(), "function");
    }

    #[test]
    fn test_create_test_entity() {
        let uuid = Uuid::new_v4();
        let entity = create_test_entity("TestMethod", EntityKind::Method, uuid);

        assert_eq!(entity.name, "TestMethod");
        assert_eq!(entity.kind, EntityKind::Method);
        assert_eq!(entity.uuid, uuid);
        assert_eq!(entity.language, "java");
        assert_eq!(entity.repo_name, "test-repo");
    }

    #[test]
    fn test_create_embedded_test_entity() {
        let embedded = create_embedded_test_entity("TestClass", EntityKind::Class);

        assert_eq!(embedded.entity.name, "TestClass");
        assert_eq!(embedded.entity.kind, EntityKind::Class);
        assert_eq!(embedded.vector.len(), 384);
        assert!(embedded.vector.iter().all(|&v| (v - 0.1).abs() < 1e-6));
    }

    #[test]
    fn test_multiple_test_entities() {
        let entity1 = create_embedded_test_entity("Service1", EntityKind::Class);
        let entity2 = create_embedded_test_entity("Service2", EntityKind::Class);

        assert_ne!(entity1.entity.uuid, entity2.entity.uuid);
        assert_ne!(entity1.entity.fqn, entity2.entity.fqn);
    }

    #[test]
    fn test_entity_with_relationships() {
        let uuid1 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"entity1");
        let uuid2 = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, b"entity2");

        let mut entity = create_test_entity("Caller", EntityKind::Method, uuid1);
        entity
            .relationships
            .push((uuid2, crate::models::RelationshipType::Calls));

        assert_eq!(entity.relationships.len(), 1);
        assert_eq!(entity.relationships[0].0, uuid2);
        assert_eq!(
            entity.relationships[0].1,
            crate::models::RelationshipType::Calls
        );
    }
}
