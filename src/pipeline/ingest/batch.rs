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
pub async fn ingest_batch(
    entities: &[EmbeddedEntity],
    vector_db: &VectorDb,
    graph_db: &GraphDb,
) -> Result<()> {
    if entities.is_empty() {
        return Ok(());
    }

    info!("Ingesting batch of {} entities…", entities.len());

    // Fire both writes concurrently; surface the first failure.
    tokio::try_join!(
        vector_db.upsert(entities),
        graph_db.upsert_entities(entities),
    )?;

    info!("Batch ingestion complete");
    Ok(())
}
