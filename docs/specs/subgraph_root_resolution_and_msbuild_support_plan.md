# Deterministic Subgraph Root Resolution & MSBuild/NuGet Build-System Support — Implementation Plan

**Status:** Proposed
**Scope:** Part A: `src/db/graph/query_subgraph.rs`, `src/db/graph/query.rs`, `src/models/subgraph.rs`. Part B: `src/pipeline/input.rs`, `src/pipeline/files.rs`, `src/pipeline/parser/mod.rs`, `src/pipeline/parser/languages/mod.rs`, `src/pipeline/parser/languages/msbuild.rs` (new), `src/pipeline/ingest/resolve/cross_repo.rs`, `tests/run_build_systems_e2e.sh`, `tests/run_cross_repo_dep_e2e.sh`, `README.md`, `CHANGELOG.md`, `src/mcp_tools/list_repo_dependencies.rs`, `docs/specs/csharp_support_plan.md`
**Methodology:** BDD/TDD (Red → Green → Refactor), per `AGENTS.md` § Testing Strategy
**Related:** `docs/specs/find_callers_target_resolution_plan.md` (the resolution ladder Part A reuses; its `**Status:** Proposed` line is stale — it is implemented in the tree at `src/db/graph/query.rs:6-211`), `docs/specs/csharp_support_plan.md` §14.1 (the MSBuild phase this plan supersedes and corrects), companion plan `knot-server/docs/specs/nested_types_overview_graph_plan.md`

Two independent defects found while integrating the C# support with knot-server (2026-08-30). They share **no code paths** and can be implemented, tested and released independently; they are documented together because they were triaged together. Either part can be extracted into its own spec without affecting the other.

---

## 1. Context and Problem Statement

### 1.1 Part A — resolving an entity by bare name is arbitrary

`GET /api/repos/csharp-code-map/graph?entity=McpServer` (knot-server delegating to
`GraphDb::get_entity_subgraph`) resolves the root to the **constructor**
`CodeMap.Mcp.McpServer.McpServer` (kind `csharp_constructor`, `src/CodeMap.Mcp/McpServer.cs:32`)
instead of the class `CodeMap.Mcp.McpServer` (kind `csharp_class`, `McpServer.cs:20`).

In C# every explicit constructor shares its class's name, so this is systematic:
measured against the live Neo4j (2026-08-30), **201 distinct names collide** between
`csharp_class` and `csharp_constructor` entities in `csharp-code-map` alone. Java never
exposed the defect because its extractor emits no constructor entities
(`rg constructor queries/java.scm` → no matches).

Worse, the defect is two-dimensional (see §4): the root *pick* is unordered, **and** the
traversal re-matches by name so the returned subgraph is the union of every homonym's
neighbourhood while `root_id` names one arbitrary member.

### 1.2 Part B — C# repositories report `build_system: "none"` and have no cross-repo edges

```
$ curl -s localhost:3000/api/repos/openlogi-net/graph/repos?depth=3
{"root_id":"openlogi-net","nodes":[{"id":"openlogi-net",...,"build_system":"none",...}],"edges":[],...}
```

`.csproj` files are never discovered (§10.1), so no `ProjectIdentity` is emitted,
`link_cross_repo_dependencies` (`src/pipeline/ingest/resolve/cross_repo.rs:8-63`) takes the
`else` branch and writes `build_system = "none"` with empty group/artifact/version, and no
`DEPENDS_ON` edge can ever be produced for a C# repo. This blocks the entire cross-repo
dependency graph (knot-server `/graph/repos`, `/deps`, MCP `list_repo_dependencies`) for C#.

---

## 2. Verified Evidence (live Neo4j, 2026-08-30)

| Fact | Value |
|---|---|
| Entities named `McpServer` in `csharp-code-map` | 2 — `csharp_class` (McpServer.cs:20, FQN `CodeMap.Mcp.McpServer`) and `csharp_constructor` (:32, FQN `CodeMap.Mcp.McpServer.McpServer`) |
| Names colliding class↔constructor in `csharp-code-map` | 201 |
| Entities in `openlogi-net` / `csharp-code-map` | 3 292 / 7 970 |
| `csharp_namespace` homonyms | one per declaring file — 356 / 1 055 |
| Synthetic `<module>` entities | one per file with orphaned references, identical `name` **and** `fqn` (`src/pipeline/parser/orphans.rs:40-64`) |
| `openlogi-net` `.csproj` files with UTF-8 BOM | 9 of 9 (`head -c 3` → `ef bb bf`) |
| `openlogi-net` `PackageReference` elements | 15, all with `Version` attribute; 0 CPM; 0 `PackageId` anywhere in the repo |
| `csharp-code-map` `PackageReference` elements | 83, of which **78 version-less** (resolved by 19 `<PackageVersion>` entries in `Directory.Packages.props` — Central Package Management) |
| Only explicit `PackageId` in either repo | `codemap-mcp` v2.8.1 (`csharp-code-map/src/CodeMap.Daemon/CodeMap.Daemon.csproj:9-10`) |
| `ProjectReference` elements | 22 (openlogi-net) / 50 (csharp-code-map) |
| `$(...)` MSBuild properties in version-bearing position | **none** in either repo — `$(Configuration)`, `$(RuntimeIdentifier)`, `$(NoWarn)` appear only in `Condition`/append positions |

---

# PART A — Deterministic Subgraph Root Resolution

## 3. Current State (A)

All line numbers are from the current working tree.

### 3.1 The root query has no ordering (`src/db/graph/query_subgraph.rs:59-80`)

```rust
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
```

The `entity_uuid` branch is deterministic — `uuid` has a uniqueness constraint
(`src/db/graph/connection.rs:24-25`). The `entity_name` branch has **no `ORDER BY` and no
tie-break**: `LIMIT 1` on an unordered match is Neo4j-implementation-defined and can change
across re-indexes. Only the first row is consumed (`:88-112`); absence yields an empty
`SubgraphResult` with `root_id: None`.

### 3.2 The traversal re-matches ALL homonyms by name (`:146-154`, `:156-164`)

```rust
        let cypher = format!(
            "MATCH (root:Entity {{{root_match}, repo_name: $repo_name}}){arrow}(related:Entity)
             WHERE related.repo_name = $repo_name{kind_filter}
             RETURN DISTINCT related.uuid, related.name, related.kind, related.fqn,
                    related.signature, related.docstring, related.file_path, related.start_line",
            root_match = root_match_clause, ...
```

`root_match_clause` is the *string* `"name: $name"` spliced back into the traversal, so the
name branch binds **every** homonym and returns the `DISTINCT` union of their neighbourhoods —
while `root_id` (set at `:283`) points at one arbitrary member. The `LIMIT 1` on the root query
and the unbounded match in the traversal are inconsistent by construction. **Fixing only the
root pick without anchoring the traversal on the chosen UUID leaves the union behaviour intact;
both sites must change together.**

### 3.3 Supporting facts

- `visible_kinds` filtering applies to `related` only (`:135-144`); the root is inserted
  unconditionally (`:117-118`) and survives. Keep this behaviour (knot-server's
  `filter_unconnected_nodes` retains the root unconditionally too).
