//! Database module.
//!
//! Exposes two sub-modules:
//! - [`vector`]: Qdrant client wrapper (vector store).
//! - [`graph`]: neo4rs client wrapper (graph store).

pub mod graph;
pub mod vector;

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
