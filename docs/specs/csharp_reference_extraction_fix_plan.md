# C# Reference Extraction Fixes: Qualified References and Declaration-Name False Positives

**Status:** Proposed
**Scope:** `src/pipeline/parser/languages/csharp/refs.rs`, `src/pipeline/ingest/resolve/non_calls.rs`, `queries/csharp.scm` (optional phase), `tests/run_csharp_e2e.sh`, `tests/testing_files/csharp/`
**Methodology:** BDD/TDD (Red → Green → Refactor), per `AGENTS.md` § Testing Strategy
**Related:** `docs/specs/csharp_support_plan.md` §9.3 (AST-to-intent mapping)

---

## 1. Context and Problem Statement

Two defects were found while validating the C# indexer against a real-world
codebase (`~/workspace/github/openlogi-net`, an Avalonia/.NET application with
~200 `.cs` files). Both defects were reproduced from live `find_callers`
output cross-checked against the source files.

### 1.1 Defect A — Real references in `Config.cs` / `ConfigCodec.cs` are missing

The type `GestureOwner` is a discriminated union modelled with a C# abstract
record and nested sealed records:

```csharp
// src/OpenLogi.Core/Gestures/GestureOwner.cs
public abstract record GestureOwner
{
    private GestureOwner() { }
    public sealed record Off : GestureOwner;
    public sealed record Button(ButtonId Id) : GestureOwner;
    public static readonly Off OffValue = new();
}
```

The nested record `GestureOwner.Off` is used in five places across two files:

| File | Line | Code |
|---|---|---|
| `Config.cs` | 117 | `device.GestureOwner is GestureOwner.Off` |
| `Config.cs` | 141 | `d.GestureOwner is GestureOwner.Off` |
| `Config.cs` | 149 | `d.GestureOwner is GestureOwner.Off` |
| `Config.cs` | 178 | `Gestures.GestureOwner.Off => null` (switch arm) |
| `Config.cs` | 204 | `= Gestures.GestureOwner.OffValue;` |
| `ConfigCodec.cs` | 70 | `GestureOwner.Off => "Off"` (switch arm) |
| `ConfigCodec.cs` | 431 | `return GestureOwner.OffValue;` |

**None** of these produce a `REFERENCES` edge to
`OpenLogi.Core.Gestures.GestureOwner.Off` (nor to `…GestureOwner.OffValue`) in
the graph. `find_callers GestureOwner.Off` reports only two references, both
artefacts (see Defect B).

### 1.2 Defect B — False `REFERENCES` edge from the `LightingEffect` enum

```csharp
// src/OpenLogi.App/ViewModels/LightingEffect.cs
public enum LightingEffect { Solid, Breathing, Cycle, Off }
```

`find_callers` reports:

```
### Target: `OpenLogi.Core.Gestures.GestureOwner.Off`
## References (type annotations/usages)
- `LightingEffect` (csharp_enum) at src/OpenLogi.App/ViewModels/LightingEffect.cs:4
```

`LightingEffect` has no relationship whatsoever with `GestureOwner`. The two
types live in different assemblies (`OpenLogi.App` vs `OpenLogi.Core`) and
`LightingEffect.cs` does not even `using OpenLogi.Core.Gestures`.

---

## 2. Root Cause Analysis (AST-verified)

The tree-sitter s-expressions below were obtained by dumping
`tree.root_node().to_sexp()` for the exact source shapes above. They are
**measured, not assumed**.

### 2.1 AST shapes

| C# source | Tree-sitter AST (abridged) |
|---|---|
| `d.GestureOwner is GestureOwner.Off` | `is_pattern_expression expression:(member_access_expression) pattern:(constant_pattern (member_access_expression expression:(identifier "GestureOwner") name:(identifier "Off")))` |
| `Gestures.GestureOwner.Off => null` | `switch_expression_arm (constant_pattern (member_access_expression expression:(member_access_expression expression:(identifier "Gestures") name:(identifier "GestureOwner")) name:(identifier "Off")))` |
| `= Gestures.GestureOwner.OffValue;` | `assignment_expression right:(member_access_expression expression:(member_access_expression …) name:(identifier "OffValue"))` |
| `GestureOwner.Button b => b.Id` | `switch_expression_arm (declaration_pattern type:(qualified_name qualifier:(identifier "GestureOwner") name:(identifier "Button")) name:(identifier "b"))` |
| `enum LightingEffect { …, Off }` | `enum_declaration body:(enum_member_declaration_list (enum_member_declaration name:(identifier "Off")))` |

