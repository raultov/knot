# Technical Specification: Root Cause & Resolution for Subgraph Disconnection

## 1. Executive Summary
The subgraph generation feature in `knot` v1.3.7 suffers from a profound architectural issue regarding how hierarchical class-method relationships are represented in the graph database. While the codebase relies on the `CONTAINS` relationship to navigate from classes to their methods, **the `CONTAINS` edges are never actually created in Neo4j by any indexer**. The fallback to `enclosing_class` properties in edge extraction queries was a workaround that masked this fundamental missing link, and the latest refactoring broke the fragile equilibrium. Additionally, a serialization bug in `neo4rs` causes edge queries to fail completely, resulting in "0 edges" being rendered.

## 2. Root Cause Analysis

### 2.1. The Missing `CONTAINS` Edges (Disconnected Nodes)
In languages like Java, classes do not directly `CALL` other classes; their methods call other methods. To find dependencies between classes, a graph traversal must walk `ClassA -> MethodA -> MethodB -> ClassB`. The expected path involves `-[:CONTAINS]->` and `-[:CALLS]->` edges.
However, the `knot` indexer only saves `enclosing_class` as a text property on the method node (`e.enclosing_class = "ClassA"`). **It never creates physical `-[:CONTAINS]->` edges in Neo4j**. 
When the Cypher traversal executes `MATCH ... -[:CALLS|EXTENDS|CONTAINS*1..3]- ...`, Neo4j cannot descend into the methods because no `CONTAINS` edge exists. The traversal halts at the class level, only discovering entities connected by direct inheritance (`EXTENDS`, `IMPLEMENTS`).

### 2.2. The Parameter Binding Bug ("0 Edges" Rendered)
In Step 3 (Edge Extraction), the query passes a Rust `Vec<String>` to the `$uuids` parameter. The `neo4rs` driver does not reliably serialize Vectors for the Cypher `IN` operator depending on context. Consequently, `WHERE a.uuid IN $uuids` fails silently, returning 0 rows. This is why the UI shows nodes but no connecting lines.

## 3. Implementation Plan (in `knot` library)

### 3.1. Create Physical `CONTAINS` Edges at Index Time
Instead of relying on fragile, complex roll-up queries over text properties, we must materialize the hierarchy in the database.

In `src/db/graph/upsert.rs`, add a new step at the end of the `upsert_entities` function (or wherever entities are finalized) to auto-link entities:

```rust
// Auto-link entities using their enclosing_class property
let link_cypher = format!(
    "MATCH (m:Entity {{repo_name: $repo_name}})
     WHERE m.enclosing_class IS NOT NULL AND m.enclosing_class <> ''
     MATCH (c:Entity {{name: m.enclosing_class, repo_name: $repo_name}})
     MERGE (c)-[:CONTAINS]->(m)"
);
self.graph.run(query(&link_cypher).param("repo_name", repo_id)).await?;
```

### 3.2. Fix the Neo4rs `$uuids` Bug
In `src/db/graph/query.rs` (Edge extraction):
Use string interpolation to inject the UUIDs safely into the Cypher query.

```rust
let uuids_list: Vec<String> = nodes.iter().map(|n| format!("'{}'", n.uuid)).collect();
let uuids_str = uuids_list.join(", ");
// Use `WHERE a.uuid IN [{uuids_str}]` in all UNION branches
```

### 3.3. Ensure Traversal Uses `CONTAINS`
In `src/db/graph/query.rs` (Traversal):
Always inject `CONTAINS` into the traversal relationships if `visible_kinds` filtering is active, ensuring the query can dive into methods.

```rust
let mut traversal_rels = relationships.to_vec();
if visible_kinds.is_some() && !traversal_rels.contains(&"CONTAINS") {
    traversal_rels.push("CONTAINS");
}
let traversal_rel_filter = traversal_rels.join("|");
// Use in direction_arrow: format!("-[:{traversal_rel_filter}*1..{depth}]-")
```

### 3.4. Update Edge Roll-Up Queries
With physical `CONTAINS` edges available, the complex edge roll-up queries should be rewritten to use the actual graph topology rather than text matching.

Example for `Class -> Class` synthetic edge based on method calls:
```cypher
MATCH (c1:Entity)-[:CONTAINS]->(m1:Entity)-[r:{rel_filter}]->(m2:Entity)<-[:CONTAINS]-(c2:Entity)
WHERE c1.uuid IN [{uuids_str}] AND c2.uuid IN [{uuids_str}] AND c1.uuid <> c2.uuid
  AND NOT m1.kind IN [{visible_kind_list}]
  AND NOT m2.kind IN [{visible_kind_list}]
RETURN DISTINCT c1.uuid AS source_uuid, c2.uuid AS target_uuid, type(r) AS relationship
```

## 4. Next Steps
1. Implement these fixes in the `knot` library.
2. Bump `knot` version to `1.3.9`.
3. Resync repositories (e.g., `HikariCP`) to generate the new `CONTAINS` edges.
