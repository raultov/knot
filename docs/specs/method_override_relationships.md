# Method-Level OVERRIDES Relationships (JVM: Java, Kotlin, Groovy)

Status: **Planned** — not implemented. This document is the TDD/BDD spec.
Target version: v1.6.0
Author context: raised while indexing the `nextflow` Groovy project.
Revision: **v2 (2026-07-21)** — post-review corrections. Two design decisions were
revised (hierarchy traversal and query directionality, see §2), the method→type
grouping strategy was replaced (§5.2), the query design was rewritten (§5.4), and
new BDD scenarios (D2, J, K, M, Q4) plus a pre-implementation verification
checklist (§7 step 0) were added.

---

## 1. Problem statement

knot models inheritance **only at the type level**. Given:

```groovy
// modules/nf-commons/src/main/nextflow/ISession.groovy
interface ISession {
    UUID getUniqueId()
}

// modules/nextflow/src/main/groovy/nextflow/Session.groovy
class Session implements ISession {
    UUID getUniqueId() { ... }
}
```

knot creates exactly one inheritance edge:

```
Session (class) -[IMPLEMENTS]-> ISession (interface)
```

The methods `Session.getUniqueId` and `ISession.getUniqueId` are **two independent
entities with no edge between them**. The only relationship the method
`ISession.getUniqueId` has is its container link:

```
ISession -[CONTAINS]-> getUniqueId
```

### Observed symptom