### 2.2 Root cause of Defect A — no qualified reference intents exist

`collect_all_reference_intents_csharp` (`src/pipeline/parser/languages/csharp/refs.rs:190`)
and `extract_type_references_csharp` (`refs.rs:162`) only ever emit
**single-identifier** `TypeReference` intents, and both guard with
`is_member_access_name` (`refs.rs:491`), which discards any identifier that is
the `name:` field of a `member_access_expression`.

Consequences, segment by segment:

| Expression | `GestureOwner` emitted? | `Off` / `OffValue` emitted? |
|---|---|---|
| `GestureOwner.Off` | ✅ (root of the chain) | ❌ (member-access name) |
| `Gestures.GestureOwner.Off` | ❌ (name of the inner member access) | ❌ (name of the outer member access) |
| `Gestures.GestureOwner.OffValue` | ❌ | ❌ |

So the model has **no vocabulary for "the nested member of a type"**. The
graph edge cannot exist because the intent is never produced.

Note this also explains an observation that initially looked contradictory:
`Config.cs` methods *do* have `REFERENCES` edges to the outer record
`GestureOwner` — those come from the single-identifier root of `GestureOwner.Off`,
not from a qualified path.

Emitting a bare `TypeReference{"Off"}` would **not** be an acceptable fix: it
is exactly the mechanism that produces Defect B. The fix must emit the dotted
path and resolve it against fully-qualified names.

### 2.3 Root cause of Defect B — declaration names are treated as type references

The `enum_member_declaration name:(identifier "Off")` node reaches the
catch-all branch at `refs.rs:236`:

```rust
"identifier" if !is_member_access_name(node) => {
    let type_name = node_text(node, source);
    if is_capitalized(&type_name) {
        out.push((ReferenceIntent::TypeReference { type_name, line }, byte_pos));
    }
}
```

`Off` is capitalized and is not a member-access name, so a
`TypeReference{"Off"}` intent is emitted. The orphan/covered-range pass
attributes it to the entity that covers that byte offset — the `LightingEffect`
enum entity. Then
`resolve_non_call_reference_typed` (`src/pipeline/ingest/resolve/non_calls.rs:81`)
runs the disambiguation ladder:

1. `name_to_uuids["Off"]` → candidates.
2. Filtered to type-like kinds — `CSharpRecord` passes `is_type_like`
   (`non_calls.rs:16-47`).
3. No same-file candidate; enclosing-class FQN probe fails.
4. `candidate_uuids.len() == 1` → **resolved** (`non_calls.rs:136`).

The self-reference filter in `enrich.rs:302` does not help: it compares the
referenced name against the *owning entity's* name (`LightingEffect` ≠ `Off`).

The same defect class produces two more artefacts:

- `declaration_pattern type:(qualified_name … name:(identifier "Button"))`
  emits a bare `TypeReference{"Button"}`, which will latch onto whatever type
  named `Button` happens to be unambiguous in the index.
- The nested declarations' own names (`Off`, `Button` inside `GestureOwner.cs`)
  are emitted from within the outer record's covered range, producing the
  redundant `GestureOwner → GestureOwner.Off` / `→ GestureOwner.Button`
  `REFERENCES` edges visible in `find_callers` output. `CONTAINS` already
  models that containment.

### 2.4 Blast radius

Both defects are **C#-specific** (they live in the C# reference walker). The
resolver change in Phase 3 touches shared code but is purely additive (see
§6.3).

---

## 3. Goals and Non-Goals

### Goals

- G1. `X.Y` and `A.B.C` member/type paths in C# produce a reference to the
  entity actually named by the path, resolved via fully-qualified name.
- G2. Identifiers that are the *declared name* of a declaration never become
  type references.
