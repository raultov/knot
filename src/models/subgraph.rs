use serde::{Deserialize, Serialize};

/// Direction for subgraph traversal from the root entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubgraphDirection {
    /// Follow outgoing edges from the root.
    Outgoing,
    /// Follow incoming edges into the root.
    Incoming,
    /// Follow both outgoing and incoming edges.
    #[default]
    Both,
}

/// A node in the entity subgraph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphNode {
    pub uuid: String,
    pub name: String,
    pub kind: Option<String>,
    pub fqn: Option<String>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub file_path: Option<String>,
    pub start_line: Option<i64>,
}

/// An edge connecting two nodes in the subgraph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphEdge {
    pub source_uuid: String,
    pub target_uuid: String,
    pub relationship: String,
}

/// Result of a subgraph traversal query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphResult {
    pub root_id: Option<String>,
    pub nodes: Vec<SubgraphNode>,
    pub edges: Vec<SubgraphEdge>,
    /// True when the total number of nodes found exceeds the requested limit.
    pub truncated: bool,
    /// Total nodes discovered during traversal (before truncation).
    pub total_nodes_found: usize,
}
