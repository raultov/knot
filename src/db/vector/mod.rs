//! Qdrant vector database client wrapper.
//!
//! Responsibilities:
//! - Create (or verify) the target collection.
//! - Delete all existing vectors associated with a repository path before
//!   a full re-index (prevents orphan vectors).
//! - Batch-insert [`EmbeddedEntity`] records as Qdrant points.

mod connection;
mod delete;
mod search;
mod upsert;
mod utils;

use qdrant_client::Qdrant;

/// Thin wrapper around the Qdrant async client.
pub struct VectorDb {
    pub(crate) client: Qdrant,
    pub(crate) collection: String,
    pub(crate) embed_dim: u64,
}

// Re-export traits so they're available with VectorDb imports
pub use connection::VectorConnectExt;
pub use delete::VectorDeleteExt;
pub use search::VectorSearchExt;
pub use upsert::VectorUpsertExt;

#[cfg(test)]
pub(crate) mod test_utils {
    use crate::models::{EmbeddedEntity, EntityKind, ParsedEntity};
    use uuid::Uuid;

    /// Helper function to create a test entity
    pub fn create_test_entity(name: &str, kind: EntityKind, uuid: Uuid) -> ParsedEntity {
        ParsedEntity {
            uuid,
            name: name.to_string(),
            kind,
            fqn: format!("com.example.test.{}", name),
            signature: Some("public void test()".to_string()),
            docstring: Some("Test entity".to_string()),
            inline_comments: vec!["comment".to_string()],
            decorators: vec!["@Test".to_string()],
            language: "java".to_string(),
            file_path: "/test/Test.java".to_string(),
            start_line: 10,
            end_line: 20,
            enclosing_class: Some("TestClass".to_string()),
            repo_name: "test-repo".to_string(),
            reference_intents: vec![],
            calls: vec![],
            relationships: vec![],
            embed_text: "test content".to_string(),
            rust_attributes: None,
            impl_trait: None,
            impl_target: None,
            generics: None,
            lifetimes: None,
        }
    }

    /// Helper function to create an embedded entity with a vector
    pub fn create_embedded_entity(
        name: &str,
        kind: EntityKind,
        vector_value: f32,
    ) -> EmbeddedEntity {
        let uuid = Uuid::new_v5(&crate::models::NAMESPACE_KNOT, name.as_bytes());
        EmbeddedEntity {
            entity: create_test_entity(name, kind, uuid),
            vector: vec![vector_value; 384],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use crate::models::EntityKind;

    #[test]
    fn test_create_test_entity() {
        use uuid::Uuid;
        let uuid = Uuid::new_v4();
        let entity = create_test_entity("TestClass", EntityKind::Class, uuid);

        assert_eq!(entity.name, "TestClass");
        assert_eq!(entity.kind, EntityKind::Class);
        assert_eq!(entity.uuid, uuid);
        assert_eq!(entity.repo_name, "test-repo");
    }

    #[test]
    fn test_create_embedded_entity() {
        let embedded = create_embedded_entity("TestMethod", EntityKind::Method, 0.5);

        assert_eq!(embedded.entity.name, "TestMethod");
        assert_eq!(embedded.entity.kind, EntityKind::Method);
        assert_eq!(embedded.vector.len(), 384);
        assert!(embedded.vector.iter().all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn test_multiple_embedded_entities() {
        let entity1 = create_embedded_entity("Entity1", EntityKind::Class, 0.1);
        let entity2 = create_embedded_entity("Entity2", EntityKind::Class, 0.2);
        let entity3 = create_embedded_entity("Entity3", EntityKind::Function, 0.3);

        assert_eq!(entity1.vector.len(), 384);
        assert_eq!(entity2.vector.len(), 384);
        assert_eq!(entity3.vector.len(), 384);

        assert_ne!(entity1.entity.uuid, entity2.entity.uuid);
        assert_ne!(entity2.entity.uuid, entity3.entity.uuid);

        // Verify vector values are different
        assert!((entity1.vector[0] - 0.1).abs() < 1e-6);
        assert!((entity2.vector[0] - 0.2).abs() < 1e-6);
        assert!((entity3.vector[0] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_uuid_consistency() {
        let entity1a = create_embedded_entity("TestEntity", EntityKind::Class, 0.5);
        let entity1b = create_embedded_entity("TestEntity", EntityKind::Class, 0.5);

        // Same name should produce same UUID (deterministic)
        assert_eq!(entity1a.entity.uuid, entity1b.entity.uuid);
    }

    #[test]
    fn test_vector_dimensions() {
        let test_dims = vec![128, 256, 384, 768];

        for dim in test_dims {
            let vector = vec![0.5; dim];
            assert_eq!(vector.len(), dim);
        }
    }
}
