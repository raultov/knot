# `find_callers` Target Resolution: Eliminating Substring Noise in the MCP/CLI Layer

**Status:** Proposed
**Scope:** `src/db/graph/query.rs`, `src/db/graph/connection.rs`, `src/cli_tools/find_callers.rs`, `src/cli_tools/formatters.rs`, `src/mcp_tools/find_callers.rs`, `tests/run_csharp_e2e.sh`, `.prompt`, `.knot-agent.md`
**Methodology:** BDD/TDD (Red → Green → Refactor)
**Related:** `docs/specs/csharp_reference_extraction_fix_plan.md` (indexer-side defects; disjoint from this one)

---

## 1. Context and Problem Statement

Querying `find_callers` for a short entity name returns large numbers of
entities that have no relationship to the queried symbol.

**Observed** — `find_callers(entity_name: "Off", repo_name: "openlogi-net")`,
where the only entity actually named `Off` is the nested record
`OpenLogi.Core.Gestures.GestureOwner.Off`:

```
Found 21 reference(s) across all relationship types:

## Calls (function/method invocations) (19)

### Target: `OpenLogi.Tests.Hid.InventoryDedupeTests.OfflineSlot` …
### Target: `OpenLogi.Core.UpdateCheck.IsEligible` at src/OpenLogi.Core/UpdateCheck.cs:22
    Signature: `(DateTimeOffset publishedAt, DateTimeOffset now)`
### Target: `OpenLogi.Tests.Hid.ParkedLinkInventoryTests.OfflineKeyboardArrival` …
### Target: `OpenLogi.Core.UpdateCheck.Decide` at src/OpenLogi.Core/UpdateCheck.cs:55
    Signature: `(ReleaseInfo? latest, string? dismissed, DateTimeOffset now)`
```

19 of 21 results are noise. `IsEligible` and `Decide` matched because their
**parameter list** contains `DateTimeOffset`, which contains the substring
`Off`. `OfflineSlot` / `OfflineKeyboardArrival` matched because their **FQN**
contains `Off`.