- The `CONTAINS` auto-injection into the traversal when `visible_kinds` is set (`:120-126`)
  is deliberate and stays. Side effect that matters: from a *class* root, the constructor's
  neighbourhood is re-gained in one `CONTAINS` hop — choosing the class as root is a strict
  improvement, not a loss of information.
- Truncation collects from a `HashMap` before truncating (`:191-194`,
  `all_nodes.into_values().collect()` then `nodes.truncate(...)`), so **which** nodes survive
  `max_nodes` is also non-deterministic. Cheap adjacent fix: sort by `uuid` before truncating.
- The FQN of the requested entity never matches today: the root query compares `$name`
  against the `name` property, so `?entity=CodeMap.Mcp.McpServer` (fully qualified) returns an
  empty subgraph. Adopting the tier ladder fixes this as a bonus.

### 3.4 The resolution ladder already exists — reuse it

`src/db/graph/query.rs` (implemented, unit-tested, shipped as v1.7.1):

- `MatchTier` enum + `as_str` (`:6-26`); `MIN_FUZZY_LEN = 4`, `MAX_TARGETS = 25` (`:38-44`).
- `target_resolution_tiers(name)` (`:46-77`) — pure function returning the ordered predicate
  ladder: `ExactFqn` → `FqnSuffix` (only for dotted/`::` queries) → `ExactName` →
  `SignaturePrefix` (only when the query contains `(`) → `Fuzzy` (only when `len >= 4`).
- `resolve_reference_targets` (`:143-211`) — executes the ladder with early stop, first
  non-empty tier wins, `ORDER BY target.fqn` inside each tier.

The ladder returns a **set** (legitimately N homonyms in `ExactName`), which is correct for
`find_callers` but insufficient here: the subgraph needs exactly one root. Reuse the tier
*predicates*; add a kind-precedence ranking stage the reference resolver does not need.

### 3.5 The kind-precedence policy already exists — at the ingest layer

`src/pipeline/ingest/resolve/non_calls.rs:10-47` — its doc comment describes **this very
defect**:

```rust
/// Type-like kinds that may be the target of an inheritance or type-usage
/// reference. C# constructor entities share their class's name
/// (`BaseService` class + `BaseService` constructor), so name-only
/// resolution for `Extends` / `Implements` / `TypeReference` intents must
/// restrict candidates to types or the constructor wins the ambiguity and
/// the edge is dropped as ambiguous.
fn is_type_like(kind: &EntityKind) -> bool { ... }
```

The ingest layer already solved the class-vs-constructor ambiguity for edge resolution,
language-agnostically. The query layer never got the same treatment. Part A extends the
proven policy to the one layer that lacks it. Complementary precedent: `is_constructor`
(`src/pipeline/ingest/resolve/overrides.rs:112-118`).

### 3.6 Existing test coverage is zero (non-ignored)

