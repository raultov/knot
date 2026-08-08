//! Core get_entity_subgraph logic shared between CLI and MCP.
//!
//! Traverses the entity graph starting from a root entity and returns
//! all reachable nodes and edges within the specified depth, filtered
//! by relationship types and direction.

use std::sync::Arc;

use crate::db::graph::{GraphDb, SubgraphQueryExt};
use crate::models::{SubgraphDirection, SubgraphQueryOptions, SubgraphResult};

pub const DEFAULT_MAX_NODES: usize = 500;

/// Bundled parameters for [`run_get_subgraph`].
///
/// Bundling the inputs into a single struct keeps the function signature
/// within clippy's `too_many_arguments` threshold while preserving the
/// documented parameter set.
#[derive(Debug, Clone)]
pub struct SubgraphQueryParams<'a> {
    pub entity_name: &'a str,
    pub repo_name: &'a str,
    pub depth: u32,
    pub relationships: &'a [&'a str],
    pub direction: SubgraphDirection,
    pub max_nodes: Option<usize>,
    pub entity_uuid: Option<&'a str>,
    pub visible_kinds: Option<&'a [&'a str]>,
}

/// Main get_entity_subgraph function called by both CLI and MCP.
///
/// Returns a subgraph centered on the named entity within the given repository,
/// traversing the specified relationships up to the given depth. When `visible_kinds`
/// is provided, only nodes of those kinds are returned, but traversal walks through
/// ALL intermediate nodes so that class-to-class paths connected through methods
/// are preserved via synthetic roll-up edges.
pub async fn run_get_subgraph(
    params: SubgraphQueryParams<'_>,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<SubgraphResult> {
    let max_nodes = params.max_nodes.unwrap_or(DEFAULT_MAX_NODES);
    let result = graph_db
        .get_entity_subgraph(SubgraphQueryOptions {
            entity_name: params.entity_name,
            repo_name: params.repo_name,
            depth: params.depth,
            relationships: params.relationships,
            direction: params.direction,
            max_nodes,
            entity_uuid: params.entity_uuid,
            visible_kinds: params.visible_kinds,
        })
        .await?;
    Ok(result)
}