The cost is not cosmetic: an agent doing impact analysis is handed a
mostly-false dependency set, and the tool's own description already had to
carry a workaround instruction ("CRITICAL: For common method names you MUST
include a signature fragment") that pushes the problem onto the caller.

---

## 2. Root Cause Analysis

### 2.1 The predicate

`QueryExt::find_references` (`src/db/graph/query.rs:132`) issues one Cypher
query per relationship type with this `WHERE` clause
(`query.rs:157-180`, both the repo-scoped and unscoped branches):

```cypher
MATCH (entity:Entity)-[:CALLS]->(target:Entity)
WHERE target.repo_name = $repo_name
  AND (target.name = $name
   OR target.fqn = $name
   OR target.fqn CONTAINS $name
   OR (target.name + COALESCE(target.signature, '')) CONTAINS $name)
```

The same 4-clause disjunction is repeated in the two `OVERRIDES` queries
(`query.rs:217-240` and `query.rs:245-262`) — six copies in total.

### 2.2 Which clause fires for which noise

| Clause | Intent | Actual behaviour with `$name = "Off"` |
|---|---|---|
| `target.name = $name` | exact name | ✅ correct — matches the `Off` record |
| `target.fqn = $name` | fully qualified query | inert here |
| `target.fqn CONTAINS $name` | support `Foo.bar` fragments | ❌ matches `…InventoryDedupeTests.OfflineSlot`, `…ParkedLinkInventoryTests.OfflineKeyboardArrival` |
| `(target.name + signature) CONTAINS $name` | support `accept(List` fragments | ❌ matches `IsEligible(DateTimeOffset …)`, `Decide(…, DateTimeOffset now)` |

Both faulty clauses are **unanchored** `CONTAINS`, and they are **OR-ed with**
the exact match rather than used as a fallback. Consequently a perfectly
unambiguous exact hit never suppresses the fuzzy hits.

### 2.3 Secondary defect — per-bucket inconsistency

The ladder is evaluated independently inside each of the six relationship
queries. A name can therefore match "exactly" in the `calls` bucket and
"fuzzily" in the `references` bucket, so the `### Target:` groups rendered by
`format_references_result` (`src/cli_tools/find_callers.rs:59-113`) can describe
different target sets per section — with nothing in the output telling the
reader that happened.

### 2.4 Note on `find_callers` (the other trait method)

`QueryExt::find_callers` (`query.rs:302`) uses only
`callee.name = $name OR callee.fqn = $name`. It is exact and not affected. The
noisy path is `find_references`, which is what both the MCP tool
(`src/mcp_tools/find_callers.rs:110`) and the CLI `knot callers` subcommand
(`src/bin/knot.rs:76`) actually call through
`cli_tools::run_find_callers` (`src/cli_tools/find_callers.rs:16`).

### 2.5 Blast radius

Query layer only. **No re-indexing is required** to deploy this fix — a
significant operational advantage worth preserving in the design.

---

## 3. Goals and Non-Goals

### Goals

- G1. An exact-name or exact-FQN hit **suppresses** all fuzzy matching.
- G2. Signature-fragment queries (`accept(List`) keep working, but anchored so
  they cannot match a substring buried in a parameter type.
- G3. Qualified fragments (`GestureOwner.Off`, `Config::load`) resolve by FQN
  **suffix**, not by unanchored containment.
- G4. All six relationship buckets report against **one** consistent target set.
- G5. When a fuzzy fallback is used, the response says so explicitly, so an LLM
  consumer can calibrate its confidence.
- G6. No re-index required; no change to node properties or edges.

### Non-Goals

- Ranking/scoring of results (out of scope; deterministic ordering is enough).
- Changing `search_hybrid_context` semantics — semantic discovery legitimately
  wants fuzziness.
- Fixing indexer-side false edges (see the companion C# spec).

---

## 4. Design — Two-Stage Resolution

### 4.1 Stage 1: resolve the query string to a target set

A new, single query resolves `entity_name` to a set of target UUIDs, using a
**precedence ladder with early stop**: the first tier that returns a non-empty
result wins, and no later tier runs.

| Tier | Name | Predicate | Rationale |
|---|---|---|---|
| T1 | `ExactFqn` | `target.fqn = $name` (only when `$name` contains `.` or `::`) | Caller gave a fully qualified name — the most precise input possible |
| T2 | `FqnSuffix` | `target.fqn ENDS WITH '.' + $name` **or** `ENDS WITH '::' + $name` | Qualified fragment: `GestureOwner.Off`, `Config::load`. Anchored at a separator so `Off` cannot match `OfflineSlot` |
| T3 | `ExactName` | `target.name = $name` | The common case. **This is the tier that fixes the reported bug**: `Off` stops here with exactly one target |
| T4 | `SignaturePrefix` | only when `$name` contains `(` → `(target.name + COALESCE(target.signature,'')) STARTS WITH $name` | Preserves the documented `accept(List` workflow, anchored at the start so `DateTimeOffset` can never match |
| T5 | `Fuzzy` | only when `$name.len() >= MIN_FUZZY_LEN` (4) and all previous tiers are empty → the current unanchored `CONTAINS` pair | Last-resort discovery for typo'd/partial names; explicitly flagged in the output |

Applied to the bug report: `"Off"` reaches T3 and stops → 1 target, 0 noise.
`"Offlin"` would fall through to T5 (length 6, no exact hit) and be reported as
a fuzzy result.

`MIN_FUZZY_LEN = 4` is a named constant with a doc comment; it exists to stop
2–3 character queries (`Id`, `Off`, `New`, `Get`) from carpet-bombing the graph
when they genuinely have no exact match.

### 4.2 Stage 2: relationship queries by UUID

The six relationship queries stop embedding the name predicate and match on the
resolved set instead:

```cypher
MATCH (entity:Entity)-[:CALLS]->(target:Entity)
WHERE target.uuid IN $target_uuids
RETURN entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature,
       target.name AS target_name, target.fqn AS target_fqn,
       target.file_path AS target_file_path,
       target.start_line AS target_start_line, target.signature AS target_signature
ORDER BY target.fqn, entity.file_path, entity.start_line
```

Benefits beyond correctness:

- Consistency across buckets (G4) — one target set, six projections.
- `target.uuid IN [...]` is backed by the existing `entity_uuid_unique`
  constraint index (`src/db/graph/connection.rs:24`), so the six queries get
  faster, not slower.
- Deterministic ordering removes result churn between runs.

If Stage 1 returns an empty set, Stage 2 is skipped entirely and the existing
"No references found … may be unused" message is returned (with the tier
information attached, so the caller can tell "not found" from "found but
unused").

### 4.3 Result cap and disclosure

- `MAX_TARGETS = 25`: if a tier resolves to more targets than that, keep the
  first 25 by FQN and set `truncated: true`. Prevents a T5 fuzzy query from
  fanning out into thousands of relationship rows.
- The resolution metadata travels in the JSON payload so **both** the CLI and
  MCP formatters can render it.

### 4.4 JSON contract extension

`run_find_callers` currently returns the raw `find_references` object with six
array keys. Add a sibling key (purely additive — existing consumers keep
working):

```json
{
  "calls": [ … ], "extends": [ … ], "implements": [ … ],
  "references": [ … ], "overridden_by": [ … ], "overrides": [ … ],
  "resolution": {
    "query": "Off",
    "tier": "exact_name",
    "fuzzy": false,
    "truncated": false,
    "targets": [
      {
        "uuid": "…",
        "name": "Off",
        "fqn": "OpenLogi.Core.Gestures.GestureOwner.Off",
        "kind": "csharp_record",
        "file_path": "src/OpenLogi.Core/Gestures/GestureOwner.cs",
        "start_line": 15
      }
    ]
  }
}
```

`tier` is the snake_case rendering of a new `enum MatchTier { ExactFqn,
FqnSuffix, ExactName, SignaturePrefix, Fuzzy }`.

### 4.5 Output rendering

`format_references_result` (`src/cli_tools/find_callers.rs:25`) gains a
resolution header immediately after the title:

```markdown
# References to `Off`

Resolved to 1 target by exact name match:
- `OpenLogi.Core.Gestures.GestureOwner.Off` (csharp_record) at `src/OpenLogi.Core/Gestures/GestureOwner.cs:15`

Found 7 reference(s) across all relationship types:
```

and, only for `tier == Fuzzy`:

```markdown
> **Fuzzy match** — no entity matched `Offlin` exactly. The 3 target(s) below were
> found by substring match and may be unrelated. Re-run with an exact name or a
> fully qualified name (e.g. `Namespace.Type.Member`) for precise results.
```

and, only when truncated:

```markdown
> **Truncated** — 112 targets matched; showing the first 25 by FQN.
```

The same metadata is surfaced in the table formatter
(`cli_tools::formatters::format_callers_table`, reached via
`utils::format_callers_output`, `src/utils/mod.rs:131`) as a one-line header.
The `OutputFormat::Json` path needs no change — it serialises the payload
verbatim and therefore picks up `resolution` for free.

### 4.6 Tool description update

The MCP description (`src/mcp_tools/find_callers.rs:50-61`) currently reads:

> CRITICAL: For common method names (e.g., 'accept', 'process'), you MUST
> include a signature fragment (e.g., 'accept(List') to prevent thousands of
> irrelevant results.

That instruction exists to compensate for this bug. Replace with an accurate
description of the ladder:

> Matching is precedence-based: exact FQN → FQN suffix (`Type.member`) → exact
> name → signature prefix (`accept(List`) → fuzzy substring. The first tier that
> matches wins, so an exact name never returns fuzzy noise. Pass a qualified name
> (`Namespace.Type.Member`) to disambiguate homonyms. Responses state which tier
> matched and flag fuzzy results explicitly.

Per `AGENTS.md` ("Query tools changes → update tool descriptions"; "Both →
update `.prompt` and `.knot-agent.md`"), mirror the wording in `.prompt`,
`.knot-agent.md`, the `knot callers --help` text, and `README.md`.

---

## 5. Phase 0 — Tests First (the Red phase)

### 5.1 E2E fixtures — a deliberate name collision

The C# suite gains fixtures that reproduce the exact collision pattern of the
bug report. Three of them are already introduced by
`docs/specs/csharp_reference_extraction_fix_plan.md` §4.1 (`GestureOwner.cs`
with the nested `Off` record, `LightingEffect.cs`, `GestureConfig.cs`); this
plan adds one more:

**`tests/testing_files/csharp/Services/UpdateCheck.cs`**

```csharp
namespace MyApp.Services;

/// <summary>Decides whether an update banner should be shown.</summary>
public class UpdateCheck
{
    /// <summary>Contains "Off" only inside the parameter type name.</summary>
    public bool IsEligible(DateTimeOffset publishedAt, DateTimeOffset now) => publishedAt <= now;

    /// <summary>Name starts with "Off" but is unrelated to GestureOwner.Off.</summary>
    public bool OfflineSlot(int slot) => slot < 0;

    public bool Evaluate(DateTimeOffset now) => IsEligible(now, now) || OfflineSlot(1);
}
```

This gives the index, simultaneously:

- one entity named exactly `Off` (`MyApp.Gestures.GestureOwner.Off`),
- one entity whose **FQN contains** `Off` (`MyApp.Services.UpdateCheck.OfflineSlot`),
- two entities whose **signature contains** `Off` (`IsEligible`, `Evaluate` via
  `DateTimeOffset`),
- one enum member named `Off` (`MyApp.ViewModels.LightingEffect.Off`).

### 5.2 E2E assertions — `tests/run_csharp_e2e.sh`, group H

Run through **both** transports, as the suite already does for G31 (MCP
`tools/call` JSON-RPC request + `invoke_cli callers …`).

| ID | Query | Assertion |
|---|---|---|
| H40 | `find_callers("Off", repo)` | Output **contains** `GestureOwner.Off` |
| H41 | `find_callers("Off", repo)` | Output **does not contain** `OfflineSlot` |
| H42 | `find_callers("Off", repo)` | Output **does not contain** `IsEligible` |
| H43 | `find_callers("Off", repo)` | Output contains `exact name match` (tier disclosure) and **not** `Fuzzy match` |
| H44 | `find_callers("GestureOwner.Off", repo)` | Resolves via FQN suffix to exactly the same single target as H40 |
| H45 | `find_callers("MyApp.Gestures.GestureOwner.Off", repo)` | Resolves via exact FQN; identical reference set |
| H46 | `find_callers("IsEligible(DateTimeOffset", repo)` | Signature-prefix tier still finds `IsEligible`; `Evaluate` appears as a *caller*, never as a target |
| H47 | `find_callers("Offlin", repo)` | Falls through to fuzzy, output contains the `**Fuzzy match**` warning and lists `OfflineSlot` |
| H48 | CLI/MCP parity | H40's CLI output and MCP output agree on the target set |

Update the script header count and any tally in `tests/run_all_e2e_fast.sh`.

**Expected Red state:** H41, H42, H43, H44, H47 fail; H40, H45, H46 pass.

Optional cross-language guard (Rust suite, `tests/run_rust_e2e.sh`): assert
that a `Type::method` query resolves via the `::` branch of tier T2.

### 5.3 Unit tests

The ladder and the formatter must be testable **without a live Neo4j**, which
requires extracting two pure functions.

#### `src/db/graph/query.rs` — pure helpers

Extract `fn target_resolution_tiers(name: &str) -> Vec<(MatchTier, &'static str)>`
returning the ordered `(tier, cypher_predicate_fragment)` pairs for a query
string, and `fn relationship_query(rel_label: &str, repo_scoped: bool) -> String`.

1. `test_tier_ladder_order_for_plain_name` — `"Offline"` yields
   `[ExactName, Fuzzy]` — no `ExactFqn` or `FqnSuffix` or `SignaturePrefix` (no `(`).
2. `test_tier_ladder_includes_signature_prefix_when_parenthesised` —
   `"accept(List"` includes `SignaturePrefix` before `Fuzzy`.
3. `test_tier_ladder_omits_fuzzy_for_short_names` — `"Id"` (len 2 < 4) has no
   `Fuzzy` tier.
4. `test_fqn_suffix_predicate_is_separator_anchored` — the generated fragment
   contains `ENDS WITH '.' + $name` and `ENDS WITH '::' + $name`, and contains
   **no** bare `CONTAINS`.
5. `test_signature_predicate_is_prefix_anchored` — fragment uses `STARTS WITH`,
   not `CONTAINS`.
6. `test_relationship_query_matches_on_uuid_set` — generated Cypher contains
   `target.uuid IN $target_uuids` and an `ORDER BY`, and contains no `$name`.
7. `test_relationship_query_repo_scoped_variant` — repo-scoped variant filters
   `target.repo_name`.

#### `src/cli_tools/find_callers.rs` — formatter

8. `test_format_renders_resolution_header` — `resolution.tier == "exact_name"`
   renders `Resolved to 1 target by exact name match` plus the target FQN.
9. `test_format_renders_fuzzy_warning` — `fuzzy: true` renders the
   `**Fuzzy match**` block.
10. `test_format_renders_truncation_notice` — `truncated: true` renders the
    truncation block.
11. `test_format_without_resolution_key_is_unchanged` — a legacy payload with no
    `resolution` key produces byte-identical output to today (backwards
    compatibility; keeps the 10 existing formatter tests meaningful).

#### `src/mcp_tools/find_callers.rs`

12. `test_find_callers_description_documents_tier_ladder` — the description
    mentions the ladder and no longer contains the `you MUST include a
    signature fragment` workaround.

---

## 6. Phase 1 — Implementation

**File:** `src/db/graph/query.rs`

1. Add `pub enum MatchTier { ExactFqn, FqnSuffix, ExactName, SignaturePrefix,
   Fuzzy }` with `as_str()` (snake_case) and a human label
   (`"exact name match"`, `"FQN suffix match"`, …).
2. Add `const MIN_FUZZY_LEN: usize = 4;` and `const MAX_TARGETS: usize = 25;`,
   each with a doc comment stating *why* the value was chosen.
3. Add `target_resolution_tiers` and `relationship_query` (pure, unit-tested).
4. Add `async fn resolve_reference_targets(&self, name, repo_name) ->
   Result<(Vec<TargetRow>, MatchTier, bool /* truncated */)>`: iterate the
   ladder, execute one Cypher `MATCH (target:Entity) WHERE <predicate> RETURN
   target.uuid, target.name, target.fqn, target.kind, target.file_path,
   target.start_line ORDER BY target.fqn` per tier, stop at the first non-empty
   result, apply `MAX_TARGETS`.
5. Rewrite `find_references` (`query.rs:132`) as Stage 1 + Stage 2: one call to
   `resolve_reference_targets`, then the six relationship queries parameterised
   by `$target_uuids`, then attach the `resolution` object.
6. The two `OVERRIDES` queries keep their `*1..` traversal and their
   `entity.uuid <> target.uuid` guard; only the name predicate is replaced by
   the UUID set. For the `overrides` bucket the set applies to the `entity`
   endpoint (that query deliberately swaps the projection —
   `query.rs:242-258`), so the substitution is `entity.uuid IN $target_uuids`
   there.

**File:** `src/db/graph/connection.rs`

7. Add two `TEXT` indexes to `index_statements()` (`connection.rs:22`) so tiers
   T2/T5 are index-backed rather than full scans — Neo4j's `TEXT` index type
   serves `ENDS WITH` and `CONTAINS`:

```rust
"CREATE TEXT INDEX entity_name_text IF NOT EXISTS FOR (e:Entity) ON (e.name)",
"CREATE TEXT INDEX entity_fqn_text IF NOT EXISTS FOR (e:Entity) ON (e.fqn)",
```

Both use `IF NOT EXISTS`, so existing databases pick them up on next startup
with no migration. Add a unit test asserting both statements are present in
`index_statements()` (the module already tests these without a live database).

**Files:** `src/cli_tools/find_callers.rs`, `src/cli_tools/formatters.rs`

8. Render the resolution header / fuzzy warning / truncation notice, guarded by
   `if let Some(resolution) = references.get("resolution")` so legacy payloads
   are untouched (unit test 11).

**File:** `src/mcp_tools/find_callers.rs`

9. Replace the workaround sentence in the tool description; mirror in
   `.prompt`, `.knot-agent.md`, CLI `--help` and `README.md`.

---

## 7. Phase 2 — Validation

| Step | Command | Gate |
|---|---|---|
| 1 | `cargo test --lib graph` | Unit tests 1–7 + index-statement test green |
| 2 | `cargo test --lib cli_tools` / `--lib mcp_tools` | Unit tests 8–12 green |
| 3 | `cargo fmt -- --check` | Clean |
| 4 | `cargo clippy --all-targets --all-features -- -D warnings` | Clean. Note `find_references` already carries two `#[expect(…, reason)]` for length/complexity (`query.rs:127-131`); the two-stage split should let at least one be **removed** — verify and delete any that become unnecessary |
| 5 | `./tests/run_csharp_e2e.sh` | H40–H48 green |
| 6 | `./tests/run_all_e2e_fast.sh` | All suites green (the query layer is shared by every language suite) |
| 7 | `./tests/benchmark_e2e.sh` | No regression; expect an improvement, since Stage 2 matches on an indexed UUID set |
| 8 | Manual: `knot callers Off -r openlogi-net` | Exactly the `GestureOwner.Off` references; no `OfflineSlot`, no `IsEligible` |

---

## 8. Backwards Compatibility

| Consumer | Impact |
|---|---|
| Existing JSON consumers | None — `resolution` is additive; the six arrays keep their shape |
| `OutputFormat::Json` | Gains the `resolution` object automatically |
| Markdown/table formatters | Gain a header; body format unchanged |
| Indexed data | **No re-index required** — query layer only |
| Neo4j schema | Two additive `TEXT` indexes, `IF NOT EXISTS` |
| Behavioural change | Queries that previously returned fuzzy noise alongside an exact hit now return only the exact hit. This is the intended fix and must be called out in `CHANGELOG.md` as a behaviour change, not merely a bugfix |

---

## 9. Risks and Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| A workflow depended on the fuzzy behaviour to find a partially-remembered name | Medium | T5 preserves fuzzy search as a fallback; it only stops overriding exact hits. The fuzzy path is now labelled, making it discoverable rather than silent |
| `MIN_FUZZY_LEN = 4` hides legitimate 3-char symbols (`Off`, `Add`) | Low | Only applies when **no** exact match exists; if `Off` exists it is found at T3. Constant is named, documented and trivially tunable |
| `ENDS WITH` / `CONTAINS` scan cost on large graphs | Medium | The two new `TEXT` indexes; `repo_name` scoping; `MAX_TARGETS` cap; benchmark gate in step 7 |
| Six queries drift out of sync again | Medium | The whole point of `relationship_query()`: one generator, six call sites, unit-tested. Do not re-inline the Cypher |
| Tier ladder interacts badly with the C# indexer fixes | Low | The two specs are disjoint (indexer vs query layer) but share fixtures; run the C# suite after both land |

---

## 10. Deliverables Checklist

- [ ] `MatchTier` enum + `MIN_FUZZY_LEN` / `MAX_TARGETS` constants
- [ ] `target_resolution_tiers()` and `relationship_query()` pure helpers
- [ ] `resolve_reference_targets()` (Stage 1)
- [ ] `find_references` rewritten as Stage 1 + Stage 2 (6 buckets by UUID set)
- [ ] Two `TEXT` indexes in `connection.rs::index_statements()`
- [ ] `resolution` object in the JSON contract
- [ ] Resolution header / fuzzy warning / truncation notice in both formatters
- [ ] Tool description rewritten; `.prompt`, `.knot-agent.md`, `--help`, `README.md` mirrored
- [ ] 1 new E2E fixture (`Services/UpdateCheck.cs`) + assertions H40–H48
- [ ] 12 unit tests
- [ ] Redundant `#[expect(…)]` attributes on `find_references` removed if the split makes them unnecessary
- [ ] `CHANGELOG.md` entry flagged as a behaviour change
- [ ] Full validation matrix (§7) green