- G3. Zero regression on the 31 existing C# E2E assertions and on the other
  language suites.
- G4. Every fix is locked in by a test that fails before the fix.

### Non-Goals

- Full semantic type inference (variable types, generic instantiation,
  `using`-alias resolution). The suffix-matching heuristic is deliberately
  conservative: unresolvable is preferred over wrong.
- Fixing `find_callers` query-side fuzziness — that is a separate defect in the
  MCP/CLI layer, tracked in
  `docs/specs/find_callers_target_resolution_plan.md`.

---

## 4. Phase 0 — Tests First (the Red phase)

Nothing in `src/` is modified in this phase. The suite must be **red** for the
new assertions and **green** for everything else before Phase 1 begins.

### 4.1 New E2E fixtures

The fixtures are a minimal, faithful reduction of the real `openlogi-net`
shapes. Namespaces follow the existing `MyApp.*` convention of the C# suite.

**`tests/testing_files/csharp/Gestures/GestureOwner.cs`**

```csharp
namespace MyApp.Gestures;

/// <summary>
/// Which control owns a device's single gesture role: explicitly off, or a
/// named button. Mirrors a discriminated union.
/// </summary>
public abstract record GestureOwner
{
    private GestureOwner() { }

    /// <summary>Gestures explicitly turned off for this device.</summary>
    public sealed record Off : GestureOwner;

    /// <summary>The named button owns the gesture role.</summary>
    public sealed record Button(int Id) : GestureOwner;

    public static readonly Off OffValue = new();
}
```

**`tests/testing_files/csharp/ViewModels/LightingEffect.cs`**

```csharp
namespace MyApp.ViewModels;

/// <summary>Keyboard lighting effect modes.</summary>
public enum LightingEffect
{
    Solid,
    Breathing,
    Cycle,
    /// <summary>Lighting off.</summary>
    Off,
}
```

**`tests/testing_files/csharp/Gestures/GestureConfig.cs`**

```csharp
namespace MyApp.Gestures;

public class DeviceEntry
{
    public GestureOwner? Owner { get; set; }
}

public class GestureConfig
{
    /// <summary>`is` pattern against a nested record (short qualified path).</summary>
    public bool GesturesEnabled(DeviceEntry d) => !(d.Owner is GestureOwner.Off);

    /// <summary>switch arm with a fully qualified constant pattern.</summary>
    public int? OwnerOf(DeviceEntry d) => d.Owner switch
    {
        MyApp.Gestures.GestureOwner.Off => null,
        GestureOwner.Button b => b.Id,
        _ => null,
    };

    /// <summary>Static field access through a fully qualified path.</summary>
    public void Disable(DeviceEntry d) => d.Owner = MyApp.Gestures.GestureOwner.OffValue;

    /// <summary>Object creation of a nested record.</summary>
    public void Select(DeviceEntry d, int id) => d.Owner = new GestureOwner.Button(id);
}
```

### 4.2 New E2E assertions — `tests/run_csharp_e2e.sh`, group F

Appended after the current `E31` block, using the existing helpers
(`assert_edge_exists`, `assert_no_edge`, `assert_cypher_count` from
`tests/lib/assert_neo4j_relationships.sh`).

| ID | Assertion | Guards |
|---|---|---|
| F32 | `assert_edge_exists MyApp.Gestures.GestureConfig.GesturesEnabled MyApp.Gestures.GestureOwner.Off REFERENCES` | Defect A — `is` pattern |
| F33 | `assert_edge_exists MyApp.Gestures.GestureConfig.OwnerOf MyApp.Gestures.GestureOwner.Off REFERENCES` | Defect A — switch arm, fully qualified |
| F34 | `assert_edge_exists MyApp.Gestures.GestureConfig.Disable MyApp.Gestures.GestureOwner.OffValue REFERENCES` | Defect A — static field path |
| F35 | `assert_no_edge MyApp.ViewModels.LightingEffect MyApp.Gestures.GestureOwner.Off REFERENCES` | **Defect B** |
| F36 | `assert_cypher_count` — outgoing `REFERENCES` from `MyApp.ViewModels.LightingEffect` == 0 | Defect B, generalised |
| F37 | `assert_edge_exists MyApp.Gestures.GestureOwner.Off MyApp.Gestures.GestureOwner EXTENDS` | Regression guard: the `base_list` heuristic must keep working |
| F38 | `assert_no_edge MyApp.Gestures.GestureOwner MyApp.Gestures.GestureOwner.Off REFERENCES` | Declaration names are not references (`CONTAINS` covers this) |
| F39 | `assert_edge_exists MyApp.Gestures.GestureConfig.Select MyApp.Gestures.GestureOwner.Button CALLS` | `new GestureOwner.Button(id)` still redirects to the constructor |

