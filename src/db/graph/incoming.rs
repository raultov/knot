//! Edges into the entities of files an incremental run re-indexes (issue #53).
//!
//! A modified file is deleted and parsed again, but the files that reference
//! it are not: their edges into it (`CALLS`, `REFERENCES`, `EXTENDS`, …) go
//! with the deleted entities. They are captured before the deletion and
//! re-pointed afterwards at the re-indexed entity with the same file, FQN,
//! kind and signature — the nearest one by line when an overload repeats
//! them. An edge whose target no longer exists is dropped.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use neo4rs::{BoltType, query};

use super::GraphDb;

/// Edges recomputed by their own pass after every run.
const RECOMPUTED_EDGES: &[&str] = &["HTTP_CALLS"];

/// Edges re-pointed per query.
const RESTORE_BATCH: usize = 1000;

/// An edge from outside the re-indexed files into one of their entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingEdge {
    /// UUID of the entity the edge starts from.
    pub source: String,
    pub rel_type: String,
    /// Identity of the target entity, which survives its re-indexing.
    pub file_path: String,
    pub fqn: String,
    pub kind: String,
    /// `""` for an entity without a signature.
    pub signature: String,
    pub start_line: i64,
}

#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait IncomingEdgeExt {
    /// The edges from entities outside `file_paths` into entities of
    /// `file_paths` (repo-relative, in `repo_name`).
    async fn incoming_edges(
        &self,
        repo_name: &str,
        file_paths: &[String],
    ) -> Result<Vec<IncomingEdge>>;
    /// Re-create `edges` towards the re-indexed entities of `repo_name`;
    /// returns how many were restored.
    async fn restore_incoming_edges(
        &self,
        repo_name: &str,
        edges: &[IncomingEdge],
    ) -> Result<usize>;
}

impl IncomingEdgeExt for GraphDb {
    async fn incoming_edges(
        &self,
        repo_name: &str,
        file_paths: &[String],
    ) -> Result<Vec<IncomingEdge>> {
        if file_paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (src:Entity)-[r]->(t:Entity)
                     WHERE t.repo_name = $repo_name AND t.file_path IN $file_paths
                       AND t.fqn IS NOT NULL
                       AND NOT (src.repo_name = $repo_name AND src.file_path IN $file_paths)
                       AND NOT type(r) IN $recomputed
                     RETURN DISTINCT src.uuid AS source, type(r) AS rel_type,
                            t.file_path AS file_path, t.fqn AS fqn, t.kind AS kind,
                            coalesce(t.signature, '') AS signature, t.start_line AS start_line",
                )
                .param("repo_name", repo_name)
                .param("file_paths", file_paths)
                .param(
                    "recomputed",
                    RECOMPUTED_EDGES
                        .iter()
                        .map(|t| t.to_string())
                        .collect::<Vec<_>>(),
                ),
            )
            .await
            .context("Failed to read the edges into re-indexed files")?;
        let mut edges = Vec::new();
        while let Some(row) = rows.next().await? {
            let (Ok(source), Ok(rel_type), Ok(file_path), Ok(fqn), Ok(kind)) = (
                row.get::<String>("source"),
                row.get::<String>("rel_type"),
                row.get::<String>("file_path"),
                row.get::<String>("fqn"),
                row.get::<String>("kind"),
            ) else {
                continue;
            };
            edges.push(IncomingEdge {
                source,
                rel_type,
                file_path,
                fqn,
                kind,
                signature: row.get::<String>("signature").unwrap_or_default(),
                start_line: row.get::<i64>("start_line").unwrap_or_default(),
            });
        }
        Ok(edges)
    }

    async fn restore_incoming_edges(
        &self,
        repo_name: &str,
        edges: &[IncomingEdge],
    ) -> Result<usize> {
        let mut restored = 0;
        for (rel_type, edges) in edges_by_type(edges) {
            for batch in edges.chunks(RESTORE_BATCH) {
                let params: Vec<HashMap<String, BoltType>> =
                    batch.iter().map(|edge| edge_params(edge)).collect();
                let mut rows = self
                    .graph
                    .execute(
                        query(&restore_query(rel_type))
                            .param("repo_name", repo_name)
                            .param("edges", params),
                    )
                    .await
                    .with_context(|| format!("Failed to restore {rel_type} edges"))?;
                if let Some(row) = rows.next().await? {
                    restored += row.get::<i64>("restored").unwrap_or_default() as usize;
                }
            }
        }
        Ok(restored)
    }
}