`src/db/graph/query_subgraph.rs:292-496` holds eight tests, every one `#[ignore]`d
("requires local Neo4j"), unseeded, and vacuous even when run (empty DB → the root
early-return makes every `is_ok()` assertion pass). **Any new resolution logic must be
extracted into pure functions** so it is testable without Neo4j — exactly the constraint the
find_callers plan stated ("The ladder and the formatter must be testable without a live
Neo4j"). The subgraph has no CLI subcommand and no MCP tool: the only consumer is
knot-server via `knot::cli_tools::run_get_subgraph` (`src/cli_tools/subgraph.rs:38-56`,
re-exported at `src/cli_tools/mod.rs:21`), called from knot-server
`graph_handler` and `graph_expand_handler`.

---

## 4. Root Cause Analysis (A)

Three defects, in order of impact:

| # | Defect | Location | Effect |
|---|---|---|---|
| A1 | Unordered `LIMIT 1` root pick | `query_subgraph.rs:71-79` | An arbitrary homonym becomes `root_id` |
| A2 | Traversal re-matches by name, returning the union of all homonyms' neighbourhoods | `query_subgraph.rs:146-154` | Result set is a chimera of the class *and* the constructor; `root_id` may not even be connected to most of it |
| A3 | `HashMap` iteration order before `truncate` | `query_subgraph.rs:191-194` | Under `max_nodes`, the retained subset is arbitrary |

Collision sources that make A1/A2 visible (§1 of the research; all verified in-tree):
C# constructors (`queries/csharp.scm:68-70`, FQN via `csharp/fqn.rs:49-80`), C++ constructors
(`queries/cpp.scm:6-8` — a `function_definition` named like the class, same defect class,
already shipped), synthetic `<module>` entities whose name *and* FQN are the literal
`"<module>"` in every file (`orphans.rs:40-64`), one `csharp_namespace` per declaring file,
and C# partial classes (same name **and** FQN, differing only in `file_path`). Consequence:
**neither `name` nor `fqn` is a unique key — the final tie-break must be
`(file_path, start_line, uuid)`.** The precedent for a total order tail is
`find_entities_by_name_prefix` (`query.rs:529-559`: `... size(m.name), m.fqn, m.uuid`).

---

## 5. Design (A)

### 5.1 Kind ranking — a pure, language-agnostic precedence

New pure function in `src/db/graph/query.rs`, next to `target_resolution_tiers`:

```rust
/// Root-preference rank for an entity kind (wire format, i.e. the snake_case
/// string stored in Neo4j — `SubgraphNode.kind` is read straight from the
/// `root.kind` property, and `EntityKind`'s wire form is its `Display` impl,
/// `src/models/entity.rs:148-265`). Lower ranks win.
///
/// Seed list lifted from `is_type_like` (`non_calls.rs:10-47`) minus the
/// namespaces: a `csharp_namespace` named like a type must not outrank the
/// type. Namespaces are containers, ranked below callables.
pub fn root_kind_rank(kind: Option<&str>) -> u8
```

| Rank | Meaning | Wire kinds (non-exhaustive; full list in the implementation, seeded from `non_calls.rs:10-47`) |
|---|---|---|
| 0 | type declarations | `class`, `interface`, `enum`, `kotlin_class/interface/object/companion_object/enum`, `rust_struct/enum/union/trait/type_alias`, `python_class`, `c_struct`, `cpp_class`, `groovy_class/interface/trait/enum`, `csharp_class/interface/struct/record/enum/delegate` |
| 1 | callables | `method`, `function`, `kotlin_function/method`, `rust_function/method/macro_def`, `python_function/method`, `c_function`, `cpp_method`, `csharp_method/constructor/local_function/operator/indexer`, `groovy_method/function`, `macro_definition`, `scss_function/mixin`, `vcl_subroutine`, `vcl_builtin_sub`, `vcc_function/method` |
| 2 | members / data | `constant`, `kotlin_property`, `rust_constant/static`, `python_constant`, `csharp_property/field/constant/event`, `groovy_property`, `config_property`, `helm_value`, `css_variable`, `scss_variable` |
| 3 | containers | `rust_module`, `python_module`, `cpp_namespace`, `csharp_namespace`, `markdown_document`, `markdown_section` |
| 4 | everything else | `html_*`, `build_*`, `k8s_*`, `project_identity`, `vtc_*`, … |

`Some("csharp_class")` → 0 beats `Some("csharp_constructor")` → 1. `None` kind → 4.

### 5.2 Candidate ordering — a pure total order

```rust
/// Orders root candidates by (root_kind_rank, file_path, start_line, uuid).
/// The trailing (file_path, start_line, uuid) tail is a total order — required
/// because neither `name` nor `fqn` is unique (`<module>` entities share both;
/// partial classes share both). Mirrors the `m.fqn, m.uuid` tail of
/// `find_entities_by_name_prefix` (`query.rs:543-544`).
pub fn rank_root_candidates(candidates: Vec<RootCandidate>) -> Vec<RootCandidate>
```

with a new lightweight row type local to the query layer (the existing `TargetRow`
(`query.rs:28-36`) lacks `signature`/`docstring`, which the subgraph root carries):

```rust
#[derive(Debug, Clone)]
pub struct RootCandidate {
    pub uuid: String,
    pub name: String,
    pub fqn: Option<String>,
    pub kind: Option<String>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub file_path: Option<String>,
    pub start_line: Option<i64>,
}
```

### 5.3 New resolver method on `GraphDb`

```rust
    /// Resolve a user-supplied entity name to exactly one root candidate,
    /// walking the same tier ladder as `resolve_reference_targets`
    /// (`target_resolution_tiers`) with early stop, then applying
    /// `rank_root_candidates` inside the winning tier. Returns the ordered
    /// candidates of the winning tier plus the tier that produced them.
    async fn resolve_subgraph_root(
        &self,
        name: &str,
        repo_name: &str,
    ) -> Result<Option<(RootCandidate, MatchTier, usize /* total_candidates in tier */)>>
```

Per tier, one query of the shape (node aliased `target` so the existing tier predicate
strings are reused **verbatim**):

```cypher
MATCH (target:Entity {repo_name: $repo_name})
WHERE <tier predicate — from target_resolution_tiers(name)>
RETURN target.uuid, target.name, target.fqn, target.kind,
       target.signature, target.docstring, target.file_path, target.start_line
ORDER BY target.fqn
LIMIT 25   -- MAX_TARGETS; candidates beyond the cap cannot be ranked fairly
```

First non-empty tier wins; `rank_root_candidates` picks the head. `None` only if every tier
is empty (→ caller returns the existing empty `SubgraphResult`).

### 5.4 Anchor the traversal on the resolved UUID (fixes A2)

In `get_entity_subgraph`:

- `entity_uuid` branch: unchanged resolution, but the traversal switches to the UUID anchor.
- `entity_name` branch: call `resolve_subgraph_root`; on `Some`, use the winning candidate as
  the root node.
- The traversal Cypher becomes, for **both** branches:

```cypher
MATCH (root:Entity {uuid: $root_uuid, repo_name: $repo_name}){arrow}(related:Entity)
WHERE related.repo_name = $repo_name{kind_filter}
RETURN DISTINCT related.uuid, ...
```

with a single `.param("root_uuid", root_uuid)`. `root_match_clause` is deleted (it exists
only to be spliced; §3.2). Determinism of the whole result now follows from the resolver.

### 5.5 Deterministic truncation (fixes A3)

Before `nodes.truncate(options.max_nodes)` (`:191-194`), sort the collected nodes by `uuid`.
One line; makes the retained subset deterministic. Behaviour note: truncation already drops
arbitrary nodes; after this it drops the same ones every time.

### 5.6 Disclosure — additive, optional

`SubgraphResult` (`src/models/subgraph.rs:50-60`) gains one field:

```rust
    /// How the root was resolved when queried by name. `None` when the subgraph
    /// was queried by UUID or no root was found. Additive: `serde(default)` keeps
    /// older payloads deserialisable; knot-server is not required to surface it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_resolution: Option<RootResolution>,
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootResolution {
    pub query: String,
    pub tier: MatchTier,          // already serde snake_case
    pub total_candidates: usize,
    pub chosen: RootCandidate,    // derives Serialize
    pub candidates: Vec<RootCandidate>, // capped at 10 for payload size
}
```

Same additive-sibling-key strategy as §4.4 of the find_callers plan ("existing consumers keep
working"). The two literal `SubgraphResult` constructions in `query_subgraph.rs` (`:105-111`,
`:282-288`) get `root_resolution: None` / `Some(...)`. knot-server's `subgraph_to_response`
ignores the new field — zero forced churn there; exposing it in the HTTP API is out of scope.

### 5.7 What deliberately does NOT change

- `SubgraphQueryParams` / `run_get_subgraph` signatures — the fix is a policy change, not a
  new input; and knot-server constructs the struct literally (`graph.rs:80`, `:228`), so a new
  field would be a breaking change for the only consumer.
- `visible_kinds` semantics, including "root survives the kind filter" (§3.3).
- The `CONTAINS` auto-injection and the four-branch roll-up in the edge query.
- knot-server's `resolve_entity` UUID path (already deterministic).
- `CURRENT_STATE_VERSION` (`src/pipeline/state.rs:33`) — no bump: purely query-side, no FQN
  or on-disk state change.

---

## 6. TDD/BDD Plan (A)

Preamble: every step is Red → Green → Refactor. Unit tests follow the crate's plain
descriptive naming (`test_<descriptive_snake_case>`), modelled on
`test_tier_ladder_order_for_plain_name` (`query.rs:834-868`). No production line before a
failing test justifies it.

### Step A-1 — kind ranking (pure)

**File:** `src/db/graph/query.rs`

#### Red

- `test_root_kind_rank_prefers_type_declarations_over_callables` → `rank(class) < rank(method)`
  and `rank(csharp_class) < rank(csharp_constructor)`
- `test_root_kind_rank_containers_rank_below_types` → `rank(csharp_namespace) > rank(csharp_class)`,
  `rank(rust_module) > rank(rust_struct)`
- `test_root_kind_rank_handles_missing_kind` → `root_kind_rank(None) == 4`
- `test_root_kind_rank_is_total_over_known_kinds` → for every variant of `EntityKind`
  (iterate `Display` output), the function returns without panic and `<= 4`

#### Green

`root_kind_rank` per §5.1 — a `match` on the `&str` with explicit arms and a `_ => 4` tail.

### Step A-2 — candidate ordering (pure)

**File:** `src/db/graph/query.rs`

#### Red

- `test_rank_root_candidates_prefers_type_over_homonym` → two candidates named `UserService`
  (`csharp_class` @ `Services/UserService.cs:12`, `csharp_constructor` @ `:18`); the class heads
  the vector. This fixture mirrors `tests/testing_files/csharp/Services/UserService.cs:11-18`
  exactly — the only constructor in the C# fixture set.
- `test_rank_root_candidates_tie_breaks_by_file_then_line_then_uuid` → two `csharp_class`
  candidates, same name, same kind, different `file_path`; lexicographically-first path wins.
  Then same path, different `start_line`; lower line wins.
- `test_rank_root_candidates_is_stable` → equal keys preserve input order (sort must be stable).

#### Green

`rank_root_candidates` per §5.2 — `sort_by_key` with a `(u8, String, i64, String)` key (use
`unwrap_or_default()`-style defaults for `Option` fields so `None` sorts deterministically).

### Step A-3 — the resolver

**File:** `src/db/graph/query_subgraph.rs` (+ tier wiring in `src/db/graph/query.rs`)

#### Red (live, `#[ignore]`, documented seeding — update the module doc comment to describe
seeding via `knot-indexer` on `tests/testing_files/csharp` with a known repo name)

