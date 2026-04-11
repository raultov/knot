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
