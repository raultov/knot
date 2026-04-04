//! Database module.
//!
//! Exposes two sub-modules:
//! - [`vector`]: Qdrant client wrapper (vector store).
//! - [`graph`]: neo4rs client wrapper (graph store).

pub mod graph;
pub mod vector;
