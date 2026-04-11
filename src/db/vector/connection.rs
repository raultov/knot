use anyhow::{Context, Result};
use qdrant_client::qdrant::{
    CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, Distance, FieldType,
    VectorParamsBuilder,
};
use tracing::info;

use super::VectorDb;

/// Extension trait for connection and initialization operations.
#[allow(async_fn_in_trait)]
pub trait VectorConnectExt {
    async fn connect(url: &str, collection: &str, embed_dim: u64) -> Result<Self>
    where
        Self: Sized;
    async fn ensure_collection(&self) -> Result<()>;
}

impl VectorConnectExt for VectorDb {
    /// Connect to Qdrant and return a ready-to-use [`VectorDb`].
    async fn connect(url: &str, collection: &str, embed_dim: u64) -> Result<VectorDb> {
        let client = qdrant_client::Qdrant::from_url(url)
            .build()
            .context("Failed to build Qdrant client")?;

        Ok(VectorDb {
            client,
            collection: collection.to_owned(),
            embed_dim,
        })
    }

    /// Ensure the collection exists; create it with cosine distance if not.
    /// Also ensures a Keyword payload index on 'repo_name' for optimized multi-repo queries.
    async fn ensure_collection(&self) -> Result<()> {
        let exists = self
            .client
            .collection_exists(&self.collection)
            .await
            .context("Failed to check collection existence")?;

        if !exists {
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
        } else {
            info!("Qdrant collection '{}' already exists", self.collection);
        }

        // Ensure Keyword payload index on 'repo_name' for fast multi-repo filtering
        info!(
            "Ensuring Keyword payload index on 'repo_name' for collection '{}'",
            self.collection
        );
        self.client
            .create_field_index(CreateFieldIndexCollectionBuilder::new(
                &self.collection,
                "repo_name",
                FieldType::Keyword,
            ))
            .await
            .context("Failed to create payload index on 'repo_name'")?;

        Ok(())
    }
}