- `test_resolve_subgraph_root_exact_name_prefers_class_over_constructor`
- `test_resolve_subgraph_root_exact_fqn_wins_ladder` → query `MyApp.Services.UserService`
  resolves via `ExactFqn` (today this returns an empty subgraph — §3.3)
- `test_resolve_subgraph_root_no_match_returns_none`
- `test_get_entity_subgraph_traversal_anchors_on_chosen_root` → for the
  class-vs-constructor seed, every returned node is connected to the *class* root, and the
  constructor's FQN does not appear as a traversal source unless connected via the class

#### Green

`resolve_subgraph_root` per §5.3; rewire `get_entity_subgraph` per §5.4.

### Step A-4 — traversal anchoring + deterministic truncation

**File:** `src/db/graph/query_subgraph.rs`

#### Red

- `test_get_entity_subgraph_by_name_is_deterministic` → repeated calls (10×) return identical
  `root_id` and node UUID sets (live, ignored)
- `test_get_entity_subgraph_truncation_is_stable` → with `max_nodes = 2` on a >2-node result,
  the retained UUID set is identical across runs

#### Green

Single `root_uuid`-anchored traversal (§5.4); sort-before-truncate (§5.5); delete
`root_match_clause`.

### Step A-5 — disclosure

**File:** `src/models/subgraph.rs`, `src/db/graph/query_subgraph.rs`

#### Red

- `test_subgraph_result_serialises_root_resolution_when_present`
- `test_subgraph_result_omits_root_resolution_when_none`
- `test_subgraph_result_deserialises_without_root_resolution` → older-JSON-shaped payload
  (no field) deserialises with `root_resolution == None`

#### Green

Per §5.6. Wire the `Some` construction in the name-resolution path.

### Step A-6 — validation gates

1. `cargo fmt -- --check`
2. `cargo clippy --all-targets -- -D warnings` (crate lints: `unsafe_code = deny`,
   `allow_attributes = warn` — no `#[allow]` anywhere; refactors over suppressions)
3. `cargo test` (the eight pre-existing `#[ignore]`d tests stay ignored; nothing new depends
   on a live DB in the default run)
4. Cross-layer verification through knot-server (manual/integration, documented in
   `CHANGELOG.md`): against the indexed `csharp-code-map`,
   `GET /api/repos/csharp-code-map/graph?entity=McpServer` → `root_id` names the
   `csharp_class`, `nodes` include the class's members; and
   `?entity=CodeMap.Mcp.McpServer` (FQN) returns a non-empty subgraph.

---

## 7. Blast Radius (A)

| File | Change |
|---|---|
| `src/db/graph/query.rs` | + `root_kind_rank`, `rank_root_candidates`, `RootCandidate`; `resolve_subgraph_root` on the `GraphDb` impl block; ~120 LOC + ~150 test LOC. Additive. |
| `src/db/graph/query_subgraph.rs` | Resolver wiring, UUID-anchored traversal, deletion of `root_match_clause`, sort-before-truncate, disclosure. The only behavioural file. |
| `src/models/subgraph.rs` | + `RootResolution`, one `Option` field on `SubgraphResult` (`serde(default)`). |
| knot-server | **No change required.** `subgraph_to_response` ignores the new field. Behaviour improves: `?entity=` name queries become deterministic and FQN queries start working. |
| CLI / MCP | No subgraph surface exists (verified §3.6) — no consumer to migrate. |

**Behaviour change to call out explicitly** (mirroring the v1.7.1 find_callers entry): callers
that passed a bare name and happened to receive the union of all homonyms' neighbourhoods will
now receive the chosen root's neighbourhood only — result sets get **smaller**, not just
differently rooted. This is the intended fix and must be documented as a behaviour change in
`CHANGELOG.md`, not merely a bugfix.

---

## 8. Acceptance Criteria (A)

- [ ] `?entity=<bare name>` on a repo with class+constructor homonyms resolves the **type declaration** as root, for C# and C++ alike
- [ ] Fully-qualified `?entity=<fqn>` resolves via `ExactFqn` (previously: empty subgraph)
- [ ] Returned subgraph contains only nodes connected to the chosen root (no homonym union)
- [ ] Same query, same DB → identical `root_id`, node set and edge set across calls
- [ ] Truncation retains a stable subset across calls
- [ ] `root_resolution` disclosure present in the serialized result when resolving by name; absent otherwise; older JSON without the field still deserialises
- [ ] `SubgraphQueryParams`/`run_get_subgraph` signatures unchanged; knot-server compiles unchanged
- [ ] All new logic covered by non-ignored unit tests (no live DB in `cargo test`)
- [ ] `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green; no new `#[allow]`/`#[expect]`
- [ ] `CHANGELOG.md` entry names the behaviour change explicitly

## 9. Rejected Alternatives (A)

**Order the root query in Cypher (`ORDER BY` + `LIMIT 1`).** Rejected: kind-precedence needs a
per-kind mapping in Cypher (ungreppable, untestable without a DB) or a new property written at
ingest; the Rust-side rank over ≤25 candidates is free, pure and unit-tested. Also fixes
nothing about A2.

**Restrict the root query to type-like kinds (`WHERE target.kind IN [...]`).** Rejected: turns
"prefer the type" into "fail unless a type" — a query for a function name (`McpServer.RunAsync`
method homonyms) must still resolve. Ranking degrades gracefully; filtering hard-fails.

**Reuse `resolve_reference_targets` unchanged.** Rejected: it returns a capped **set** and has
no kind notion — exactly the missing semantics. The tier *predicates* are reused; the
executor is not (§3.4).

**Anchor the traversal by re-running the name match with `ORDER BY` and skipping the root
query.** Rejected: two sources of truth for "which root"; the resolver's disclosure and the
traversal's anchor could diverge. One resolver, one winner, one anchor.

**Add `prefer_type_root: bool` to `SubgraphQueryParams`.** Rejected: breaking for knot-server
(literal struct construction), and there is no known caller that wants the old behaviour.

---

# PART B — MSBuild/NuGet Build-System Support

## 10. Current State (B)

### 10.1 Why `.csproj` is invisible today

