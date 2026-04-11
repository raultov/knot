//! Stage 5 — Ingest: dual-write to Qdrant (vector) and Neo4j (graph).
//!
//! Both writes are issued concurrently via `tokio::try_join!` to avoid
//! bottlenecking on either database. The two stores are kept in sync
//! through the shared UUID that every entity carries.

mod batch;
mod resolve;

pub use batch::ingest_batch;
pub use resolve::{
    resolve_and_save_relationships, resolve_reference_intents,
    resolve_reference_intents_with_context,
};