Also update in the same commit:

- The script header comment (`31 assertions` → `39 assertions`).
- Per-kind counts `A12b` (classes +2: `DeviceEntry`, `GestureConfig`),
  `A12c` (records +3: `GestureOwner`, `Off`, `Button`), `A12e` (enums +1).
- Any suite tally in `tests/run_all_e2e_fast.sh`.

**Expected Red state:** F32, F33, F34 fail (missing edge); F35, F36, F38 fail
(spurious edge present); F37, F39 pass.

### 4.3 New unit tests

Unit tests are the primary safety net (per `AGENTS.md`); the E2E suite is the
integration proof.

#### `src/pipeline/parser/languages/csharp/tests.rs` — parser level

Assertions run against `ParsedEntity::reference_intents` after `extract()`.

1. `test_is_pattern_emits_qualified_nested_type_reference`
   `GesturesEnabled` contains `TypeReference{"GestureOwner.Off"}` **and still
   contains** `TypeReference{"GestureOwner"}` (no regression on the existing
   edge).
2. `test_switch_arm_constant_pattern_emits_qualified_reference`
   `OwnerOf` contains `TypeReference{"MyApp.Gestures.GestureOwner.Off"}`.
3. `test_static_member_access_emits_qualified_reference`
   `Disable` contains `TypeReference{"MyApp.Gestures.GestureOwner.OffValue"}`.
4. `test_declaration_pattern_emits_qualified_not_bare`
   `OwnerOf` contains `TypeReference{"GestureOwner.Button"}` and **does not**
   contain a bare `TypeReference{"Button"}`.
5. `test_enum_member_name_is_not_a_type_reference`
   The `LightingEffect` entity has no `TypeReference` whose `type_name` is
   `"Off"`, `"Solid"`, `"Breathing"` or `"Cycle"`.
6. `test_declaration_names_are_not_type_references`
   The `GestureOwner` record entity has no `TypeReference{"Off"}` /
   `{"Button"}`; the nested records still carry their `Extends` intent.
7. `test_invocation_receiver_path_does_not_duplicate_call_intent`
   `Console.WriteLine(x)` yields exactly one `Call{method:"WriteLine"}` and no
   `TypeReference{"Console.WriteLine"}` (see §5.1, rule R4).

#### `src/pipeline/ingest/resolve/non_calls.rs` — resolver level

Built with the existing `mock_resolution_entity_with_kind` helper.

8. `test_qualified_type_reference_resolves_by_fqn_suffix`
   Given an entity with FQN `MyApp.Gestures.GestureOwner.Off` (`CSharpRecord`),
   a `TypeReference{"GestureOwner.Off"}` resolves to it.
9. `test_qualified_reference_resolves_non_type_member`
   `TypeReference{"GestureOwner.OffValue"}` resolves to the `CSharpField`
   entity (the type-like filter must be relaxed for dotted paths).
10. `test_qualified_reference_has_no_bare_last_segment_fallback`
    `TypeReference{"Task.FromResult"}` produces **no** edge even when an
    unrelated entity named `FromResult` exists.
11. `test_qualified_reference_ambiguous_suffix_is_skipped`
    Two entities in different files whose FQNs both end in `.GestureOwner.Off`
    → zero edges and `references_ambiguous_skipped` incremented.
12. `test_qualified_reference_prefers_same_file`
    Same-file candidate wins over an equally-matching foreign-file candidate
    (consistency with the existing ladder).

