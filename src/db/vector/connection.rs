use anyhow::{Context, Result};
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, Distance, FieldType,
    VectorParamsBuilder,
};
use tracing::info;

use super::VectorDb;

/// Probe the real stored vector dimension of a Qdrant collection.
///
/// `None` ⇒ the collection does not exist (a fresh deployment — the guard
/// ladder never blocks on it). Shared by every knot binary's startup guard
/// (`crate::startup_guard`); knot-server should call this instead of
/// keeping its own `vector_guard` copy.
pub async fn probe_collection_dim(url: &str, collection: &str) -> Result<Option<u64>> {
    let client = Qdrant::from_url(url)
        .timeout(std::time::Duration::from_secs(300))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to build Qdrant client for the collection-dimension probe")?;

    let exists = client
        .collection_exists(collection)
        .await
        .context("Failed to check collection existence during the dimension probe")?;
    if !exists {
        return Ok(None);
    }

    let info = client
        .collection_info(collection)
        .await
        .context("Failed to query collection info during the dimension probe")?;

    let dim = info
        .result
        .as_ref()
        .and_then(|i| i.config.as_ref())
        .and_then(|c| c.params.as_ref())
        .and_then(|p| p.vectors_config.as_ref())
        .and_then(|vc| match vc.config {
            Some(qdrant_client::qdrant::vectors_config::Config::Params(ref p)) => Some(p.size),
            _ => None,
        });
    Ok(dim)
}

/// Extension trait for connection and initialization operations.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait VectorConnectExt {
    async fn connect(url: &str, collection: &str, embed_dim: u64) -> Result<Self>
    where
        Self: Sized;
    async fn ensure_collection(&self) -> Result<()>;
}

impl VectorConnectExt for VectorDb {
    /// Connect to Qdrant and return a ready-to-use [`VectorDb`].
    async fn connect(url: &str, collection: &str, embed_dim: u64) -> Result<VectorDb> {
        let client = Qdrant::from_url(url)
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
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

#[cfg(test)]
mod tests {
    use super::super::VectorDb;
    use super::VectorConnectExt;

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_vector_db_connection() {
        // This test requires a running Qdrant instance
        // Run with: cargo test -- --ignored --test-threads=1
        let result = VectorDb::connect("http://localhost:6334", "test_collection", 384).await;
        assert!(result.is_ok(), "Should be able to connect to Qdrant");
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_vector_db_ensure_collection() {
        let vector_db = VectorDb::connect("http://localhost:6334", "test_collection_ensure", 384)
            .await
            .expect("Failed to connect to Qdrant");

        let result = vector_db.ensure_collection().await;
        assert!(result.is_ok(), "Should be able to ensure collection exists");
    }

    #[ignore = "requires local Qdrant instance running on http://localhost:6334"]
    #[tokio::test]
    async fn test_vector_db_ensure_collection_idempotent() {
        let vector_db =
            VectorDb::connect("http://localhost:6334", "test_collection_idempotent", 384)
                .await
                .expect("Failed to connect to Qdrant");

        let result1 = vector_db.ensure_collection().await;
        assert!(result1.is_ok());

        // Second call should also succeed (idempotent)
        let result2 = vector_db.ensure_collection().await;
        assert!(result2.is_ok());
    }
}
