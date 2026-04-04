//! Qdrant vector database client wrapper.
//!
//! Responsibilities:
//! - Create (or verify) the target collection.
//! - Delete all existing vectors associated with a repository path before
//!   a full re-index (prevents orphan vectors).
//! - Batch-insert [`EmbeddedEntity`] records as Qdrant points.

use anyhow::{Context, Result};
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, PointStruct,
        UpsertPointsBuilder, VectorParamsBuilder,
    },
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::models::EmbeddedEntity;

/// Thin wrapper around the Qdrant async client.
pub struct VectorDb {
    client: Qdrant,
    collection: String,
    embed_dim: u64,
}

impl VectorDb {
    /// Connect to Qdrant and return a ready-to-use [`VectorDb`].
    pub async fn connect(url: &str, collection: &str, embed_dim: u64) -> Result<Self> {
        let client = Qdrant::from_url(url)
            .build()
            .context("Failed to build Qdrant client")?;

        Ok(Self {
            client,
            collection: collection.to_owned(),
            embed_dim,
        })
    }

    /// Ensure the collection exists; create it with cosine distance if not.
    pub async fn ensure_collection(&self) -> Result<()> {
        let exists = self
            .client
            .collection_exists(&self.collection)
            .await
            .context("Failed to check collection existence")?;

        if exists {
            info!("Qdrant collection '{}' already exists", self.collection);
            return Ok(());
        }

        info!(
            "Creating Qdrant collection '{}' (dim={}, distance=Cosine)",
            self.collection, self.embed_dim
        );

        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection)
                    .vectors_config(VectorParamsBuilder::new(self.embed_dim, Distance::Cosine)),
            )
            .await
            .context("Failed to create Qdrant collection")?;

        Ok(())
    }

    /// Delete all points in the collection whose `file_path` payload field
    /// contains `repo_path`. Called before a full re-index to avoid orphans.
    pub async fn delete_by_repo(&self, repo_path: &str) -> Result<()> {
        warn!(
            "Deleting existing vectors for repo '{}' from collection '{}'",
            repo_path, self.collection
        );

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection).points(Filter::must([
                    Condition::matches_text("file_path", repo_path),
                ])),
            )
            .await
            .context("Failed to delete existing vectors")?;

        Ok(())
    }

    /// Upsert a batch of [`EmbeddedEntity`] records into Qdrant.
    pub async fn upsert(&self, entities: &[EmbeddedEntity]) -> Result<()> {
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
                payload.insert("file_path", e.entity.file_path.clone());
                payload.insert("start_line", e.entity.start_line as i64);
                if let Some(sig) = &e.entity.signature {
                    payload.insert("signature", sig.clone());
                }
                if let Some(doc) = &e.entity.docstring {
                    payload.insert("docstring", doc.clone());
                }

                // Fold 128-bit UUID into a 64-bit Qdrant point ID via XOR.
                let id = uuid_to_point_id(e.entity.uuid);
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

    /// Search for similar vectors in Qdrant.
    ///
    /// Returns the top N matching points with their payloads (metadata).
    pub async fn search(&self, vector: &[f32], limit: usize) -> Result<Vec<serde_json::Value>> {
        // Build search request directly
        let search_request = qdrant_client::qdrant::SearchPoints {
            collection_name: self.collection.clone(),
            vector: vector.to_vec(),
            limit: limit as u64,
            with_payload: Some(qdrant_client::qdrant::WithPayloadSelector {
                selector_options: Some(
                    qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
                ),
            }),
            ..Default::default()
        };

        let search_result = self
            .client
            .search_points(search_request)
            .await
            .context("Failed to search Qdrant")?;

        let results = search_result
            .result
            .into_iter()
            .filter_map(|scored_point| {
                if !scored_point.payload.is_empty() {
                    let mut json_obj = serde_json::json!({});
                    for (key, value) in scored_point.payload {
                        json_obj[&key] = qdrant_value_to_json(&value);
                    }
                    Some(json_obj)
                } else {
                    None
                }
            })
            .collect();

        Ok(results)
    }
}

/// Fold a 128-bit UUID into a 64-bit Qdrant point ID via XOR.
///
/// Collision probability for typical codebase sizes is negligible.
fn uuid_to_point_id(uuid: Uuid) -> u64 {
    let bytes = uuid.as_u128();
    let hi = (bytes >> 64) as u64;
    let lo = bytes as u64;
    hi ^ lo
}

/// Convert a Qdrant payload value to JSON.
fn qdrant_value_to_json(value: &qdrant_client::qdrant::Value) -> serde_json::Value {
    use qdrant_client::qdrant::value::Kind;

    match &value.kind {
        Some(Kind::StringValue(s)) => serde_json::json!(s),
        Some(Kind::IntegerValue(i)) => serde_json::json!(i),
        Some(Kind::DoubleValue(d)) => serde_json::json!(d),
        Some(Kind::BoolValue(b)) => serde_json::json!(b),
        Some(Kind::ListValue(list)) => {
            let values = list
                .values
                .iter()
                .map(qdrant_value_to_json)
                .collect::<Vec<_>>();
            serde_json::json!(values)
        }
        Some(Kind::NullValue(_)) => serde_json::json!(null),
        Some(Kind::StructValue(_)) => serde_json::json!(null),
        None => serde_json::json!(null),
    }
}
