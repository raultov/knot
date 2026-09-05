use serde::{Deserialize, Serialize};

use crate::db::graph::MatchTier;

#[cfg(test)]
mod tests {
    //! §5.6 disclosure payload tests — additive `root_resolution` field.
    use super::*;

    #[test]
    fn test_subgraph_result_serialises_root_resolution_when_present() {
        let chosen = RootCandidateLite {
            uuid: "u1".to_string(),
            name: "UserService".to_string(),
            fqn: Some("MyApp.Services.UserService".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("Services/UserService.cs".to_string()),
            start_line: Some(12),
        };
        let result = SubgraphResult {
            root_id: Some("u1".to_string()),
            nodes: vec![],
            edges: vec![],
            truncated: false,
            total_nodes_found: 0,
            root_resolution: Some(RootResolution {
                query: "UserService".to_string(),
                tier: MatchTier::ExactName,
                total_candidates: 2,
                chosen: chosen.clone(),
                candidates: vec![chosen],
            }),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"root_resolution\""), "got: {json}");
        assert!(json.contains("\"tier\":\"exact_name\""), "got: {json}");
        assert!(json.contains("\"total_candidates\":2"), "got: {json}");
        assert!(json.contains("\"query\":\"UserService\""), "got: {json}");
    }

    #[test]
    fn test_subgraph_result_omits_root_resolution_when_none() {
        // `skip_serializing_if = "Option::is_none"` must drop the key from
        // the JSON output.
        let result = SubgraphResult {
            root_id: Some("u1".to_string()),
            nodes: vec![],
            edges: vec![],
            truncated: false,
            total_nodes_found: 0,
            root_resolution: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("root_resolution"),
            "root_resolution should be omitted when None, got: {json}"
        );
    }

    #[test]
    fn test_subgraph_result_deserializes_without_root_resolution() {
        // Older payloads (no `root_resolution` key) must still deserialize.
        let legacy_json = r#"{
            "root_id": "u1",
            "nodes": [],
            "edges": [],
            "truncated": false,
            "total_nodes_found": 0
        }"#;
        let result: SubgraphResult = serde_json::from_str(legacy_json).unwrap();
        assert!(result.root_resolution.is_none());
        assert_eq!(result.root_id.as_deref(), Some("u1"));
    }
}

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

/// Options for querying a subgraph.
#[derive(Debug, Clone)]
pub struct SubgraphQueryOptions<'a> {
    pub entity_name: &'a str,
    pub repo_name: &'a str,
    pub depth: u32,
    pub relationships: &'a [&'a str],
    pub direction: SubgraphDirection,
    pub max_nodes: usize,
    pub entity_uuid: Option<&'a str>,
    pub visible_kinds: Option<&'a [&'a str]>,
}

/// How the root was resolved when the subgraph was queried by name.
///
/// Additive payload — `serde(default)` keeps older payloads deserializable;
/// knot-server is not required to surface it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootResolution {
    /// The original query string.
    pub query: String,
    /// Which tier of the resolution ladder produced the chosen root.
    pub tier: MatchTier,
    /// Number of candidates the winning tier yielded before ranking
    /// (capped at `MAX_TARGETS` for fairness).
    pub total_candidates: usize,
    /// The candidate chosen as the root.
    pub chosen: RootCandidateLite,
    /// All candidates the winning tier yielded, in rank order
    /// (capped at 10 for payload size).
    pub candidates: Vec<RootCandidateLite>,
}

/// Alias of [`crate::db::graph::RootCandidate`] — a single source of truth
/// for the root-candidate shape shared by the db projection and this wire
/// payload. The db struct and this alias serialize identically, so the wire
/// format is unchanged.
pub type RootCandidateLite = crate::db::graph::RootCandidate;

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
    /// How the root was resolved when queried by name. `None` when the subgraph
    /// was queried by UUID or no root was found. Additive: `serde(default)` keeps
    /// older payloads deserializable; knot-server is not required to surface it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_resolution: Option<RootResolution>,
}