#### Integrated regression test — `csharp/tests.rs`

13. `test_gesture_owner_off_regression_parser_to_resolver`

    Mirrors the existing `test_groovy_extends_resolves_to_extends_relationship`
    pattern (`non_calls.rs:449`): parse the three fixture sources with
    `extract_entities`, map to `ResolutionEntity`, run
    `resolve_reference_intents`, then assert in one shot:

    - `LightingEffect` has **no** `References` edge to the `Off` record ← Defect B
    - `GesturesEnabled` and `OwnerOf` **have** a `References` edge to the `Off`
      record ← Defect A
    - `Disable` **has** a `References` edge to `OffValue` ← Defect A
    - `Off` and `Button` retain their `Extends` edge to `GestureOwner`
    - `GestureOwner` has no `References` edge to `Off` / `Button`

    This single test is the executable statement of the whole bug report and is
    the one to run first when touching C# reference extraction in future.

---

## 5. Phase 1 — Qualified reference intents (parser)

**File:** `src/pipeline/parser/languages/csharp/refs.rs`

### 5.1 New helper: dotted path extraction

```rust
/// Reduce a `member_access_expression` / `qualified_name` chain to its dotted
/// text when **every** segment is a plain identifier.
///
/// `GestureOwner.Off`               -> Some("GestureOwner.Off")
/// `MyApp.Gestures.GestureOwner.Off` -> Some("MyApp.Gestures.GestureOwner.Off")
/// `Device(key).GestureOwner`        -> None  (root segment is an invocation)
/// `this.Owner`                      -> None  (root segment is not an identifier)
fn dotted_path(node: Node<'_>, source: &[u8]) -> Option<String>;
```

Implementation notes:

- Walk the chain through `expression:` (for `member_access_expression`) or
  `qualifier:` (for `qualified_name`), collecting each `name:` segment.
- Abort with `None` on the first non-`identifier` segment
  (`invocation_expression`, `this_expression`, `base_expression`,
  `element_access_expression`, literals, `predefined_type`, …). Being strict
  here is what keeps the intent stream free of noise.
- Return segments joined with `.`.

### 5.2 Emission rules

A qualified `TypeReference` is emitted when **all** of the following hold:

- **R1** — the node kind is `member_access_expression` or `qualified_name`.
- **R2** — `dotted_path` returns `Some(path)` with ≥ 2 segments.
- **R3** — the **penultimate** segment is capitalized (`is_capitalized`). This
  is the "the qualifier is a type, not a variable" heuristic:
  `GestureOwner.Off` ✅, `d.Owner` ❌, `args.Length` ❌.
- **R4** — the node is **not** the `function:` field of an
  `invocation_expression`. Those are already handled by `single_call_intents`
  (`refs.rs:301`) and would otherwise produce a duplicate
  `CALLS` + `REFERENCES` pair.
- **R5** — the node is **not** the `expression:`/`qualifier:` field of another
  `member_access_expression` / `qualified_name`. Only the **outermost** node of
  a chain emits, so `MyApp.Gestures.GestureOwner.Off` produces exactly one
  intent, not three nested prefixes.

### 5.3 Integration points

- `collect_all_reference_intents_csharp` (`refs.rs:190`) — add a branch for
  `member_access_expression` / `qualified_name` before the generic recursion.
  **Keep descending into children** afterwards: the existing single-identifier
  emissions (the chain root, e.g. `GestureOwner` in `GestureOwner.Off`) must
  survive so no currently-passing E2E assertion regresses.
- `extract_type_references_csharp` (`refs.rs:162`) — mirror the same branch for
  the per-entity path used by `extract_reference_intents_csharp`.

### 5.4 Documentation

Extend the AST-to-intent table in the module docstring (`refs.rs:3-28`) and
`docs/specs/csharp_support_plan.md` §9.3 with:

| AST node | Emitted intent |
|---|---|
| outermost `member_access_expression` / `qualified_name` chain of plain identifiers with a capitalized penultimate segment, not an invocation callee | `TypeReference { type_name: "<dotted path>" }` |

---

## 6. Phase 2 — Declaration names are not references (parser)