- `src/pipeline/input.rs:11-16` `CORE_EXTENSIONS` and `:22-66` `SUPPORTED_EXTENSIONS`
  (hand-duplicated union — the spec's own trap, flagged in `csharp_support_plan.md`) contain
  no `xml`, `csproj`, `sln` or `slnx`.
- `:84-92` `BUILD_SYSTEM_NAMES` is exact-filename: `"Jenkinsfile", "pom.xml", "Cargo.toml",
  "package.json", "tsconfig.json"`. `discover_files` (`:113-204`) checks extensions first
  (`:154-178`) and falls through to the filename branch (`:180-185`); `pom.xml` is admitted
  only because `xml` is in no extension table and the exact name matches.
- `Path::extension()` on `CodeMap.Storage.Engine.csproj` returns `"csproj"`, so **the ordinary
  extension path handles `.csproj`** — the blocker asserted in `csharp_support_plan.md:262-266`
  ("requires suffix-based discovery") does not exist. §14.1 of that plan is superseded by this
  section.
- `src/pipeline/files.rs:22-47` `is_supported_file` re-implements the same predicate from
  `SUPPORTED_EXTENSIONS` + `BUILD_SYSTEM_NAMES` for incremental indexing. Discovery reads
  `CORE_EXTENSIONS` + `BUILD_SYSTEM_NAMES`. **Any new extension must be added to both
  `CORE_EXTENSIONS` and `SUPPORTED_EXTENSIONS`, or incremental re-index silently diverges from
  discovery** (existing tests: `input.rs:206-354`, five tests; none covers a
  `BUILD_SYSTEM_NAMES` filename-only match).

### 10.2 The four existing build-system parsers are hand-written, not tree-sitter

| Parser | Technique | ProjectIdentity FQN | BuildDependency name = FQN | EntityKind variants |
|---|---|---|---|---|
| `languages/xml.rs` (Maven) | `roxmltree` DOM walk, local-name matching (`child_text`, `:130-137`) — namespaces ignored | `maven:{groupId}:{artifactId}`, signature `version: {v}, build_system: maven` | `{groupId}:{artifactId}:{version}` | `ProjectIdentity`, `BuildDependency`, `BuildPlugin` |
| `languages/gradle.rs` | line scanner; artifact from **containing directory** (`:127-134`) | `gradle:{group}:{artifact}` | `{config}:{group}:{artifact}:{version}` | `ProjectIdentity`, `BuildDependency`, `BuildPlugin`, `BuildTask` |
| `languages/toml.rs` (Cargo) | `toml` crate | `cargo:{name}` (plus `CargoPackage` `cargo:{name}:{version}`) | `cargo:{dep}:{version}` | `CargoPackage`, `ProjectIdentity`, `BuildDependency`, `CargoFeature`, `WorkspaceMember` |
| `languages/json_config.rs` (npm) | `serde_json` | `npm:{name}` (scoped names intact) | `npm:{dep}:{version}` | `ProjectIdentity`, `BuildDependency`, `ConfigProperty` |

Registration is two lines per parser: `pub mod` in `languages/mod.rs` (22-line file) + a
`match ext` arm in `parser/mod.rs` (`parse_single_file`, `:253-261` carries two
`#[expect]` complexity waivers; filename-first dispatch block `:281-288` currently handles
only `Jenkinsfile`; `"xml"`/`"toml"` arms at `:521-522`; catch-all `:543-546`).

### 10.3 Cross-repo wiring — where NuGet must plug in

`src/pipeline/ingest/resolve/cross_repo.rs`:

- `link_cross_repo_dependencies` (`:8-63`): primary `ProjectIdentity` = `min_by_key` on
  repo-relative path component count (shallowest wins; pinned by cross-repo e2e Test 8);
  else `upsert_repository(..., "none", "", "", "")`. Then every `BuildDependency` →
  `match_dependency_to_repository` → `upsert_repo_dependency` (`upsert.rs:379-396`,
  `MERGE (from)-[:DEPENDS_ON]->(to)` — both ends must be indexed repos).
- `parse_build_system_from_fqn` (`:65-77`): prefix ladder `maven:`/`gradle:`/`cargo:`/`npm:`,
  else `"unknown"`.
- `parse_artifact_identity` (`:79-95`): the `"cargo"` arm returns `("", rest)` — **the flat
  `PackageId` shape NuGet needs already exists**.
- `match_dependency_to_repository` (`:119-171`) — the ordering hazards a NuGet arm must
  respect:
  - `parse_maven_style_dep` (`:173-191`) strips a `prefix:` only when the prefix **contains a
    dot**. `"nuget"` has none, so `nuget:Acme.Auth.Lib:1.0.0` would be read as
    group=`Acme.Auth.Lib`, artifact=`1.0.0` and trigger maven/gradle lookups. **The NuGet arm
    must be inserted before this branch.**
  - The Cargo fallback (`:138-147`) takes `dep_name.split(':').next()` (= `"nuget"`), passes
    the `!contains('.')` and `!= "helm"`/`!= "npm"` guards, and queries
    `find_repository_by_artifact("", "nuget", "cargo")`. **The guard must gain
    `&& crate_name != "nuget"`.**
- `upsert_repository` (`upsert.rs:344-375`): `MERGE ... SET` **overwrites** identity on
  re-index — re-indexing after this change repairs the `"none"` nodes automatically.
- `find_repository_by_artifact` (`query_repo.rs:83-114`): **exact string equality** on
  `(build_system, group_id, artifact_id)` — NuGet's case-insensitive IDs are matched by
  literal-spelling convention on both sides (npm/cargo precedent); a case-folding fix is
  deferred (§19).

### 10.4 Real-data constraints (§2 table)

The fixture repos force four must-haves:

1. **Attribute-form `PackageReference Version`** (100 % of openlogi-net's 15 deps).
2. **Central Package Management** — 78/83 of csharp-code-map's deps are version-less and
   resolve through `Directory.Packages.props`. Without CPM, 100 % of that repo's dependencies
   are `version: unknown`.
3. **Identity fallback chain** — openlogi-net has zero `PackageId`; identity must fall back to
   `AssemblyName` → csproj file stem (the `gradle.rs:127-134` containing-directory precedent).
4. **`ProjectReference` recognised and skipped** — 22 + 50 elements; emitting them as
   cross-repo-capable `BuildDependency` is v2 (they point inside the same repo, and
   `match_dependency_to_repository` would no-op anyway, but the parser must not mis-read them
   as package refs).

`roxmltree = "0.21"` (resolved 0.21.1) is already a dependency for Maven — **no new crates**.
`quick-xml` is not present. The 9 BOM-prefixed `.csproj` files of openlogi-net mandate a
defensive UTF-8 BOM strip before parsing (two research passes disagreed on whether
roxmltree 0.21.1 tolerates a leading BOM; the spec therefore mandates strip-plus-test rather
than relying on either claim). The `child_text` local-name technique from `xml.rs` makes
MSBuild `xmlns` a non-issue.

---

## 11. Design (B)

### 11.1 Wire format (follows the npm/cargo conventions)

| Entity | FQN | name | signature |
|---|---|---|---|
| `ProjectIdentity` (explicit `PackageId`) | `nuget:{PackageId}` | `{PackageId}` | `version: {v}, build_system: nuget, identity: package_id` |
| `ProjectIdentity` (fallback) | `nuget:{AssemblyName\|file stem}` | same | `version: {v}, build_system: nuget` |
| `BuildDependency` | `nuget:{PackageId}:{Version}` | same | `None` (NuGet has no scopes; docstring `NuGet dependency: {name}`) |

Language tag on emitted entities: the file's own (`"csproj"`, `"props"`, `"sln"`) —
repo `primary_language` is the modal `e.language` over all entities
(`query_repo.rs:168-188`) and is dominated by thousands of `csharp` source entities, so build
files cannot skew it. The `identity: package_id` marker is inert for
`parse_version_from_signature` (`:109-117` splits on `,` and reads the `version: ` token).

Version resolution for a project: `<Version>` property in the `.csproj` → `"unknown"`.
(Inherited `Directory.Build.props` versions: deferred, §19.)

### 11.2 Discovery (v1 surface)

| Where | Change |
|---|---|
| `input.rs` `CORE_EXTENSIONS` **and** `SUPPORTED_EXTENSIONS` | + `"csproj"` (extension-based, like any source; always indexed regardless of `--include-config-files`) |
| `input.rs` `BUILD_SYSTEM_NAMES` | + `"Directory.Packages.props"` (exact filename; its `props` extension is in no table, so the filename branch admits it) |
| `files.rs` | no edit — reads the same constants; add a sync-guard unit test (Step B-1) |
| Deferred | `"sln"`, `"slnx"`, `"Directory.Build.props"`, `"packages.config"` (§19) |

### 11.3 Parser — new `src/pipeline/parser/languages/msbuild.rs`

Mirrors `xml.rs` structure, `roxmltree`, BOM strip, local-name helpers shared/copied from
`xml.rs:130-137`. One public entry per admitted file kind:

- `extract_entities_csproj(source, file_path, repo_name)` →
  `ProjectIdentity` (chain: `<PackageId>` → `<AssemblyName>` → file stem; version
  `<Version>` → `"unknown"`; marker per §11.1) + one `BuildDependency` per
  `<PackageReference Include="X" Version="Y"/>`; `Version` attribute absent → CPM lookup
  (§11.4) → `"unknown"`; `ProjectReference` skipped (docstring-level debug, no entity).
- `extract_entities_props(source, file_path, repo_name)` → no entities in v1; exists so the
  dispatcher has a target and the CPM map builder can reuse the element-walk (keeps
  `parse_single_file` arms total).

Registration: `pub mod msbuild;` in `languages/mod.rs`; dispatch in `parser/mod.rs`:
`"csproj"` arm next to `"xml"` (`:521`), and a filename-first block next to `Jenkinsfile`
(`:281-288`) for `Directory.Packages.props`.

### 11.4 CPM resolution — the one genuinely new mechanism

Version-less `PackageReference` requires reading a *different* file. `ParseConfig` already
carries `repo_root` (`parser/mod.rs:53`) — no plumbing change:

1. From the `.csproj`'s repo-relative dir, walk up to `repo_root` looking for
   `Directory.Packages.props` (nearest ancestor wins — the MSBuild inheritance semantics
   simplified deliberately, documented in the module doc).
2. Parse once per file path, cache process-wide: `OnceLock<Mutex<HashMap<PathBuf,
   Arc<CpmMap>>>>` where `CpmMap = HashMap<String, String>` (`Include` → `Version`), because
   `parse_single_file` runs under Rayon (23 `.csproj` in csharp-code-map must not re-read the
   props file 23 times). Precedent for repo-relative ancestor walking:
   `rust_crate_discovery.rs:35-54`. Cache lifetime = process; the indexer is short-lived and
   build files do not change mid-run (documented).
3. Miss → `"unknown"` (npm/cargo precedent for unresolvable versions).

Refactor alternative (rejected for v1, §15): build the CPM map once in the prepare stage and
thread it through `ParseConfig` — cleaner but touches shared pipeline plumbing.

### 11.5 Cross-repo wiring — three small edits in `cross_repo.rs` (the shared-code surface)

1. `parse_build_system_from_fqn`: + `else if fqn.starts_with("nuget:") { "nuget" }`.
2. `parse_artifact_identity`: + `"nuget" => ("", rest)` (or fold into the `"cargo"` arm —
   spec the explicit arm for readability).
3. Primary selection (`link_cross_repo_dependencies` `:18-44`): among `ProjectIdentity`
   candidates whose FQN starts with `"nuget:"`, prefer those whose signature carries the
   `identity: package_id` marker (§11.1); fall back to the existing global
   shallowest-`min_by_key` when no marked candidate exists. Implementation: partition
   candidates into marked/unmarked; if any marked, `min_by_key` over the marked subset only.
   Build-system-agnostic by construction — no other parser emits the marker, so Maven/Gradle/
   Cargo/npm selection is bit-for-bit unchanged (pinned by cross-repo e2e Test 8).
   Rationale: `csharp-code-map`'s shallowest `.csproj` is `src/CodeMap.Core/...` (depth 2),
   which ties with the `PackageId`-bearing `CodeMap.Daemon.csproj` (also depth 2) — pure
   depth would pick `CodeMap.Core` by alphabetical accident and break dependency matching
   against the published `codemap-mcp`.
4. `match_dependency_to_repository`: insert the NuGet arm **before** the
   `parse_maven_style_dep` branch (§10.3 ordering hazard):

```rust
    if let Some(pkg) = dep_name.strip_prefix("nuget:") {
        let name = pkg.split(':').next().unwrap_or(pkg);
        if let Some(repo) = graph_db
            .find_repository_by_artifact("", name, "nuget")
            .await?
        {
            return Ok(Some(repo));
        }
    }
```

   and add `&& crate_name != "nuget"` to the Cargo fallback guard as belt-and-braces.

Known consequence (documented, accepted for v1): `openlogi-net` has no `PackageId`, so its
identity is the alphabetically-first depth-2 stem, `nuget:OpenLogi.Agent`. Harmless for
cross-repo (nothing consumes it as a package); the principled fix is solution-level identity,
deferred with its selection-rule decision (§19).

### 11.6 State version

No `CURRENT_STATE_VERSION` bump: the change is additive (new file kinds → new entities; no
existing FQN shape moves). Existing repos gain the identity on their next index; release notes
recommend `knot-indexer --clean` for immediate effect. `upsert_repository`'s `MERGE ... SET`
repairs the previously-written `"none"` repositories on re-index (§10.3).

---

## 12. TDD/BDD Plan (B)

### Step B-1 — discovery

**File:** `src/pipeline/input.rs` (+ sync-guard test targetting `files.rs`)

#### Red

- `test_discover_files_csproj_always_indexed` → a repo with `src/App/App.csproj`,
  `Directory.Packages.props`, `src/App/Program.cs`: all three discovered with
  `include_config_files = false`
- `test_discover_files_csproj_and_props_in_both_extension_and_name_paths` → asserts
  `discover_files` and `is_supported_file` agree on `x.csproj` and
  `Directory.Packages.props` (the hand-duplicated-tables trap, §10.1)
- `test_discover_files_props_not_swept_by_extension` → `Some.props` (not
  `Directory.Packages.props`) is **not** discovered

#### Green

§11.2 table edits.

### Step B-2 — `.csproj` parser core

**File:** `src/pipeline/parser/languages/msbuild.rs` (new)

#### Red (unit, no DB — pattern per `xml.rs:139-394`)

- `test_extract_csproj_package_reference_attribute_version` → dependency
  `nuget:Tomlyn:0.17.0` from the real `OpenLogi.Core.csproj` shape
- `test_extract_csproj_identity_from_package_id` → FQN `nuget:codemap-mcp`, signature
  contains `identity: package_id` (from the real `CodeMap.Daemon.csproj` shape)
- `test_extract_csproj_identity_falls_back_to_assembly_name`
- `test_extract_csproj_identity_falls_back_to_file_stem`
- `test_extract_csproj_version_property`
- `test_extract_csproj_project_reference_skipped`
- `test_extract_csproj_tolerates_utf8_bom` → the exact bytes of a BOM-prefixed fixture
  (`\xef\xbb\xbf<Project ...`) parse and yield the identity (§10.4)
- `test_extract_csproj_empty_project_yields_nothing`
- `test_extract_csproj_xmlns_and_conditions_ignored` → namespaced legacy project file parses
  via local names; `Condition` attributes don't affect extraction
- `test_extract_csproj_versionless_without_props_is_unknown`

#### Green

§11.3.

### Step B-3 — CPM

**File:** `msbuild.rs`

#### Red

- `test_extract_csproj_cpm_resolves_version_from_props` → version-less reference + a
  `Directory.Packages.props` string → `nuget:LibGit2Sharp:0.30.0` (real
  csharp-code-map shapes, both files fed as raw strings with a tempdir layout)
- `test_extract_csproj_cpm_nearest_ancestor_wins` → props in the project dir shadows a root
  props
- `test_cpm_cache_parses_props_once` → counter/file-mtime probe proves a single read for two
  projects sharing a props file

#### Green

§11.4.

### Step B-4 — cross-repo wiring

**File:** `src/pipeline/ingest/resolve/cross_repo.rs`

#### Red (unit, extending the existing `:193-282` block)

- `test_parse_build_system_nuget`
- `test_parse_artifact_identity_nuget_flat`
- `test_match_dependency_nuget_precedes_maven_style` → `match_dependency_to_repository` is
  private; test the ordering through a seam — either make the matcher's per-format parse
  steps pure helpers and assert `parse_maven_style_dep("nuget:Acme.Auth.Lib:1.0.0")`
  **would** misfire (regression documentation) plus an integration-style live test, or test
  via the e2e suite; spec the pure-helper route
- `test_primary_selection_prefers_package_id_marker` → three identities
  (`nuget:CodeMap.Core` depth 2 unmarked, `nuget:codemap-mcp` depth 2 marked,
  `nuget:Something` depth 1 unmarked) → the marked one wins
- `test_primary_selection_falls_back_to_shallowest_without_marker` → Maven fixtures behave
  exactly as today (marker absent)

#### Green

§11.5.

### Step B-5 — build-systems e2e extension

**File:** `tests/run_build_systems_e2e.sh` (379 lines, 14 assertions; env: Neo4j
`:17687`, Qdrant `:16334`, `REPO_NAME="build_systems_e2e_test_repo"`, fixtures at
`tests/testing_files/build_systems/` — one file per build system at repo-root depth)

#### Red

New fixtures in `tests/testing_files/build_systems/`:
`App.csproj` (attribute-form versions, BOM-prefixed bytes — mirrors openlogi-net),
`ClassLib.csproj` (version-less — mirrors csharp-code-map), `Directory.Packages.props`
(real `PackageVersion` shape), `Service.cs` (one source file so `primary_language` resolves).

New assertions (same inline grep helper pattern, `exit 1` on fail, ~6 tests):
search finds `Tomlyn` and `LibGit2Sharp`; `explore_file App.csproj` lists the dependency
entities; `explore_file ClassLib.csproj` shows the CPM-resolved version; cypher-shell
`MATCH (r:Repository {name: $REPO_NAME}) RETURN r.build_system` → `nuget`.

#### Green

Nothing beyond the parser work of B-2/B-3 — this step verifies the pipeline end to end.

### Step B-6 — cross-repo e2e extension

**File:** `tests/run_cross_repo_dep_e2e.sh` (682 lines; synthesises repo pairs via heredocs —
Maven pair `:153-228`, Cargo pair `:432-458`; Test 8 `:546-620` pins shallowest-path
selection with direct cypher-shell assertions `:636-672`)

#### Red

Third pair, modelled on the Cargo pair:

- `$TMP_NUGET_LIB_DIR/Acme.Auth.Lib.csproj`:
  `<PackageId>Acme.Auth.Lib</PackageId><Version>1.0.0</Version>` + `AuthService.cs`
- `$TMP_NUGET_CLIENT_DIR/ClientApp.csproj`:
  `<PackageReference Include="Acme.Auth.Lib" Version="1.0.0" />` + `Program.cs`

Assertions: `knot deps client-app-repo` → `Acme.Auth.Lib`; `knot deps --reverse` → client;
MCP `list_repo_dependencies` → both directions; cypher-shell: `build_system = "nuget"`,
`artifact_id = "Acme.Auth.Lib"`, `DEPENDS_ON` count ≥ 1.

#### Green

B-4.

### Step B-7 — validation gates and docs

1. Gate battery per `AGENTS.md:323-327`: `cargo fmt -- --check`, `cargo clippy --all-targets
   -- -D warnings`, `cargo test`; then `./tests/run_all_e2e_fast.sh` (suites already
   registered in the `SUITES` array — no new registration needed; `run_build_systems_e2e.sh`
   and `run_cross_repo_dep_e2e.sh` are entries 10 and 14).
2. `README.md`: build-systems list + C# section gains MSBuild/NuGet.
3. `CHANGELOG.md`: entry in the `## vX.Y.Z — Title` house style (`Feat(parser)` /
   `Feat(ingest)` / `Docs` bullets), noting the `--clean` re-index recommendation.
4. `src/mcp_tools/list_repo_dependencies.rs:42` doc comment: add `nuget` to the supported
   build-system enumeration.
5. `docs/specs/csharp_support_plan.md` §14.1: mark superseded by this plan (its
   "suffix-based discovery" blocker premise is wrong, §10.1).

---

## 13. Blast Radius (B)

| File | Change | Risk |
|---|---|---|
| `src/pipeline/input.rs` | Two constant tables + one filename entry | **Shared** — low; five existing tests + two new sync-guards |
| `src/pipeline/parser/languages/msbuild.rs` | New module (~350 LOC impl + ~250 test) | **Additive** — none |
| `src/pipeline/parser/mod.rs` | One ext arm + one filename block | Additive; `parse_single_file` already carries two `#[expect]` waivers, none new |
| `src/pipeline/parser/languages/mod.rs` | One `pub mod` | Trivial |
| `src/pipeline/ingest/resolve/cross_repo.rs` | Two prefix arms + selection partition + one guard clause | **Shared — the highest-regression-risk edit of Part B**; covered by existing cross-repo e2e (incl. Test 8) + new unit tests |
| `tests/run_build_systems_e2e.sh`, `tests/run_cross_repo_dep_e2e.sh` | Fixtures + ~12 assertions | Additive |
| Docs | README, CHANGELOG, MCP doc comment, plan supersession | — |

**Nothing existing changes behaviour** except the intended one: C# repos move from
`build_system: "none"` to `"nuget"` on re-index.

## 14. Acceptance Criteria (B)

- [ ] `openlogi-net` re-indexed → `Repository { build_system: "nuget", artifact_id: "OpenLogi.Agent" }` (documented stem-fallback identity)
- [ ] `csharp-code-map` re-indexed → `Repository { build_system: "nuget", artifact_id: "codemap-mcp", version: "2.8.1" }` — the `PackageId` marker outranks depth ties
- [ ] All 83 of csharp-code-map's `PackageReference`s yield `BuildDependency` entities; the 78 version-less ones resolve through `Directory.Packages.props` (19 known versions) or `"unknown"`
- [ ] Zero `BuildDependency` entities from `ProjectReference` elements
- [ ] BOM-prefixed `.csproj` files parse (unit test pins it)
- [ ] A NuGet lib/client repo pair produces `DEPENDS_ON`; `knot deps`, `--reverse`, and MCP `list_repo_dependencies` report it
- [ ] Maven/Gradle/Cargo/npm primary selection unchanged (cross-repo e2e Test 8 stays green)
- [ ] `props`/`config` extensions are not swept into discovery (`Some.props` stays invisible)
- [ ] Gates: fmt / clippy `-D warnings` / `cargo test` / `run_all_e2e_fast.sh` green
- [ ] No `CURRENT_STATE_VERSION` bump; release notes recommend `--clean` re-index

## 15. Rejected Alternatives (B)

**Follow `csharp_support_plan.md` §14.1's premise and add suffix-based discovery.** Rejected:
the premise is factually wrong — `Path::extension()` yields `"csproj"`, the ordinary
extension branch suffices (§10.1). The plan's own text is corrected instead.

**tree-sitter grammar for MSBuild XML.** Rejected: all four existing build-system parsers are
hand-written; `.csproj` is plain XML and `roxmltree` (already a dependency) is the established
tool.

**Emit `ProjectReference` as `BuildDependency` for intra-repo linking.** Rejected for v1:
both ends live in the same repo; `match_dependency_to_repository` filters self-matches
(`matched_repo != cfg.repo_name`), so the edges would be dead weight. Revisit if multi-repo
solution-sharding becomes a use case.

**Case-insensitive NuGet matching (`toLower` on both sides of
`find_repository_by_artifact`).** Deferred: npm/cargo precedent is exact-match; changing the
lookup semantics is a shared-code risk out of proportion for v1. Both fixture repos use
consistent casing.

**Thread a pre-built CPM map through `ParseConfig` instead of caching.** Rejected for v1:
cleaner ownership but touches the shared parse plumbing and every parser's call sites; the
process-local cache is contained and the indexer is short-lived (§11.4).

**Parse `.sln`/`.slnx` for solution-level identity in v1.** Deferred with a documented
decision point (§19): the shallowest-path selection rule would need a priority scheme beyond
depth, which is a `cross_repo.rs` semantic change deserving its own plan.

---

## 16. Shared Release Mechanics

- Version: knot `Cargo.toml:3` reads `1.6.2` while `CHANGELOG.md`'s head entry is
  `v1.7.1` (an in-flight inconsistency in the working tree). Version both parts as the next
  entry after the CHANGELOG head and align `Cargo.toml` at release time — maintainer's call.
- Part A's CHANGELOG entry must use the bold **Behavior Change** marker (§7), matching the
  v1.7.1 style.
- Order of landing: Part A and Part B are independent; Part A is ~1 day, Part B ~3-4 days
  including e2e work. If split into separate PRs, Part B's Step B-4 is the only shared-file
  edit either part makes, and it does not collide with Part A.

## 17. Risks and Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| NuGet arm ordering in `match_dependency_to_repository` misfires against maven-style parsing (`nuget:` prefix has no dot → stripped) | Certain if misplaced | Arm inserted before the maven branch; unit test `test_match_dependency_nuget_precedes_maven_style` (B-4) |
| Cargo fallback queries `find_repository_by_artifact("", "nuget", "cargo")` | Certain without guard | `crate_name != "nuget"` guard + e2e |
| Hand-duplicated extension tables diverge (incremental re-index drops `.csproj`) | Medium | Sync-guard unit test in B-1 (§10.1 trap) |
| roxmltree/BOM behaviour differs across versions | Low | Defensive strip + dedicated unit test, independent of the library's behaviour (§10.4) |
| Selection-partition in `cross_repo.rs` perturbs existing build systems | Low | Marker is emitted only by the new parser; Test 8 of the cross-repo e2e pins the old rule; fallback path is the unmodified `min_by_key` |
| CPM cache returns stale data within one process | Very low | Cache keyed by resolved props path; build files immutable during an index run (documented) |
| Part A shrinks result sets for callers relying on the homonym union | Low (no known caller) | Declared behaviour change in CHANGELOG; the union is a defect, not a feature (§7) |

## 18. Deliverables Checklist

- [ ] Part A: `root_kind_rank`, `rank_root_candidates`, `RootCandidate`, `resolve_subgraph_root`, UUID-anchored traversal, deterministic truncation, `RootResolution` disclosure
- [ ] Part A: ~10 unit tests + 4 live `#[ignore]` tests (seeding documented)
- [ ] Part B: `input.rs`/`files.rs` discovery edits + sync-guard tests
- [ ] Part B: `languages/msbuild.rs` (csproj + props entries, BOM strip, CPM cache) + ~16 unit tests
- [ ] Part B: `cross_repo.rs` nuget wiring + 5 unit tests
- [ ] Part B: build-systems fixtures (4 files) + ~6 e2e assertions; NuGet cross-repo pair + ~7 assertions
- [ ] Docs: README, CHANGELOG (both parts), `list_repo_dependencies.rs:42`, `csharp_support_plan.md` §14.1 supersession note
- [ ] Full gate matrix green for both parts (§6 Step A-6, §12 Step B-7)

## 19. Out of Scope — Known Adjacent Defects

1. **Solution-level identity (`.sln`/`.slnx`).** Would fix `openlogi-net`'s arbitrary
   stem-based identity properly, but requires a selection-rule change in
   `link_cross_repo_dependencies` beyond the marker partition (a solution file at depth 0
   would always outrank a `PackageId` project at depth 2 — `csharp-code-map` would become
   `nuget:CodeMap` instead of `nuget:codemap-mcp`). Needs its own decision + plan; v1 keeps
   the marker-based rule, which already produces the correct identity for the repo that
   publishes a package.
2. **C++ constructor homonyms** (`queries/cpp.scm:6-8`) — same defect class as C#; fixed
   *de facto* by Part A's ranking (`cpp_class` beats `cpp_method` at equal name) but deserves
   an explicit e2e assertion in the C++ suite when convenient.
3. **`kotlin_companion_object` display-bucket typo** — `explore_file.rs:185` maps
   `"kotlin_companion"`, which never matches the wire format `"kotlin_companion_object"`
   (`entity.rs:161`). One-line fix, unrelated to this plan.
4. **Case-insensitive NuGet IDs** — deferred (§15).
5. **Inherited `<Version>` from `Directory.Build.props`** — deferred; both fixture repos
   carry versions in the winning project file.
