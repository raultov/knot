use anyhow::Context;
use anyhow::Result;
use qdrant_client::Payload;
use qdrant_client::qdrant::{PointStruct, UpsertPointsBuilder};
use tracing::info;

use super::{VectorDb, utils};
use crate::models::EmbeddedEntity;

/// Extension trait for upsert and write operations.
#[allow(async_fn_in_trait)]
pub trait VectorUpsertExt {
    async fn upsert(&self, entities: &[EmbeddedEntity]) -> Result<()>;
}

impl VectorUpsertExt for VectorDb {
    /// Upsert a batch of [`EmbeddedEntity`] records into Qdrant.
    async fn upsert(&self, entities: &[EmbeddedEntity]) -> Result<()> {
        if entities.is_empty() {
            return Ok(());
        }

        let points: Vec<PointStruct> = entities
            .iter()
            .map(|e| {
                let mut payload = Payload::new();
                payload.insert("uuid", e.entity.uuid.to_string());
                payload.insert("name", e.entity.name.clone());
                payload.insert("kind", e.entity.kind.to_string());
                payload.insert("language", e.entity.language.clone());
                payload.insert("repo_name", e.entity.repo_name.clone());
                payload.insert("file_path", e.entity.file_path.clone());
                payload.insert("start_line", e.entity.start_line as i64);
                if let Some(sig) = &e.entity.signature {
                    payload.insert("signature", sig.clone());
                }
                if let Some(doc) = &e.entity.docstring {
                    payload.insert("docstring", doc.clone());
                }

                // Fold 128-bit UUID into a 64-bit Qdrant point ID via XOR.
                let id = utils::uuid_to_point_id(e.entity.uuid);
                PointStruct::new(id, e.vector.clone(), payload)
            })
            .collect();

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection, points))
            .await
            .context("Failed to upsert points into Qdrant")?;

        info!("Upserted {} vectors into Qdrant", entities.len());
        Ok(())
    }
}