**File:** `src/pipeline/parser/languages/csharp/refs.rs`

### 6.1 New helper

```rust
/// `true` when `node` is the `name:` field of a declaration — the identifier
/// *introduces* a symbol rather than referring to one. Declared names must
/// never become type references: an enum member `Off` is not a usage of an
/// unrelated record named `Off`.
fn is_declaration_name(node: Node<'_>) -> bool;
```

Parent kinds covered (node must be the parent's `name:` field):

```
enum_member_declaration          class_declaration        interface_declaration
struct_declaration               record_declaration       enum_declaration
method_declaration               constructor_declaration  destructor_declaration
property_declaration             event_declaration        delegate_declaration
namespace_declaration            file_scoped_namespace_declaration
local_function_statement         variable_declarator      parameter
type_parameter                   catch_declaration        declaration_expression
declaration_pattern
```

### 6.2 Integration

Add to the identifier guard in both walkers, next to `is_member_access_name`:

```rust
"identifier" if !is_member_access_name(node) && !is_declaration_name(node) => { … }
```

Note the `type:` fields are untouched, so parameter types, return types,
property types and `declaration_pattern type:` continue to emit references —
which is what keeps assertions D24 and friends green.

### 6.3 Effects

- Kills Defect B at the source: the enum member `Off` is no longer a reference.
- Kills the `qualified_name` bare-last-segment artefact (`Button`), reinforced
  by Phase 1 rule R5.
- Removes the redundant `Outer → Nested` `REFERENCES` edges (F38); `CONTAINS`
  already expresses containment.

---

## 7. Phase 3 — FQN-suffix resolution for dotted references (resolver)

**File:** `src/pipeline/ingest/resolve/non_calls.rs`

### 7.1 New branch in `resolve_non_call_reference_typed`

Guarded by `name.contains('.')`, executed before the existing ladder:

1. **Exact FQN** — `fqn_to_uuid.get(path)`.
2. **FQN suffix** — take the last segment, look it up in `name_to_uuids`, keep
   candidates whose FQN (via `uuid_to_fqn`, already available in
   `ResolutionContext`, `context.rs:50`) equals `path` or ends with
   `format!(".{path}")`. For non-C# callers also accept `format!("::{path}")`.
3. **No `is_type_like` filter in this branch.** The FQN-suffix constraint is far
   stronger than a kind filter, and the legitimate target of a dotted path may
   be a field (`OffValue`), property or method.
4. **Existing ladder on the surviving candidates** — same-file preference →
   single candidate → otherwise ambiguous (`references_ambiguous_skipped`).
5. **No fallback to the bare last segment.** If nothing matches (external types
   such as `Task.FromResult`, `Console.WriteLine`, `StringComparison.Ordinal`)
   the intent is counted as unresolved. This is the invariant that prevents
   re-introducing Defect B.

### 7.2 Why this is safe for other languages

`resolve_non_call_reference_typed` is shared. Today a dotted `name` fails the
very first lookup (`name_to_uuids.get(name)`, `non_calls.rs:91`) and is counted
as unresolved. The new branch can therefore only **add** resolutions; it can
never remove or redirect an existing edge. Languages that already emit dotted
type names (Python `module.Class`, TS namespace paths) gain accuracy for free.

### 7.3 Optional refinement (defer unless needed)

C# entities carry `enclosing_class_fqn` (set in `enrich.rs:314`). It could seed
a "resolve relative to the enclosing namespace first" step for short paths.
Not required to close either defect; revisit only if §7.1 step 2 proves
ambiguous on a real repo.

---

## 8. Phase 4 — Enum members as entities (optional, recommended)

Not on the critical path; Phases 1–3 close both reported defects.

**Change:** add to `queries/csharp.scm`

```scheme
; --- Enum member declarations ---
(enum_member_declaration
  name: (identifier) @csharp.enum_member.name)
```

routed in `capture.rs` to `EntityKind::CSharpConstant` with FQN
`MyApp.ViewModels.LightingEffect.Off`.

**Benefit:** enum members are referenced qualified everywhere in idiomatic C#
(`LightingEffect.Off`, `DeviceKind.Unknown`). With Phase 1 in place those paths
become resolvable to a real entity instead of dying as unresolved, and any
structural ambiguity between `LightingEffect.Off` and `GestureOwner.Off`
disappears by construction.

**Cost:** entity count grows (one node per enum member); `A12` per-kind counts
and the Qdrant collection size change; needs its own unit + E2E assertions
(`assert_cypher_exists` for `MyApp.ViewModels.LightingEffect.Off` as
`csharp_constant`, and an edge assertion from a consumer).

**Decision required from the maintainer before implementing.**

---

## 9. Phase 5 — Validation

| Step | Command | Gate |
|---|---|---|
| 1 | `cargo test --lib csharp` | Tests 1–7, 13 green |
| 2 | `cargo test --lib resolve` | Tests 8–12 green |
| 3 | `cargo fmt -- --check` | Clean |
| 4 | `cargo clippy --all-targets --all-features -- -D warnings` | Clean, **no `#[allow]`** (per `AGENTS.md`; `#[expect(reason = …)]` only as a documented last resort) |
| 5 | `./tests/run_csharp_e2e.sh` | 39/39 |
| 6 | `./tests/run_all_e2e_fast.sh` | All suites green (Phase 3 touches shared code) |
| 7 | Re-index `openlogi-net` with `--clean`, then `knot callers "GestureOwner.Off" -r openlogi-net` | Lists `GesturesEnabled`, `GestureButtons`, `EnableGestures`, `GestureOwner(string)`, `SerializeDevice`, `ParseGestureOwner`; **does not** list `LightingEffect` |

### 9.1 Manual verification matrix (real repo)

| Expectation | Source of truth |
|---|---|
| 5 references to `GestureOwner.Off` from `Config.cs` | lines 117, 141, 149, 178 |
| 2 references to `GestureOwner.OffValue` | `Config.cs:204`, `ConfigCodec.cs:431` |
| 1 reference to `GestureOwner.Off` from `ConfigCodec.cs` | line 70 |
| 0 references from `LightingEffect` | `LightingEffect.cs` has no `using OpenLogi.Core.Gestures` |

---

## 10. Risks and Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Qualified emission floods the intent stream on large repos (every `Foo.Bar` in the codebase) | Medium | Rules R3–R5 are restrictive; unresolved intents are dropped at resolution and cost only transient memory. Measure with `./tests/benchmark_e2e.sh` before/after. |
| `ENDS WITH`-style suffix matching resolves to the wrong homonym | Low | Ambiguity is *skipped*, not guessed (test 11); same-file preference applies first. |
| Phase 2 removes a reference some existing E2E assertion depended on | Low | Full `run_all_e2e_fast.sh` gate in step 6; the C# suite's D-group assertions all target `type:` fields, which are untouched. |
| Phase 3 alters non-C# behaviour | Very low | Additive-only by construction (§7.2); covered by the cross-language E2E run. |

---

## 11. Deliverables Checklist

- [ ] 3 new fixture files under `tests/testing_files/csharp/`
- [ ] 8 new E2E assertions (F32–F39) + header/count updates in `run_csharp_e2e.sh`
- [ ] 13 new unit tests (7 parser, 5 resolver, 1 integrated regression)
- [ ] `dotted_path` + qualified emission in `refs.rs` (Phase 1)
- [ ] `is_declaration_name` guard in `refs.rs` (Phase 2)
- [ ] FQN-suffix branch in `non_calls.rs` (Phase 3)
- [ ] Phase 4 decision recorded (implement or explicitly defer)
- [ ] `refs.rs` module docstring table updated
- [ ] `docs/specs/csharp_support_plan.md` §9.3 updated
- [ ] `README.md` C# section + `CHANGELOG.md` entry
- [ ] Full validation matrix (§9) green

## 12. Out of Scope — tracked elsewhere

`find_callers "Off"` returning unrelated entities (`OfflineSlot`,
`IsEligible(DateTimeOffset …)`) is **not** an indexer defect: it originates in
the substring-based Cypher predicates of the MCP/CLI query layer. See
`docs/specs/find_callers_target_resolution_plan.md`.
