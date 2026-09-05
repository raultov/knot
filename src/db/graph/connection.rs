use anyhow::{Context, Result};
use neo4rs::query;
use tracing::info;

use super::GraphDb;

/// Extension trait for connection and initialization operations.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait ConnectExt {
    async fn connect(uri: &str, user: &str, password: &str) -> Result<Self>
    where
        Self: Sized;
    async fn ensure_indexes(&self) -> Result<()>;
}

/// Index and constraint statements applied at startup.
///
/// Module-private so the statements can be unit-tested without a live Neo4j.
fn index_statements() -> &'static [&'static str] {
    &[
        "CREATE CONSTRAINT entity_uuid_unique IF NOT EXISTS \
         FOR (e:Entity) REQUIRE e.uuid IS UNIQUE",
        "CREATE INDEX entity_repo_name IF NOT EXISTS \
         FOR (e:Entity) ON (e.repo_name)",
        "CREATE INDEX entity_file_path IF NOT EXISTS \
         FOR (e:Entity) ON (e.file_path)",
        // Composite index for CONTAINS auto-link parent lookup:
        // OPTIONAL MATCH (c1:Entity {fqn, repo_name}). Without it the
        // lookup degrades to a per-row scan of every entity in the repo
        // and large repos time out at the end of indexing.
        "CREATE INDEX entity_repo_fqn IF NOT EXISTS \
         FOR (e:Entity) ON (e.repo_name, e.fqn)",
        "CREATE CONSTRAINT repo_name_unique IF NOT EXISTS \
         FOR (r:Repository) REQUIRE r.name IS UNIQUE",
        "CREATE INDEX repo_artifact IF NOT EXISTS \
         FOR (r:Repository) ON (r.group_id, r.artifact_id)",
        "CREATE TEXT INDEX entity_name_text IF NOT EXISTS \
         FOR (e:Entity) ON (e.name)",
        "CREATE TEXT INDEX entity_fqn_text IF NOT EXISTS \
         FOR (e:Entity) ON (e.fqn)",
    ]
}

impl ConnectExt for GraphDb {
    /// Connect to Neo4j via Bolt and return a ready-to-use [`GraphDb`].
    async fn connect(uri: &str, user: &str, password: &str) -> Result<GraphDb> {
        let graph =
            neo4rs::Graph::new(uri, user, password).context("Failed to connect to Neo4j")?;

        info!("Connected to Neo4j at {uri}");
        Ok(GraphDb { graph })
    }

    /// Ensure necessary indexes exist for UUID, repo_name, file_path, and
    /// the composite (repo_name, fqn) index used by CONTAINS auto-linking.
    async fn ensure_indexes(&self) -> Result<()> {
        for stmt in index_statements() {
            self.graph
                .run(query(stmt))
                .await
                .context("Failed to create Neo4j index/constraint")?;
        }

        info!("Neo4j indexes/constraints verified");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::GraphDb;
    use super::ConnectExt;
    use super::index_statements;

    // -----------------------------------------------------------------------
    // Unit tests (no Neo4j needed — assert on index statement strings)
    // -----------------------------------------------------------------------

    #[test]
    fn test_index_statements_include_composite_repo_fqn_index() {
        let stmts = index_statements();
        let found = stmts
            .iter()
            .any(|s| s.contains("entity_repo_fqn") && s.contains("ON (e.repo_name, e.fqn)"));
        assert!(
            found,
            "index_statements() must include composite index ON (e.repo_name, e.fqn)"
        );
    }

    #[test]
    fn test_index_statements_are_idempotent() {
        for s in index_statements() {
            assert!(
                s.contains("IF NOT EXISTS"),
                "statement must start with IF NOT EXISTS for safe migration: {s}"
            );
        }
    }

    #[test]
    fn test_index_statements_preserve_existing_indexes() {
        let all: String = index_statements().join("\n");
        for name in &[
            "entity_uuid_unique",
            "entity_repo_name",
            "entity_file_path",
            "repo_name_unique",
            "repo_artifact",
            "entity_name_text",
            "entity_fqn_text",
        ] {
            assert!(
                all.contains(name),
                "index_statements() must preserve existing: {name}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Integration tests (require local Neo4j on bolt://localhost:7687)
    // -----------------------------------------------------------------------

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_graph_db_connection() {
        // This test requires a running Neo4j instance
        // Run with: cargo test -- --ignored --test-threads=1
        let result = GraphDb::connect("bolt://localhost:7687", "neo4j", "password").await;
        assert!(result.is_ok(), "Should be able to connect to Neo4j");
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_graph_db_ensure_indexes() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.ensure_indexes().await;
        assert!(
            result.is_ok(),
            "Should be able to create indexes in Neo4j: {:?}",
            result.err()
        );
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_ensure_indexes_creates_entity_repo_fqn_index() {
        use neo4rs::query;
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        graph_db
            .ensure_indexes()
            .await
            .expect("ensure_indexes must succeed");

        let mut rows = graph_db
            .graph
            .execute(query("SHOW INDEXES YIELD name, properties"))
            .await
            .expect("SHOW INDEXES must succeed");

        let mut found = false;
        while let Ok(Some(row)) = rows.next().await {
            if row.get::<String>("name").unwrap_or_default() == "entity_repo_fqn" {
                let props: Vec<String> = row.get::<Vec<String>>("properties").unwrap_or_default();
                assert_eq!(
                    props,
                    vec!["repo_name".to_string(), "fqn".to_string()],
                    "entity_repo_fqn index must be ON (repo_name, fqn)"
                );
                found = true;
                break;
            }
        }
        assert!(
            found,
            "entity_repo_fqn composite index not found after ensure_indexes"
        );
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_contains_auto_link_explain_succeeds() {
        use neo4rs::query;
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        graph_db
            .ensure_indexes()
            .await
            .expect("ensure_indexes must succeed");

        // Cipher identical to build_contains_auto_link_cypher() in upsert.rs.
        // EXPLAIN does not execute the query — it only returns the plan.
        let cypher = "\
            UNWIND $entity_uuids AS entity_uuid\n\
            MATCH (m:Entity {uuid: entity_uuid})\n\
            WHERE m.enclosing_class IS NOT NULL AND m.enclosing_class <> ''\n\
            OPTIONAL MATCH (c1:Entity {fqn: m.enclosing_class_fqn, repo_name: $repo_name})\n\
            WITH m, c1\n\
            OPTIONAL MATCH (c2:Entity {name: m.enclosing_class, repo_name: $repo_name, file_path: m.file_path})\n\
            WITH m, COALESCE(c1, c2) AS c\n\
            WHERE c IS NOT NULL\n\
            MERGE (c)-[:CONTAINS]->(m)";
        let explain = format!("EXPLAIN {cypher}");

        let result = graph_db
            .graph
            .execute(
                query(&explain)
                    .param("repo_name", "nonexistent")
                    .param("entity_uuids", Vec::<String>::new()),
            )
            .await;

        assert!(
            result.is_ok(),
            "EXPLAIN of auto-link cypher must succeed: {:?}",
            result.err()
        );
    }
}
