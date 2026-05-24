# Technical Specification: Subgraph Disconnection & Edge Extraction Fix

## 1. Problem Description

The entity subgraph feature in `knot` v1.3.7 (which introduced kind-aware traversal) exhibits two major bugs when viewing isolated graphs (e.g., focused entities in the graph viewer):
1.  **Missing Edges**: The graph viewer shows "0 edges" even when nodes are clearly related. This is caused by a serialization issue in `neo4rs` where `Vec<String>` parameters passed to an `IN` clause fail to match correctly in Neo4j.
2.  **Disconnected Components**: Nodes that should be connected through intermediate methods (e.g., `ClassA -> method1 -> method2 -> ClassB`) appear as isolated islands. This occurs because the traversal query lacks the `CONTAINS` relationship required to reach method nodes from classes, preventing the traversal from finding paths between visible entities.

## 2. Root Cause Analysis

### 2.1. Parameter Binding Bug
In `src/db/graph/query.rs`, the edge extraction query uses:
```rust
query(&edge_cypher).param("uuids", uuids)
```
Where `uuids` is a `Vec<String>`. The resulting Cypher `WHERE a.uuid IN $uuids` fails to resolve the list items correctly in the current version of the driver/database integration, leading to empty result sets.

### 2.2. Traversal Connectivity Gap
The traversal logic currently uses strictly the relationships requested by the user. For a Java codebase, classes are connected to methods via `CONTAINS`. If a user filters for `CALLS` but hides `Functions`, the traversal must still walk through `CONTAINS -> CALLS -> CONTAINS` to link two classes. Without automatic injection of `CONTAINS` into the **traversal** string, these paths are never discovered.

### 2.3. Filter Leakage in Direct Edges
The first branch of the `UNION` edge query:
```cypher
MATCH (a:Entity)-[r]->(b:Entity)
```
Does not constrain `r` to the requested relationship types. If `CONTAINS` is injected for traversal, it would also appear in the output even if the user didn't request it.

## 3. Proposed Changes (knot repository)

### 3.1. Implementation in `src/db/graph/query.rs`

#### Task 1: Connectivity-Aware Traversal
Modify `get_entity_subgraph` to use a separate relationship filter for the traversal phase that always includes `CONTAINS` when `visible_kinds` filtering is active.

```rust
// Step 2: Traversal relationships
let mut traversal_rels = relationships.to_vec();
if visible_kinds.is_some() && !traversal_rels.contains(&"CONTAINS") {
    traversal_rels.push("CONTAINS");
}
let traversal_rel_filter = traversal_rels.join("|");
// Use traversal_rel_filter in the direction_arrow generation
```

#### Task 2: UUID List Interpolation
To bypass the `neo4rs` binding bug, convert the collected UUIDs into a string literal for direct interpolation in the Cypher query.

```rust
// Step 3: Data preparation
let uuids_list: Vec<String> = nodes.iter().map(|n| format!("'{}'", n.uuid)).collect();
let uuids_str = uuids_list.join(", ");
```

#### Task 3: Edge Query Refactoring
Update the `edge_cypher` template to:
1.  Constrain the first `MATCH` branch to the original `rel_filter`.
2.  Replace all occurrences of `$uuids` with `[{uuids_str}]`.

```cypher
MATCH (a:Entity)-[r:{rel_filter}]->(b:Entity)
WHERE a.uuid IN [{uuids_str}] AND b.uuid IN [{uuids_str}]
RETURN DISTINCT a.uuid AS source_uuid, b.uuid AS target_uuid, type(r) AS relationship
UNION
// ... (repeat for other UNION branches)
```

#### Task 4: Clean up Parameters
Remove `.param("uuids", uuids)` from the query execution.

## 4. Verification Plan

### 4.1. Unit & Integration Testing
1.  Update existing tests in `src/db/graph/query.rs` to ensure they pass with the new parameter requirements.
2.  Add `test_get_entity_subgraph_connectivity`:
    *   Setup a graph: `ClassA -CONTAINS-> MethodA -CALLS-> MethodB <-CONTAINS- ClassB`.
    *   Query `ClassA` with `depth: 3`, `relationships: ["CALLS"]`, and `kinds: ["class"]`.
    *   Verify: Result contains `ClassA`, `ClassB` and an edge `ClassA -> ClassB`.

### 4.2. Versioning
Bump `knot` crate version to `1.3.8`.