Running `find_callers("getUniqueId")` (or inspecting the method's relationships)
returns only the containing interface `ISession` (`groovy_interface`, line 26) via
`CONTAINS`. The implementations in child classes (`Session.getUniqueId`, etc.) never
appear, because **no method→method edge exists**.

### Root-cause evidence (code references)

- `src/pipeline/parser/languages/groovy.rs:111` — `extract_inheritance_intents`
  attaches `Extends`/`Implements` intents to the **type** entity only.
- `src/pipeline/ingest/resolve/mod.rs:189-212` — `ReferenceIntent::Extends`/`Implements`
  resolve to `RelationshipType::Extends`/`Implements` at the type level. No method
  granularity anywhere in `resolve/`.
- `src/db/graph/upsert.rs:33` — `build_contains_auto_link_cypher` creates
  `(container)-[:CONTAINS]->(method)` via `enclosing_class_fqn`. This is the single
  edge observed.
- `src/db/graph/query.rs:124-204` — `find_references` matches incoming
  `CALLS/EXTENDS/IMPLEMENTS/REFERENCES` edges whose **target name = "getUniqueId"**.
  No method→method edge type exists, so implementations are never returned. Real call
  sites (`session.getUniqueId()`) are additionally dropped as ambiguous (multiple
  same-named candidates, no receiver-type inference for Groovy).

---

## 2. Goal

Introduce a **new, additive** relationship `OVERRIDES` that links an overriding /
implementing method in a subtype to the corresponding method declared in a
supertype (interface or superclass), built at **index time**, so that
`find_callers` / `find_references` surfaces the relationship **bidirectionally**:

- Query the interface/superclass method → see all implementing/overriding methods.
- Query an implementation → see the declared method it overrides/implements.

### Design decisions

| Decision | Choice |
| --- | --- |
| Approach | **A** — real edges created at index time (resolution pass). |
| Relationship name | **`OVERRIDES`** (precise for JVM; distinct from type-level `IMPLEMENTS`). |
| Hierarchy traversal | **Nearest-declaration linking** *(revised in v2)*: at index time each method links only to the **nearest** declaration(s) of the same name found walking up its type hierarchy (walking straight through supertypes that do not declare the method). Full transitive visibility is resolved **at query time** with variable-length Cypher (`-[:OVERRIDES*1..]->`). Rationale: materializing the full transitive closure densifies the graph with O(depth²) redundant edges and is an anti-pattern in graph DBs; Neo4j resolves transitivity natively. Diamond/cycle safety is preserved (index-time `visited` guard + query-time `DISTINCT`). |
| Directionality (in queries) | **Two directed buckets** *(revised in v2)* instead of one undirected match: `overridden_by` (incoming edges → implementations) and `overrides` (outgoing edges → declarations). An undirected `-[:OVERRIDES]-` match mixes ancestors and descendants in one bucket and, because both endpoints of an `OVERRIDES` edge share the same method name, returns duplicate rows and the queried node itself. |
| Method matching | **By name only** (Groovy ad-hoc parser often lacks full signatures). |
| Language scope | **JVM only** (Java, Kotlin, Groovy) via kind allowlist **plus a file-extension guard** (§4). |
| Edge direction | `subtype.method -[OVERRIDES]-> supertype.method`. |

### Non-goals (this iteration)

- Cross-repo override linking (supertype in a dependency repo). Intra-repo only.
- Signature/arity-based disambiguation of overloads.
- Fan-out of ambiguous calls to implementations (documented as a follow-up).
- Non-JVM languages (Rust/Python/C/C++/TS). Must remain provably unaffected.
- **Incremental-linking completeness** (see §9): edges are created only when the
  subtype *and* the supertype's methods are in the same indexing batch. A full
  re-index produces complete coverage; mixed incremental runs may miss edges.
- Modifier-aware filtering (static / private). Modifiers are not persisted on JVM
  entities today, so they cannot be excluded at resolve time (see §6 Scenario K).
- Stale-edge garbage collection on file rename/delete (`MERGE` never deletes).
  Pre-existing for all relationship types; not introduced nor fixed here.

---

## 3. Guarantees for "does not affect other languages"

1. **Additive only.** The pass only *adds* `OVERRIDES` edges. It never mutates or
   removes existing entities or edges. Existing indexes for any language cannot regress.
2. **JVM guard (kind allowlist + file extension).** The pass skips any entity whose
   `file_path` does not end in a JVM source extension (`.java`, `.kt`, `.kts`,
   `.groovy`, `.gvy`, `.gradle`). This matters because the generic kinds
   `Class`/`Interface`/`Method` are shared with TypeScript — a kind-only allowlist
   would leak `OVERRIDES` edges into TS. Note that `ResolutionEntity`
   (`src/models/entity.rs:332-347`) has **no `language` field**, so the guard must
   be extension-based. Every other language yields **zero** `OVERRIDES` edges by
   construction — enforced by a dedicated test (§6, Scenario N, which includes
   TypeScript explicitly).
3. **Serialization compatibility.** `RelationshipType` derives serde
   `Serialize`/`Deserialize` with no `FromStr` inverse parser; adding an enum
   variant is backward compatible — existing data simply lacks `OVERRIDES`. No
   migration.
4. **Single exhaustive match.** The only exhaustive `match` on `RelationshipType`
   is the `Display` impl (`src/models/relationship.rs:133-150`). Upsert is generic
   via `to_string()` (`src/db/graph/upsert.rs:291-317`). Minimal, low-risk surface.

---

## 4. Kind allowlists & language guard

```text
Language guard (applied FIRST, to every entity):
  file_path ends with one of: .java .kt .kts .groovy .gvy .gradle

JVM method-like kinds:
  EntityKind::Method          (Java)
  EntityKind::KotlinMethod
  EntityKind::GroovyMethod

JVM type-like kinds:
  EntityKind::Class, EntityKind::Interface, EntityKind::Enum      (Java)
  EntityKind::KotlinClass, EntityKind::KotlinInterface, EntityKind::KotlinEnum
  EntityKind::KotlinObject, EntityKind::KotlinCompanionObject
  EntityKind::GroovyClass, EntityKind::GroovyInterface, EntityKind::GroovyTrait,
  EntityKind::GroovyEnum
```

Changes vs v1 of this spec:

- `EntityKind::KotlinFunction` **removed** from the method-like list: top-level and
  extension functions are never override targets/sources (extension functions are
  statically dispatched and cannot override).
- `KotlinObject`, `KotlinCompanionObject`, `Enum`, `KotlinEnum`, `GroovyEnum`
  **added** to the type-like list: Kotlin objects (including anonymous
  `object : Iface { }` literals, which the parser already emits with `Implements`
  intents, `kotlin.rs:602-634`) and enums can implement interfaces and override
  methods.
- The TypeScript caveat from v1 is **resolved**: the extension guard makes TS
  ineligible regardless of shared kinds. Scenario N tests TS explicitly.

**Constructor exclusion:** any method-like entity whose `name` equals its enclosing
type's `name` (Java/Groovy constructor heuristic) or equals `<init>` is excluded.
Groovy already never emits constructor call sites as method entities
(`groovy.rs:979`), but Java/Kotlin constructor extraction must be confirmed in
step 0 (§7); the name-based heuristic covers all parsers regardless.

---

## 5. Architecture / touch points

### 5.1 New relationship variant
`src/models/relationship.rs`
- Add `RelationshipType::Overrides` with doc comment (covers interface-impl and
  superclass-override).
- Add `Display` arm → `"OVERRIDES"`.

### 5.2 New resolution pass
`src/pipeline/ingest/resolve/overrides.rs` (new module; register in
`resolve/mod.rs`).

Public entry:
```rust
/// Adds `subtype.method -[Overrides]-> supertype.method` edges for JVM entities.
/// Runs AFTER type-level Extends/Implements edges are resolved and BEFORE upsert.
/// Pure in-memory, batch-local, additive.
pub(crate) fn link_method_overrides(entities: &mut [ResolutionEntity]);
```

Invocation: inside `resolve_reference_intents_with_context`
(`src/pipeline/ingest/resolve/mod.rs`), immediately after the parallel
`par_iter_mut` intent-resolution block completes (~line 292) and before the caller
runs `upsert_relationships` (`resolve/mod.rs:58-61`). The pass operates on the
**batch only**; the globally hydrated `fqn_to_uuid`/`name_to_uuids` maps are *not*
used in this iteration (that is precisely the incremental limitation, §9).

**Method→type grouping strategy (revised in v2).** v1 proposed grouping methods by
resolving `enclosing_class_fqn`, with an optional fallback to
`enclosing_class` + package. Verification showed this is unworkable:
`enclosing_class_fqn` is populated **only by the Rust parser**
(`rust/fqn.rs:134`); every JVM entity leaves it `None` (`entity.rs:433`), and
`ResolutionEntity` has no `package` field. Instead, group by **FQN arithmetic**:

```text
enclosing_type_fqn = method.fqn minus its final ".<name>" segment
```

Confirmed FQN formats (all satisfy `method_fqn == type_fqn + "." + method_name`):

| Language | Type FQN | Method FQN | Source |
| --- | --- | --- | --- |
| Groovy | `pkg.Class` | `pkg.Class.method` | `build_fqn`, `groovy.rs:462-468` |
| Java | `Class` / `Outer.Inner` | `Class.method` / `Outer.Inner.method` | `compute_fqn_and_context`, `context.rs:100-107` (no package prefix — pre-existing collision caveat, see §9) |
| Kotlin | `Class` / `Obj` | `Class.method` / `Obj.method`; anonymous objects: `Foo.bar.<anonymous@line>` | `kotlin.rs` tests at :928-974 |

If stripping the last segment of a method FQN does not resolve to a type entity in
the batch, the method is **skipped** (no crash) — this subsumes v1's Scenario F.

**Algorithm (two phases — required by the borrow checker):**

Phase 1 (immutable borrows only):
1. Build `fqn -> uuid` and `uuid -> index` maps for all batch entities.
2. `supertypes: HashMap<Uuid, Vec<Uuid>>` from each **type-like** entity's
   already-resolved `relationships`, keeping only `Extends`/`Implements` targets.
3. `methods_by_type: HashMap<Uuid, Vec<(String /*name*/, Uuid, usize /*idx*/)>>`:
   for each **method-like** entity (language guard + kind allowlist + constructor
   exclusion), strip its last FQN segment, look the result up in `fqn -> uuid`
   (type entities only — never look methods up by FQN, since overloads share FQN),
   and group under that type's uuid.
4. Compute edges into a standalone `Vec<(usize /*method idx*/, Uuid /*decl uuid*/)>`
   plus a dedup `HashSet<(usize, Uuid)>`. For each type `T` and each of its methods
   `m`, run a **nearest-declaration BFS** over `supertypes`:
   ```
   frontier = supertypes[T]; visited = {T}
   while frontier not empty:
       next = []
       for S in frontier:
           if S in visited: continue
           visited.add(S)
           if methods_by_type[S] declares m.name:
               for each declaration d: emit (m.idx → d.uuid)   // nearest found:
                                                                // do NOT expand S
           else:
               next.extend(supertypes.get(S, []))              // walk through
       frontier = next
   ```
   - Supertypes whose uuid is not in `uuid -> index` (resolved from Neo4j but not
     re-parsed in this batch) are skipped without panic.
   - Cycles and diamonds are handled by `visited`; the same declaration reached via
     two paths is emitted once via the dedup set.
   - JVM entities have no alias redirect (`alias_map` is TS-specific); none applied.

Phase 2 (mutable):
5. For each `(idx, decl_uuid)` in the edge vec, push
   `(decl_uuid, RelationshipType::Overrides)` onto `entities[idx].relationships`
   (per-method dedup `HashSet<(Uuid, RelationshipType)>` guards against duplicates).

Helpers (centralized in `overrides.rs`):
```rust
fn is_jvm_file(file_path: &str) -> bool;
fn is_jvm_method_like(kind: &EntityKind) -> bool;
fn is_jvm_type_like(kind: &EntityKind) -> bool;
fn enclosing_type_fqn<'a>(method_fqn: &'a str, method_name: &str) -> Option<&'a str>;
```

### 5.3 Upsert
No changes. `upsert_relationships` groups by `rel_type.to_string()` and emits generic
Cypher (`src/db/graph/upsert.rs:278-317`).

### 5.4 Query — two directed buckets (rewritten in v2)
`src/db/graph/query.rs` (`find_references`)

The existing loop (`query.rs:137-201`) is homogeneous and hardcodes
`-[:{rel_label}]->`; **do not** add `OVERRIDES` to that `rel_types` vec. Instead,
after the loop, run two explicit variable-length queries and store their results
under two new keys in the result JSON (`"overrides": []` and `"overridden_by": []`
added to the empty-result init at `query.rs:129-134`):

```cypher
// Bucket "overridden_by": implementations/descendants of the queried method
MATCH (entity:Entity)-[:OVERRIDES*1..]->(target:Entity)
WHERE target.repo_name = $repo_name          // repo clause only when repo filter present
  AND (target.name = $name OR target.fqn = $name OR target.fqn CONTAINS $name
       OR (target.name + COALESCE(target.signature, '')) CONTAINS $name)
  AND entity.uuid <> target.uuid             // defensive anti-self (pathological cycles)
RETURN DISTINCT entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature,
       target.name AS target_name, target.fqn AS target_fqn,
       target.file_path AS target_file_path,
       target.start_line AS target_start_line, target.signature AS target_signature

// Bucket "overrides": declarations/ancestors the queried method overrides
MATCH (entity:Entity)-[:OVERRIDES*1..]->(target:Entity)
WHERE entity.repo_name = $repo_name          // predicate on the SOURCE this time
  AND (entity.name = $name OR entity.fqn = $name OR entity.fqn CONTAINS $name
       OR (entity.name + COALESCE(entity.signature, '')) CONTAINS $name)
  AND entity.uuid <> target.uuid
RETURN DISTINCT target.name AS "entity.name", target.kind AS "entity.kind",
       target.file_path AS "entity.file_path", target.start_line AS "entity.start_line",
       target.signature AS "entity.signature",
       entity.name AS target_name, entity.fqn AS target_fqn,
       entity.file_path AS target_file_path,
       entity.start_line AS target_start_line, entity.signature AS target_signature
```

Notes:
- `DISTINCT` is mandatory: diamond hierarchies make the same implementation
  reachable through multiple paths.
- In the `overrides` bucket the **found** endpoint (the declaration) is projected
  into the `entity.*` slots so `format_reference_entry` works unchanged; the
  queried method fills the `target_*` slots.
- Variable length `*1..` is safe: `OVERRIDES` is intra-repo only, inheritance
  chains are shallow, and the `entity.uuid <> target.uuid` guard covers
  pathological cyclic edges.
- All `OVERRIDES` edges are intra-repo by construction, so scoping the matched
  endpoint by `repo_name` is sufficient; intermediate path nodes are same-repo.

### 5.5 Formatter
`src/cli_tools/find_callers.rs` (`format_references_result`)
- Add both entries to the `rel_types` vec (lines 31-36):
  `("overridden_by", "Overridden by (method implementations)")` and
  `("overrides", "Overrides (declared supertype methods)")`.
- `total_refs` (lines 38-42) already iterates that vec — no extra change.
- The empty-result JSON init lives in `find_references` (`query.rs:129-134`), **not**
  here (v1 mis-attributed it).

### 5.6 MCP tool description (mandatory, not optional)
`src/mcp_tools/find_callers.rs:55` — the description literally reads "Returns
Markdown grouped by relationship type (Calls, Extends, Implements, References)".
Leaving it unchanged would make the tool description lie about its own output.
Update to mention `Overrides` / `Overridden by`.

### 5.7 Subgraph query allowlist
`src/db/graph/query_subgraph.rs:24-37` — `valid_rels` rejects unknown relationship
types with `anyhow::bail!`. Add `"OVERRIDES"` so `get_entity_subgraph` can traverse
the new edges.

---

## 6. BDD scenarios (write these tests FIRST)

Notation: Given/When/Then. Unit tests live beside the pass
(`src/pipeline/ingest/resolve/overrides.rs`, `#[cfg(test)]`) using the
`resolve::test_utils` mock builders; query/format tests live in their modules; the
end-to-end scenario lives under `tests/`.

### Scenario A — Groovy interface implementation (the reported case)
```
Given a Groovy interface ISession with method getUniqueId
  And a Groovy class Session that IMPLEMENTS ISession with method getUniqueId
When link_method_overrides runs
Then Session.getUniqueId has an OVERRIDES edge to ISession.getUniqueId
  And ISession.getUniqueId has NO outgoing OVERRIDES edge
```

### Scenario B — Java interface implementation
```
Given a Java interface Repository with method save
  And a Java class UserRepository implements Repository with method save
When link_method_overrides runs
Then UserRepository.save -[OVERRIDES]-> Repository.save exists
```

### Scenario C — Kotlin superclass override
```
Given a Kotlin open class Base with method greet (KotlinMethod)
  And a Kotlin class Derived : Base with method greet
When link_method_overrides runs
Then Derived.greet -[OVERRIDES]-> Base.greet exists
```

### Scenario D — Multi-level hierarchy (nearest-declaration linking)
```
Given interface A.run, class B implements A declaring run, class C extends B declaring run
When link_method_overrides runs
Then C.run -[OVERRIDES]-> B.run          (nearest declaration only)
  And B.run -[OVERRIDES]-> A.run
  And C.run has NO direct edge to A.run  (transitivity is a query-time concern, Q1/Q2)
```

### Scenario D2 — Skipped intermediate declaration (walk-through)
```
Given interface A.run, class B implements A WITHOUT declaring run,
      class C extends B declaring run
When link_method_overrides runs
Then C.run -[OVERRIDES]-> A.run          (B is walked through transparently)
```

### Scenario E — Diamond / cycle safety
```
Given interface Top.f, interfaces Left and Right each extending Top and declaring f,
      class Impl implementing both Left and Right declaring f
When link_method_overrides runs
Then Impl.f overrides Left.f and Right.f (nearest, per path)
  And Left.f overrides Top.f, Right.f overrides Top.f
  And the pass terminates (visited guard)
Variant: Left/Right do NOT declare f → Impl.f -[OVERRIDES]-> Top.f exactly once
(deduped across the two converging paths)
```

### Scenario F — Method→type grouping via FQN strip
```
Given a JVM method entity whose enclosing_class_fqn is None (always the case for JVM)
When link_method_overrides runs
Then the method is grouped under the type whose fqn equals the method fqn minus its
     last segment (covers nested classes: Outer.Inner.method → Outer.Inner; covers
     Kotlin anonymous objects: Foo.bar.<anonymous@30>.foo → Foo.bar.<anonymous@30>)
  And a method whose stripped fqn matches no type in the batch is skipped (no crash)
```

### Scenario G — Name-only matching, no false positives across unrelated types
```
Given class Foo.process and unrelated class Bar.process with NO inheritance relation
When link_method_overrides runs
Then no OVERRIDES edge is created between Foo.process and Bar.process
```

### Scenario H — Overloads (name-only, documented behavior)
```
Given interface I with visit(A) and visit(B) (two methods named visit)
  And class Impl implements I with visit(A) and visit(B)
When link_method_overrides runs
Then every Impl.visit links to every I.visit at the nearest declaring level
     (N×M name-only fan-out) — accepted
```

### Scenario I — No supertype method match
```
Given class C implements I, C has helper() but I does not declare helper()
When link_method_overrides runs
Then C.helper has no OVERRIDES edge
```

### Scenario J — Constructors are excluded
```
Given a Java/Groovy class Base and a class Sub extends Base
When link_method_overrides runs
Then no OVERRIDES edge exists between constructor-like entities
     (method name == enclosing type name, or "<init>")
```

### Scenario K — Static / private methods (documented limitation)
```
Given class Base with a static method util() and class Sub extends Base with static util()
When link_method_overrides runs
Then Sub.util MAY link to Base.util (static hiding is semantically not overriding,
     but modifiers are not persisted on JVM entities, so name-only matching cannot
     distinguish this today) — pinned as current behavior; modifier-aware filtering
     is a follow-up (§10). Same for private methods.
```

### Scenario M — Incremental indexing limitation
```
Given ISession.groovy unchanged and only Session.groovy re-indexed
When link_method_overrides runs on the Session-only batch
Then no OVERRIDES edge is created (supertype methods are absent from the batch)
  And the pass completes without error
  (A full re-index creates the edge; see §9 and §10 follow-up.)
```

### Scenario N — Non-JVM languages are unaffected (guardrail)
```
Given a Rust trait+impl, a Python base/derived class, and a TypeScript
      interface+class pair, each with same-named methods
When link_method_overrides runs
Then ZERO OVERRIDES edges are created for Rust, Python, or TypeScript entities
     (TypeScript shares the generic Class/Interface/Method kinds — the
     file-extension guard is what excludes it)
```

### Scenario Q1 — Query returns implementations when querying the interface method
```
Given the graph contains Session.getUniqueId -[OVERRIDES]-> ISession.getUniqueId
  And SubSession.getUniqueId -[OVERRIDES]-> Session.getUniqueId
When find_references("getUniqueId") targets ISession.getUniqueId
Then the "overridden_by" bucket contains Session.getUniqueId
  And SubSession.getUniqueId (transitive via *1..)
```

### Scenario Q2 — Query returns the declaration when querying an implementation
```
Given the same graph
When find_references targets SubSession.getUniqueId
Then the "overrides" bucket contains Session.getUniqueId
  And ISession.getUniqueId (transitive via *1..)
```

### Scenario Q3 — Formatter renders the new buckets
```
Given a references result with non-empty "overrides" and "overridden_by" arrays
When format_references_result runs
Then output contains "Overridden by (method implementations)" and
     "Overrides (declared supertype methods)" with the entries
  And total_refs includes both buckets
```

### Scenario Q4 — Query dedup and anti-self
```
Given the diamond graph of Scenario E (all four types declaring f)
When find_references targets Top.f
Then the "overridden_by" bucket contains Left.f, Right.f and Impl.f exactly once
  And never contains Top.f itself
```

### Scenario E2E — Full index + query
```
Given a small multi-file JVM fixture (interface + two implementers, one transitive
      chain, one skipped-declaration chain)
When the fixture is indexed end-to-end
Then find_callers on the interface method returns all implementers under Overridden by
  And find_callers on an implementer returns the interface method under Overrides
```

---

## 7. Implementation order (strict TDD)

0. **Pre-implementation verification checklist** (no code written before this):
   - [ ] Java: confirm method FQN format on a real fixture via the generic enrich
         path (`Class.method`; nested `Outer.Inner.method`). Confirmed at unit level
         (`context.rs:510-519`); verify end-to-end.
   - [ ] Kotlin: confirm method FQN format for classes, objects, companion objects
         and anonymous objects (`kotlin.rs:928-974` covers the anonymous case).
   - [ ] Groovy: confirmed — `pkg.Class.method` (`groovy.rs:462-468`).
   - [ ] Java/Kotlin: confirm constructors are not emitted as method-like entities
         (Groovy confirmed clean, `groovy.rs:979`); if they are, the name==type-name
         exclusion (§4) must cover them — add a fixture asserting it.
   - [ ] Confirm `enum`/`object` entities receive `Extends`/`Implements` intents
         from their parsers; if not, they silently produce no edges (acceptable,
         document).
1. **Red:** add `RelationshipType::Overrides` compile stub + `Display` arm; write
   Scenarios A–J, K, M, N as failing unit tests against an unimplemented
   `link_method_overrides` (function present, body `todo!()` or no-op).
2. **Green:** implement `link_method_overrides` (two-phase: maps →
   nearest-declaration BFS → dedup → apply) until A–J, K, M, N pass.
3. **Refactor:** extract `is_jvm_file` / `is_jvm_method_like` / `is_jvm_type_like` /
   `enclosing_type_fqn`, tidy BFS.
4. **Red:** wire the pass into `resolve_reference_intents_with_context`; add
   Scenarios Q1–Q4 (query + formatter) as failing.
5. **Green:** implement the two directed buckets (§5.4) + formatter labels +
   MCP description + `valid_rels` entry.
6. **Red/Green:** add the E2E fixture + Scenario E2E under `tests/`; register in the
   fast e2e suite alongside existing suites (see `all_e2e_fast`).
7. **Verify:** `cargo test`, `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt -- --check`, plus targeted `cargo test link_method_overrides` and the
   e2e suite.

---

## 8. Manual validation (post-merge)

1. Re-index nextflow (`OVERRIDES` edges are created at ingest; existing index has none).
2. `find_callers getUniqueId --repo nextflow` → expect `Session.getUniqueId` (and other
   implementers) under the **Overridden by** group.
3. Query an implementer's method → expect `ISession.getUniqueId` under the
   **Overrides** group.

---

## 9. Risks & mitigations

| Risk | Mitigation |
| --- | --- |
| **Incremental indexing misses edges** — the pass is batch-local; if the supertype was not re-parsed in this batch, its methods are unavailable and no edge is created (Scenario M) | Documented limitation. Full re-index for complete coverage (§8 step 1). Follow-up: preload supertype methods from Neo4j or link via graph-wide Cypher like the CONTAINS auto-link (§10). |
| Method→type grouping via FQN strip fails on an unexpected FQN format | Formats confirmed for all three JVM languages (§5.2); step-0 checklist re-verifies on fixtures; unmatched methods are skipped, never crash (Scenario F). |
| Java FQNs lack package prefix (pre-existing: `Class.method`) | Same-named classes in different packages already collide in `fqn_to_uuid` today; this spec neither worsens nor fixes it. Cross-package collisions yield at worst a spurious edge — noted, follow-up candidate. |
| Static/private methods produce semantically wrong edges (hiding ≠ overriding) | Modifiers are not persisted on JVM entities; name-only cannot distinguish (Scenario K pinned as current behavior). Follow-up: parser-level modifier capture + filtering (§10). |
| Overload fan-out (name-only) | Documented/accepted (Scenario H); arity refinement is a future option. |
| Constructors linked as overrides | Name==type-name / `<init>` exclusion (§4, Scenario J); step-0 verifies parser behavior. |
| Supertype in a dependency repo (cross-repo) | Out of scope; intra-repo only this iteration. |
| Stale edges after rename/delete (`MERGE` never deletes) | Pre-existing for all relationship types; not introduced here; GC is a follow-up (§10). |
| Performance on large repos | Single in-memory pass over the batch, O(methods × hierarchy depth) with nearest-declaration early-exit; negligible, off the parallel path. If needed, parallelize per top-level type in a later refactor. |

---

## 10. Follow-ups (out of scope)

- **Incremental completeness:** preload supertype method maps from Neo4j (extending
  `load_entity_mappings`) or perform the linking with graph-wide Cypher, so mixed
  incremental runs also create edges.
- **Modifier-aware filtering:** persist `static`/`private`/visibility modifiers in
  the JVM parsers and exclude static/private methods from `OVERRIDES` (Scenario K).
- Use `OVERRIDES` edges to fan out ambiguous call-site resolution to implementations,
  improving the `CALLS` bucket for interface-typed receivers.
- Cross-repo override linking using loaded dependency entity mappings.
- Optional arity-aware matching to split overloads.
- Package-qualified Java FQNs (pre-existing collision caveat, §9).
- Stale-edge garbage collection for all relationship types.