/// `edges` grouped by relationship type, types that are not plain Cypher
/// identifiers left out (they are interpolated into the query).
fn edges_by_type(edges: &[IncomingEdge]) -> BTreeMap<&str, Vec<&IncomingEdge>> {
    let mut grouped: BTreeMap<&str, Vec<&IncomingEdge>> = BTreeMap::new();
    for edge in edges.iter().filter(|e| is_rel_type(&e.rel_type)) {
        grouped
            .entry(edge.rel_type.as_str())
            .or_default()
            .push(edge);
    }
    grouped
}

fn is_rel_type(rel_type: &str) -> bool {
    !rel_type.is_empty()
        && rel_type
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn edge_params(edge: &IncomingEdge) -> HashMap<String, BoltType> {
    HashMap::from([
        ("source".to_string(), edge.source.clone().into()),
        ("file_path".to_string(), edge.file_path.clone().into()),
        ("fqn".to_string(), edge.fqn.clone().into()),
        ("kind".to_string(), edge.kind.clone().into()),
        ("signature".to_string(), edge.signature.clone().into()),
        ("start_line".to_string(), edge.start_line.into()),
    ])
}

/// Re-point each edge of type `rel_type` at the re-indexed entity with the
/// same identity, the nearest by line when several match.
fn restore_query(rel_type: &str) -> String {
    format!(
        "UNWIND $edges AS e
         MATCH (src:Entity {{uuid: e.source}})
         MATCH (t:Entity {{repo_name: $repo_name, file_path: e.file_path, fqn: e.fqn}})
         WHERE t.kind = e.kind AND coalesce(t.signature, '') = e.signature
         WITH e, src, t ORDER BY abs(t.start_line - e.start_line), t.uuid
         WITH e, src, collect(t)[0] AS target
         MERGE (src)-[:{rel_type}]->(target)
         RETURN count(*) AS restored"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(rel_type: &str, source: &str) -> IncomingEdge {
        IncomingEdge {
            source: source.to_string(),
            rel_type: rel_type.to_string(),
            file_path: "src/Ledger.java".to_string(),
            fqn: "com.x.Ledger.post".to_string(),
            kind: "method".to_string(),
            signature: "(String entry)".to_string(),
            start_line: 4,
        }
    }

    #[test]
    fn edges_are_grouped_by_type_and_unsafe_types_dropped() {
        let edges = vec![
            edge("CALLS", "a"),
            edge("REFERENCES", "b"),
            edge("CALLS", "c"),
            edge("CALLS]->(x) DETACH DELETE x //", "d"),
            edge("", "e"),
        ];
        let grouped = edges_by_type(&edges);
        assert_eq!(
            grouped.keys().copied().collect::<Vec<_>>(),
            ["CALLS", "REFERENCES"]
        );
        let calls: Vec<&str> = grouped["CALLS"].iter().map(|e| e.source.as_str()).collect();
        assert_eq!(calls, ["a", "c"]);
    }

    #[test]
    fn restore_query_matches_identity_and_prefers_the_nearest_line() {
        let q = restore_query("CALLS");
        assert!(q.contains("MERGE (src)-[:CALLS]->(target)"));
        assert!(q.contains("file_path: e.file_path, fqn: e.fqn"));
        assert!(q.contains("coalesce(t.signature, '') = e.signature"));
        assert!(q.contains("ORDER BY abs(t.start_line - e.start_line)"));
    }

    #[test]
    fn recomputed_edges_are_not_captured() {
        assert!(RECOMPUTED_EDGES.contains(&"HTTP_CALLS"));
        assert!(is_rel_type("HTTP_CALLS") && is_rel_type("MACRO_CALLS"));
    }
}
