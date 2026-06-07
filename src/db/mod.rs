//! Database module.
//!
//! Exposes two sub-modules:
//! - [`vector`]: Qdrant client wrapper (vector store).
//! - [`graph`]: neo4rs client wrapper (graph store).

pub mod graph;
pub mod vector;

use crate::config::Config;
use anyhow::Result;

/// Initialize database connections and perform pre-flight checks.
///
/// Connects to Qdrant and Neo4j, creates the Qdrant collection if it doesn't exist,
/// and ensures Neo4j indexes are present.
pub async fn init_databases(cfg: &Config) -> Result<(vector::VectorDb, graph::GraphDb)> {
    use graph::ConnectExt;
    use vector::VectorConnectExt;

    let vector_db =
        vector::VectorDb::connect(&cfg.qdrant_url, &cfg.qdrant_collection, cfg.embed_dim).await?;
    vector_db.ensure_collection().await?;

    let graph_db =
        graph::GraphDb::connect(&cfg.neo4j_uri, &cfg.neo4j_user, &cfg.neo4j_password).await?;
    graph_db.ensure_indexes().await?;

    Ok((vector_db, graph_db))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_db_exports() {
        // Verify that GraphDb is exported and accessible
        let _ = std::any::type_name::<graph::GraphDb>();
        // Traits are verified by their implementations, not by type_name
    }

    #[test]
    fn test_vector_db_exports() {
        // Verify that VectorDb is exported and accessible
        let _ = std::any::type_name::<vector::VectorDb>();
        // Traits are verified by their implementations, not by type_name
    }

    #[test]
    fn test_module_structure() {
        // Verify module structure is correct
        assert!(std::any::type_name::<graph::GraphDb>().contains("knot::db::graph::GraphDb"));
        assert!(std::any::type_name::<vector::VectorDb>().contains("knot::db::vector::VectorDb"));
    }
}
