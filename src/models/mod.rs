//! Core data models shared across all pipeline stages.
//!
//! Every extracted entity receives a deterministic UUID v5 that acts as the primary key
//! bridging Qdrant (vector store) and Neo4j (graph store).
//!
//! UUIDs are deterministic (derived from repo_name + file_path + fqn) to enable
//! incremental indexing without breaking graph relationships.

mod cli_args;
mod entity;
mod relationship;
mod subgraph;

pub use cli_args::{Cli, Commands};
pub use entity::{EmbeddedEntity, EntityKind, NAMESPACE_KNOT, ParsedEntity, ResolutionEntity};
pub use relationship::{CallIntent, ReferenceIntent, RelationshipType};
pub use subgraph::{
    SubgraphDirection, SubgraphEdge, SubgraphNode, SubgraphQueryOptions, SubgraphResult,
};
