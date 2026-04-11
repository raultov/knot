//! Core data models shared across all pipeline stages.
//!
//! Every extracted entity receives a deterministic UUID v5 that acts as the primary key
//! bridging Qdrant (vector store) and Neo4j (graph store).
//!
//! UUIDs are deterministic (derived from repo_name + file_path + fqn) to enable
//! incremental indexing without breaking graph relationships.

mod entity;
mod relationship;

pub use entity::{EmbeddedEntity, EntityKind, NAMESPACE_KNOT, ParsedEntity, ResolutionEntity};
pub use relationship::{CallIntent, ReferenceIntent, RelationshipType};
