use anyhow::{Context, Result};
use neo4rs::query;
use std::collections::HashMap;

use super::GraphDb;
use super::query::ResolvedSubgraphRoot;
use crate::models::{
    RootCandidateLite, RootResolution, SubgraphDirection, SubgraphEdge, SubgraphNode,
    SubgraphQueryOptions, SubgraphResult,
};

/// Maximum number of candidates surfaced in the `root_resolution.candidates`
/// disclosure payload. Keeps the JSON response bounded — the ranking total
/// is reported separately via `total_candidates`.
const ROOT_RESOLUTION_CANDIDATES_CAP: usize = 10;

/// Extension trait for subgraph traversal operations.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait SubgraphQueryExt {
    async fn get_entity_subgraph(
        &self,
        options: SubgraphQueryOptions<'_>,
    ) -> Result<SubgraphResult>;
}

impl SubgraphQueryExt for GraphDb {
    #[expect(clippy::too_many_lines, reason = "Query generation logic is complex")]
    #[expect(
        clippy::cognitive_complexity,
        reason = "Query generation logic is complex"
    )]
    async fn get_entity_subgraph(
        &self,
        options: SubgraphQueryOptions<'_>,
    ) -> Result<SubgraphResult> {
        let valid_rels: &[&str] = &[
            "CALLS",
            "EXTENDS",
            "IMPLEMENTS",
            "REFERENCES",
            "REFERENCES_DOM",
            "USES_CSS_CLASS",
            "IMPORTS_SCRIPT",
            "IMPORTS_STYLESHEET",
            "MACRO_CALLS",
            "CONTAINS",
            "GENERIC_BOUND",
            "DEPENDS_ON",
            "OVERRIDES",
        ];

        for rel in options.relationships {
            if !valid_rels.contains(rel) {
                anyhow::bail!(
                    "Invalid relationship type '{rel}'. Valid types are: {}",
                    valid_rels.join(", ")
                );
            }
        }

        let depth = options.depth.clamp(1, 5);

        // --- 1. Find the root entity ---
        //
        // Two paths:
        // - `entity_uuid` set → deterministic UUID lookup (no ladder).
        // - `entity_name` only → walk the resolution ladder
        //   (`target_resolution_tiers`), apply `rank_root_candidates` to the
        //   winning tier, anchor the traversal on the chosen UUID. Disclosure
        //   (`root_resolution`) is filled when the name path resolves.
        let mut root_node: Option<SubgraphNode> = None;
        let mut root_resolution: Option<RootResolution> = None;

        if let Some(uuid) = options.entity_uuid {
            let mut rows = self
                .graph
                .execute(
                    query(
                        "MATCH (root:Entity {uuid: $uuid, repo_name: $repo_name})
                         RETURN root.uuid, root.name, root.kind, root.fqn,
                                root.signature, root.docstring, root.file_path, root.start_line
                         LIMIT 1",
                    )
                    .param("uuid", uuid)
                    .param("repo_name", options.repo_name),
                )
                .await
                .context("Failed to query root entity by uuid")?;
            if let Ok(Some(row)) = rows.next().await {
                root_node = Some(SubgraphNode {
                    uuid: row
                        .get::<String>("root.uuid")
                        .context("root.uuid is required")?,
                    name: row
                        .get::<String>("root.name")
                        .context("root.name is required")?,
                    kind: row.get::<String>("root.kind").ok(),
                    fqn: row.get::<String>("root.fqn").ok(),
                    signature: row.get::<String>("root.signature").ok(),
                    docstring: row.get::<String>("root.docstring").ok(),
                    file_path: row.get::<String>("root.file_path").ok(),
                    start_line: row.get::<i64>("root.start_line").ok(),
                });
            }
        } else {
            // Name resolution: walk the ladder, rank, build disclosure.
            match self
                .resolve_subgraph_root(options.entity_name, options.repo_name)
                .await
                .context("Failed to resolve subgraph root by name")?
            {
                Some(ResolvedSubgraphRoot {
                    winner,
                    tier,
                    total_candidates,
                    ranked,
                }) => {
                    let disclosure_candidates: Vec<RootCandidateLite> = ranked
                        .iter()
                        .take(ROOT_RESOLUTION_CANDIDATES_CAP)
                        .cloned()
                        .collect();
                    root_node = Some(SubgraphNode {
                        uuid: winner.uuid.clone(),
                        name: winner.name.clone(),
                        kind: winner.kind.clone(),
                        fqn: winner.fqn.clone(),
                        signature: winner.signature.clone(),
                        docstring: winner.docstring.clone(),
                        file_path: winner.file_path.clone(),
                        start_line: winner.start_line,
                    });
                    root_resolution = Some(RootResolution {
                        query: options.entity_name.to_string(),
                        tier,
                        total_candidates,
                        chosen: winner,
                        candidates: disclosure_candidates,
                    });
                }
                None => {
                    // Root not found — return empty result
                    return Ok(SubgraphResult {
                        root_id: None,
                        nodes: vec![],
                        edges: vec![],
                        truncated: false,
                        total_nodes_found: 0,
                        root_resolution: None,
                    });
                }
            }
        }

        let root_node = match root_node {
            Some(n) => n,
            None => {
                // entity_uuid branch yielded no row.
                return Ok(SubgraphResult {
                    root_id: None,
                    nodes: vec![],
                    edges: vec![],
                    truncated: false,
                    total_nodes_found: 0,
                    root_resolution: None,
                });
            }
        };

        let root_uuid = root_node.uuid.clone();

        // --- 2. Collect nearby nodes ---
        //
        // Traversal is anchored on the resolved UUID only — never on the bare
        // name. This fixes the homonym-union defect (the prior implementation
        // re-bound every homonym by name, returning the union of their
        // neighborhoods under a single `root_id`).
        let mut all_nodes: HashMap<String, SubgraphNode> = HashMap::new();
        all_nodes.insert(root_uuid.clone(), root_node);

        let mut traversal_rels = options.relationships.to_vec();
        if let Some(kinds) = options.visible_kinds
            && !kinds.is_empty()
            && !traversal_rels.contains(&"CONTAINS")
        {
            traversal_rels.push("CONTAINS");
        }
        let traversal_rel_filter = traversal_rels.join("|");

        let direction_arrow = match options.direction {
            SubgraphDirection::Outgoing => format!("-[:{traversal_rel_filter}*1..{depth}]->"),
            SubgraphDirection::Incoming => format!("<-[:{traversal_rel_filter}*1..{depth}]-"),
            SubgraphDirection::Both => format!("-[:{traversal_rel_filter}*1..{depth}]-"),
        };

        let kind_filter = if let Some(kinds) = options.visible_kinds {
            if !kinds.is_empty() {
                let quoted: Vec<String> = kinds.iter().map(|k| format!("'{}'", k)).collect();
                format!("\n   AND related.kind IN [{}]", quoted.join(", "))
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let cypher = format!(
            "MATCH (root:Entity {{uuid: $root_uuid, repo_name: $repo_name}}){arrow}(related:Entity)
             WHERE related.repo_name = $repo_name{kind_filter}
             RETURN DISTINCT related.uuid, related.name, related.kind, related.fqn,
                    related.signature, related.docstring, related.file_path, related.start_line",
            arrow = direction_arrow,
            kind_filter = kind_filter,
        );

        let traversal_q = query(&cypher)
            .param("root_uuid", root_uuid.clone())
            .param("repo_name", options.repo_name);
        let mut rows = self
            .graph
            .execute(traversal_q)
            .await
            .context("Failed to traverse relationships")?;

        while let Ok(Some(row)) = rows.next().await {
            let uuid = match row.get::<String>("related.uuid") {
                Ok(u) => u,
                Err(_) => continue,
            };
            all_nodes.entry(uuid.clone()).or_insert(SubgraphNode {
                uuid,
                name: row.get::<String>("related.name").ok().unwrap_or_default(),
                kind: row.get::<String>("related.kind").ok(),
                fqn: row.get::<String>("related.fqn").ok(),
                signature: row.get::<String>("related.signature").ok(),
                docstring: row.get::<String>("related.docstring").ok(),
                file_path: row.get::<String>("related.file_path").ok(),
                start_line: row.get::<i64>("related.start_line").ok(),
            });
        }

        let total_nodes_found = all_nodes.len();
        let truncated = total_nodes_found > options.max_nodes;

        let mut nodes: Vec<SubgraphNode> = all_nodes.into_values().collect();
        // Deterministic truncation: sort by uuid before truncating so the
        // retained subset is stable across calls.
        nodes.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        if truncated && nodes.len() > options.max_nodes {
            nodes.truncate(options.max_nodes);
        }

        // --- 3. Extract edges between collected nodes ---
        let mut edges: Vec<SubgraphEdge> = Vec::new();

        if nodes.len() > 1 {
            let uuids_list: Vec<String> = nodes.iter().map(|n| format!("'{}'", n.uuid)).collect();
            let uuids_str = uuids_list.join(", ");

            let edge_q = if let Some(kinds) = options.visible_kinds {
                if !kinds.is_empty() {
                    let visible_list: Vec<String> =
                        kinds.iter().map(|k| format!("'{}'", k)).collect();
                    let visible_kind_list = visible_list.join(", ");

                    // Ensure CONTAINS is included in the output filter if we are in kind-aware mode
                    // to maintain structural connectivity (e.g. Inner Classes).
                    let mut edge_rels = options.relationships.to_vec();
                    if !edge_rels.contains(&"CONTAINS") {
                        edge_rels.push("CONTAINS");
                    }
                    let rel_filter = edge_rels.join("|");

                    let edge_cypher = format!(
                        "MATCH (a:Entity)-[r:{rel_filter}]->(b:Entity)
                         WHERE a.uuid IN [{uuids_str}] AND b.uuid IN [{uuids_str}]
                         RETURN DISTINCT a.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship
                         UNION
                         MATCH (c1:Entity)-[:CONTAINS]->(m1:Entity)-[r:{rel_filter}]->(m2:Entity)<-[:CONTAINS]-(c2:Entity)
                         WHERE c1.uuid IN [{uuids_str}] AND c2.uuid IN [{uuids_str}] AND c1.uuid <> c2.uuid
                           AND NOT m1.kind IN [{visible_kind_list}]
                           AND NOT m2.kind IN [{visible_kind_list}]
                         RETURN DISTINCT c1.uuid AS source_uuid, c2.uuid AS target_uuid, type(r) AS relationship
                         UNION
                         MATCH (c1:Entity)-[:CONTAINS]->(m1:Entity)-[r:{rel_filter}]->(b:Entity)
                         WHERE c1.uuid IN [{uuids_str}] AND b.uuid IN [{uuids_str}] AND c1.uuid <> b.uuid
                           AND NOT m1.kind IN [{visible_kind_list}]
                           AND b.kind IN [{visible_kind_list}]
                         RETURN DISTINCT c1.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship
                         UNION
                         MATCH (a:Entity)-[r:{rel_filter}]->(m2:Entity)<-[:CONTAINS]-(c2:Entity)
                         WHERE a.uuid IN [{uuids_str}] AND c2.uuid IN [{uuids_str}] AND a.uuid <> c2.uuid
                           AND a.kind IN [{visible_kind_list}]
                           AND NOT m2.kind IN [{visible_kind_list}]
                         RETURN DISTINCT a.uuid AS source_uuid, c2.uuid AS target_uuid, type(r) AS relationship",
                        rel_filter = rel_filter,
                        visible_kind_list = visible_kind_list,
                        uuids_str = uuids_str,
                    );
                    query(&edge_cypher)
                } else {
                    query(&format!(
                        "MATCH (a:Entity)-[r]->(b:Entity)
                         WHERE a.uuid IN [{uuids_str}] AND b.uuid IN [{uuids_str}]
                         RETURN a.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship",
                        uuids_str = uuids_str,
                    ))
                }
            } else {
                query(&format!(
                    "MATCH (a:Entity)-[r]->(b:Entity)
                     WHERE a.uuid IN [{uuids_str}] AND b.uuid IN [{uuids_str}]
                     RETURN a.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship",
                    uuids_str = uuids_str,
                ))
            };

            let mut rows = self
                .graph
                .execute(edge_q)
                .await
                .context("Failed to query subgraph edges")?;

            while let Ok(Some(row)) = rows.next().await {
                if let (Ok(source_uuid), Ok(target_uuid), Ok(relationship)) = (
                    row.get::<String>("source_uuid"),
                    row.get::<String>("target_uuid"),
                    row.get::<String>("relationship"),
                ) {
                    edges.push(SubgraphEdge {
                        source_uuid,
                        target_uuid,
                        relationship,
                    });
                }
            }
        }

        Ok(SubgraphResult {
            root_id: Some(root_uuid),
            nodes,
            edges,
            truncated,
            total_nodes_found,
            root_resolution,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::graph::connection::ConnectExt;

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_not_found() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "nonexistent_entity",
                repo_name: "test-repo",
                depth: 3,
                relationships: &["CALLS"],
                direction: SubgraphDirection::Both,
                max_nodes: 500,
                entity_uuid: None,
                visible_kinds: None,
            })
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        assert!(sub.nodes.is_empty());
        assert!(sub.edges.is_empty());
        assert!(!sub.truncated);
        assert_eq!(sub.total_nodes_found, 0);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_valid_entity() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "TestClass",
                repo_name: "test-repo",
                depth: 2,
                relationships: &["CALLS", "EXTENDS"],
                direction: SubgraphDirection::Both,
                max_nodes: 500,
                entity_uuid: None,
                visible_kinds: None,
            })
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        assert!(!sub.truncated);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_invalid_relationship() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "TestClass",
                repo_name: "test-repo",
                depth: 2,
                relationships: &["INVALID_REL"],
                direction: SubgraphDirection::Both,
                max_nodes: 500,
                entity_uuid: None,
                visible_kinds: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_outgoing_only() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "TestClass",
                repo_name: "test-repo",
                depth: 1,
                relationships: &["CALLS"],
                direction: SubgraphDirection::Outgoing,
                max_nodes: 500,
                entity_uuid: None,
                visible_kinds: None,
            })
            .await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_multiple_relationships() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "TestClass",
                repo_name: "test-repo",
                depth: 2,
                relationships: &["CALLS", "EXTENDS", "IMPLEMENTS", "REFERENCES"],
                direction: SubgraphDirection::Both,
                max_nodes: 500,
                entity_uuid: None,
                visible_kinds: None,
            })
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        assert!(!sub.truncated || sub.total_nodes_found > 0);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_truncation() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "TestClass",
                repo_name: "test-repo",
                depth: 5,
                relationships: &["CALLS", "EXTENDS", "IMPLEMENTS", "REFERENCES"],
                direction: SubgraphDirection::Both,
                max_nodes: 2,
                entity_uuid: None,
                visible_kinds: None,
            })
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        if sub.total_nodes_found > 2 {
            assert!(sub.truncated);
            assert!(sub.nodes.len() <= 2);
        }
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_with_visible_kinds() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "TestClass",
                repo_name: "test-repo",
                depth: 3,
                relationships: &["CALLS", "EXTENDS", "IMPLEMENTS"],
                direction: SubgraphDirection::Both,
                max_nodes: 500,
                entity_uuid: None,
                visible_kinds: Some(&["class", "interface"]),
            })
            .await;
        assert!(result.is_ok());
        let sub = result.unwrap();
        if !sub.nodes.is_empty() {
            for node in &sub.nodes {
                let kind = node.kind.as_deref().unwrap_or("");
                assert!(
                    kind == "class" || kind == "interface",
                    "Expected node kind in ['class', 'interface'], got '{kind}'"
                );
            }
        }
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entity_subgraph_connectivity() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_entity_subgraph(SubgraphQueryOptions {
                entity_name: "ClassA",
                repo_name: "test-repo",
                depth: 3,
                relationships: &["CALLS"],
                direction: SubgraphDirection::Outgoing,
                max_nodes: 500,
                entity_uuid: None,
                visible_kinds: Some(&["class", "rust_struct"]),
            })
            .await;
        assert!(result.is_ok());
    }
}
