use anyhow::{Context, Result};
use neo4rs::query;
use std::collections::HashMap;

use super::GraphDb;
use crate::models::{
    SubgraphDirection, SubgraphEdge, SubgraphNode, SubgraphQueryOptions, SubgraphResult,
};

/// Extension trait for subgraph traversal operations.
#[allow(async_fn_in_trait)]
pub trait SubgraphQueryExt {
    async fn get_entity_subgraph(
        &self,
        options: SubgraphQueryOptions<'_>,
    ) -> Result<SubgraphResult>;
}

impl SubgraphQueryExt for GraphDb {
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
        let (root_q, root_match_clause) = if let Some(uuid) = options.entity_uuid {
            let q = query(
                "MATCH (root:Entity {uuid: $uuid, repo_name: $repo_name})
                 RETURN root.uuid, root.name, root.kind, root.fqn,
                        root.signature, root.docstring, root.file_path, root.start_line
                 LIMIT 1",
            )
            .param("uuid", uuid)
            .param("repo_name", options.repo_name);
            (q, "uuid: $uuid".to_string())
        } else {
            let q = query(
                "MATCH (root:Entity {name: $name, repo_name: $repo_name})
                 RETURN root.uuid, root.name, root.kind, root.fqn,
                        root.signature, root.docstring, root.file_path, root.start_line
                 LIMIT 1",
            )
            .param("name", options.entity_name)
            .param("repo_name", options.repo_name);
            (q, "name: $name".to_string())
        };

        let mut rows = self
            .graph
            .execute(root_q)
            .await
            .context("Failed to query root entity")?;

        let root_node = if let Ok(Some(row)) = rows.next().await {
            SubgraphNode {
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
            }
        } else {
            // Root not found — return empty result
            return Ok(SubgraphResult {
                root_id: None,
                nodes: vec![],
                edges: vec![],
                truncated: false,
                total_nodes_found: 0,
            });
        };

        let root_uuid = root_node.uuid.clone();

        // --- 2. Collect nearby nodes ---
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
            "MATCH (root:Entity {{{root_match}, repo_name: $repo_name}}){arrow}(related:Entity)
             WHERE related.repo_name = $repo_name{kind_filter}
             RETURN DISTINCT related.uuid, related.name, related.kind, related.fqn,
                    related.signature, related.docstring, related.file_path, related.start_line",
            root_match = root_match_clause,
            arrow = direction_arrow,
            kind_filter = kind_filter,
        );

        let traversal_q = if let Some(uuid) = options.entity_uuid {
            query(&cypher)
                .param("uuid", uuid)
                .param("repo_name", options.repo_name)
        } else {
            query(&cypher)
                .param("name", options.entity_name)
                .param("repo_name", options.repo_name)
        };
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
