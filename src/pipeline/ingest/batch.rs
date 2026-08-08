use anyhow::Result;
use tracing::info;

use crate::{
    db::{
        graph::{GraphDb, UpsertExt},
        vector::{VectorDb, VectorUpsertExt},
    },
    models::EmbeddedEntity,
};

/// Write a batch of [`EmbeddedEntity`] records to both databases simultaneously.
/// NOTE: This only creates the nodes. Relationship edges must be created in a separate
/// pass after ALL nodes have been upserted, to prevent missing-callee failures.
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub async fn ingest_batch(
    entities: &[EmbeddedEntity],
    vector_db: &VectorDb,
    graph_db: &GraphDb,
) -> Result<()> {
    if entities.is_empty() {
        return Ok(());
    }

    info!(
        "[{}] Ingesting batch of {} entities…",
        entities[0].entity.repo_name,
        entities.len()
    );

    // Fire both writes concurrently; surface the first failure.
    tokio::try_join!(
        vector_db.upsert(entities),
        graph_db.upsert_entities(entities),
    )?;

    info!(
        "[{}] Batch ingestion complete",
        entities[0].entity.repo_name
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::graph::ConnectExt;
    use crate::db::vector::VectorConnectExt;

    #[ignore = "requires local Neo4j and Qdrant instances"]
    #[tokio::test]
    async fn test_ingest_batch_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");
        let vector_db = VectorDb::connect("http://localhost:6334", "test_collection", 384)
            .await
            .expect("Failed to connect to Qdrant");

        let result = ingest_batch(&[], &vector_db, &graph_db).await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j and Qdrant instances"]
    #[tokio::test]
    async fn test_ingest_batch_with_entities() {
        use crate::db::vector::test_utils::create_embedded_entity;
        use crate::models::EntityKind;

        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");
        let vector_db = VectorDb::connect("http://localhost:6334", "test_collection", 384)
            .await
            .expect("Failed to connect to Qdrant");

        let entities = vec![
            create_embedded_entity("BatchTest1", EntityKind::Class, 0.1),
            create_embedded_entity("BatchTest2", EntityKind::Method, 0.2),
        ];

        let result = ingest_batch(&entities, &vector_db, &graph_db).await;
        assert!(result.is_ok());
    }
}
