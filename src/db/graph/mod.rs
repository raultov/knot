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
