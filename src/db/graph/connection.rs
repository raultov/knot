use anyhow::{Context, Result};
use neo4rs::query;
use tracing::info;

use super::GraphDb;

/// Extension trait for connection and initialization operations.
#[allow(async_fn_in_trait)]
pub trait ConnectExt {
    async fn connect(uri: &str, user: &str, password: &str) -> Result<Self>
    where
        Self: Sized;
    async fn ensure_indexes(&self) -> Result<()>;
}

impl ConnectExt for GraphDb {
    /// Connect to Neo4j via Bolt and return a ready-to-use [`GraphDb`].
    async fn connect(uri: &str, user: &str, password: &str) -> Result<GraphDb> {
        let graph =
            neo4rs::Graph::new(uri, user, password).context("Failed to connect to Neo4j")?;

        info!("Connected to Neo4j at {uri}");
        Ok(GraphDb { graph })
    }

    /// Ensure necessary indexes exist for fast lookups by UUID, repo_name, and file_path.
    async fn ensure_indexes(&self) -> Result<()> {
        let stmts = [
            // UUID uniqueness constraint (covers Method, Class, Interface, Function)
            "CREATE CONSTRAINT entity_uuid_unique IF NOT EXISTS \
             FOR (e:Entity) REQUIRE e.uuid IS UNIQUE",
            // Index on repo_name for multi-repository isolation and fast filtering
            "CREATE INDEX entity_repo_name IF NOT EXISTS \
             FOR (e:Entity) ON (e.repo_name)",
            // Index on file_path for quick path-based lookups
            "CREATE INDEX entity_file_path IF NOT EXISTS \
             FOR (e:Entity) ON (e.file_path)",
        ];

        for stmt in &stmts {
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
}
