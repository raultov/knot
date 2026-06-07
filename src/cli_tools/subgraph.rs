//! Core get_entity_subgraph logic shared between CLI and MCP.
//!
//! Traverses the entity graph starting from a root entity and returns
//! all reachable nodes and edges within the specified depth, filtered
//! by relationship types and direction.

use std::sync::Arc;

use crate::db::graph::{GraphDb, SubgraphQueryExt};
use crate::models::{SubgraphDirection, SubgraphQueryOptions, SubgraphResult};

pub const DEFAULT_MAX_NODES: usize = 500;

/// Main get_entity_subgraph function called by both CLI and MCP.
///
/// Returns a subgraph centered on the named entity within the given repository,
/// traversing the specified relationships up to the given depth. When `visible_kinds`
/// is provided, only nodes of those kinds are returned, but traversal walks through
/// ALL intermediate nodes so that class-to-class paths connected through methods
/// are preserved via synthetic roll-up edges.
#[allow(clippy::too_many_arguments)]
pub async fn run_get_subgraph(
    entity_name: &str,
    repo_name: &str,
    depth: u32,
    relationships: &[&str],
    direction: SubgraphDirection,
    max_nodes: Option<usize>,
    graph_db: &Arc<GraphDb>,
    entity_uuid: Option<&str>,
    visible_kinds: Option<&[&str]>,
) -> anyhow::Result<SubgraphResult> {
    let max_nodes = max_nodes.unwrap_or(DEFAULT_MAX_NODES);
    let result = graph_db
        .get_entity_subgraph(SubgraphQueryOptions {
            entity_name,
            repo_name,
            depth,
            relationships,
            direction,
            max_nodes,
            entity_uuid,
            visible_kinds,
        })
        .await?;
    Ok(result)
}
