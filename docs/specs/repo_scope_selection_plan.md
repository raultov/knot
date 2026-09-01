# Repository Scope Selection Plan — `ALL` & Multi-Repo Filtering

**Issue:** [raultov/knot#19 — Add new search across all repos option](https://github.com/raultov/knot/issues/19)
**Status:** 📝 Planned (not implemented — this document is the implementation contract)
**Approach:** BDD (E2E scenarios written first, must fail) + TDD (unit tests per phase, red → green)
**Target version:** v1.8.0 — **standalone point release** (resolved in review, see §15;
not folded into a larger feature drop — the doc updates in Phase 8 drive adoption)

---

## 1. Summary

Today every query-facing knot tool (`search_hybrid_context`, `find_callers`, `explore_file`)
accepts a single optional `repo_name` string that restricts the query to exactly one indexed
repository. Users working across several indexed repositories must either omit `repo_name`
(unfiltered — which for semantic search already spans all repos but is undocumented and
unreachable from the CLI, which always falls back to `cfg.repo_name`) or run the tool once
per repository.

This plan adds a **repository scope selector** to the existing `repo_name` parameter and the
CLI `--repo` flag:

| Input                          | Scope meaning                             |
|--------------------------------|-------------------------------------------|
| *(omitted)*                    | Unchanged: unfiltered (all repos) in MCP; `cfg.repo_name` default in CLI |
| `"my-repo"`                    | Exactly one repo (current behavior, unchanged) |
| `"repo-a,repo-b,repo-c"`       | Union of the listed repos (OR semantics)  |
| `"all"` (case-insensitive) or `"*"` | Every indexed repository (explicit sentinel) |

The implementation is deliberately **small at the database layer** (an `IN` list in Neo4j and
a multi-value keyword match in Qdrant) and **concentrated in the parameter-parsing layer**
(a new `RepoScope` model). A key finding that shapes the design:

> **`repo_name = None` already means "all repos" at both DB layers.**
> Qdrant skips the filter (`src/db/vector/search.rs:32`) and Neo4j omits the `WHERE
> repo_name = $repo_name` clause (`src/db/graph/query.rs:261-275`). The feature is therefore
> about *exposing and validating scope syntax*, not about new query machinery.

---

## 2. Background — Current State Audit

### 2.1 Parameter flow (MCP)

```
LLM client
  └─ tools/call { repo_name: "my-repo" }            ← single string, optional
       └─ mcp_tools/<tool>::handle()                ← args.get("repo_name").and_then(|v| v.as_str())
            └─ cli_tools::run_<tool>(..., repo_name: Option<&str>, ...)
                 └─ db::vector::VectorSearchExt::search(..., repo_name: Option<&str>)
                 └─ db::graph::QueryExt::<method>(..., repo_name: Option<&str>)
                      └─ Cypher: "WHERE x.repo_name = $repo_name"  (or clause omitted when None)
```

### 2.2 Affected code inventory

| Layer | File | Location | Today |
|-------|------|----------|-------|
| MCP schema | `src/mcp_tools/search_hybrid_context/mod.rs` | `tool()` L52-60 | `repo_name` string, `minLength:1, maxLength:255` |
| MCP schema | `src/mcp_tools/find_callers.rs` | `tool()` L37-46 | idem |
| MCP schema | `src/mcp_tools/explore_file.rs` | `tool()` L34-43 | idem |
| MCP handle | `src/mcp_tools/search_hybrid_context/mod.rs` | L105 | `args.get("repo_name").and_then(as_str)` |
| MCP handle | `src/mcp_tools/find_callers.rs` | L96 | idem |
| MCP handle | `src/mcp_tools/explore_file.rs` | L88 | idem |
| CLI args | `src/models/cli_args.rs` | `Search/Callers/Explore` L30-31, 43-45, 57-59 | `--repo/-r Option<String>` |
| CLI dispatch | `src/bin/knot.rs` | L53, 74, 86 | `repo.as_deref().unwrap_or(&cfg.repo_name)` — **no way to request "all"** |
| Shared logic | `src/cli_tools/search_hybrid_context.rs` | `run_search_hybrid_context` L31-36; `enrich_with_relationships` L110-115 | `Option<&str>` passthrough |
| Shared logic | `src/cli_tools/find_callers.rs` | `run_find_callers` L17-21 | `Option<&str>` passthrough |
| Shared logic | `src/cli_tools/explore_file.rs` | `run_explore_file` L85-89 | `Option<&str>` passthrough |
| Vector DB | `src/db/vector/search.rs` | `VectorSearchExt::search` L13-19, filter build L32-47 | single `MatchValue::Keyword` |
| Graph DB | `src/db/graph/query.rs` | `relationship_query` L189, `overridden_by_query` L209, `overrides_query` L231 | `x.repo_name = $repo_name` when scoped |
| Graph DB | `src/db/graph/query.rs` | `resolve_reference_targets` L253-321, `collect_reference_rows` L400-423 | param binding `repo_name` |
| Graph DB | `src/db/graph/query.rs` | `get_entities_with_dependencies` L477, `find_references` L547, `find_callers` L617, `get_file_entities` L665, `find_entities_by_name_prefix` L712, `get_file_outgoing_references` L776, `find_files_by_suffix` L834 | `Option<&str>` + duplicated query branches per scope |
| Out of scope (kept as-is) | `src/db/graph/query_subgraph.rs` | `resolve_subgraph_root` L331 (requires `&str`) | subgraph tool requires a single repo — see §13 |
| Out of scope (kept as-is) | `src/cli_tools/deps.rs`, `src/mcp_tools/list_repo_dependencies.rs` | — | `repo_name` is the *subject* of the query, not a filter — see §13 |

### 2.3 Existing tests that pin current behavior (must be updated)

| File | Test | Line | Pin |
|------|------|------|-----|
| `src/db/graph/query.rs` | query-string assertions | ~L1121 (`assert!(query_str.contains("target.repo_name = $repo_name"))`) | exact `=` predicate text |
| `src/mcp_tools/mod.rs` | `test_all_tools_have_optional_repo_name` | L112-125 | property presence only |
| `src/mcp_tools/*/tests` | schema tests | e.g. `search_hybrid_context/mod.rs` L160-172 | property presence only |
| `src/models/cli_args.rs` | `test_cli_parser_*_with_repo` | L121-130, 169-178, 205-214 | single value parsing |

---

## 3. Goals & Non-Goals

### Goals
1. `repo_name` / `--repo` accepts: one repo (unchanged), a comma-separated list, or the
   sentinel `all` (also `*`) — with identical semantics in MCP tools and CLI.
2. Results are the **union** across the selected repos; every result row keeps its
   `(repo: name)` annotation so multi-repo output is unambiguous.
3. Zero breaking changes: single-repo usage behaves byte-for-byte as before; no re-index
   required (no payload/graph schema changes).
4. Deterministic, unit-testable parsing (a pure function, no I/O).

### Non-Goals
- Per-repo result fairness for `max_results` (global limit is kept — see §14).
- Fuzzy/glob repo selection (`scope-*`, regex). Lists are exact names.
- Changing `list_repo_dependencies` (its `repo_name` is the subject, not a filter) or the
  subgraph traversal (single-repo by design).
- A new dedicated MCP parameter (e.g. `repo_scope: enum`) — the sentinel keeps one
  parameter and stays backward compatible with every existing client.

---

## 4. Design

### 4.1 Parameter syntax contract (normative)

`RepoScope::parse(raw: &str) -> RepoScope` is the single authority. Rules, in order:

1. Trim the whole input.
2. Split on `,`.
3. Trim each token; **drop empty tokens**.
4. If **any** remaining token equals `all` (case-insensitive) **or is exactly `*`** →
   `RepoScope::All`.
   *(Rationale: the sentinel wins over any other token; listing `"all,repo-a"` or `"*,repo-a"`
   cannot be interpreted as anything narrower than "everything", so honoring the wider
   request is the least-surprising behavior.)*
5. If no tokens remain (empty input / only separators) → `RepoScope::All`.
6. Deduplicate tokens **preserving first-occurrence order** (deterministic output).
7. One token → `RepoScope::One(token)`; more → `RepoScope::Many(tokens)`.

Additional contract points:

- **Case sensitivity:** repo names match case-sensitively (same as today's exact `=` match).
  Only the `all` sentinel is case-insensitive (`*` is a single character, no case to fold).
- **Unknown repo names do not error.** A list containing an unindexed name simply yields no
  rows for it — identical to today's behavior for a misspelled single repo.
- **Empty string** → `All` (MCP schema discourages it via `minLength: 1`, but the runtime
  degrades gracefully instead of erroring).
- **Arrays:** the MCP layer additionally accepts a JSON array of strings
  (`"repo_name": ["repo-a", "repo-b"]`), concatenated in order before parsing. This costs
  three lines and spares clients that model multi-select as arrays.
- **`*` IS a sentinel** (resolved in review, §15): exactly `*` means "all repos".
  It is **not** a glob — `"scope-*"` is treated as a literal repo name that simply matches
  nothing. Caveat documented: a repository literally named `*` cannot be selected
  individually (practically impossible: repo names come from the last path component,
  which cannot be `*` on any supported platform).

### 4.2 The `RepoScope` model

New file **`src/models/repo_scope.rs`**, re-exported from `src/models/mod.rs`:

```rust
/// Repository scope selected by a tool caller.
///
/// `All`   — no repository filter (every indexed repo).
/// `One`   — exactly one repository (preserves today's behavior exactly).
/// `Many`  — union of the listed repositories (OR semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoScope {
    All,
    One(String),
    Many(Vec<String>),
}

impl RepoScope {
    /// Parse a raw parameter string (see §4.1 for the normative rules).
    pub fn parse(raw: &str) -> Self;

    /// Parse an optional parameter. `None` → `All` (MCP "omitted" semantics).
    /// The CLI resolves its own `cfg.repo_name` default *before* calling this.
    pub fn parse_optional(raw: Option<&str>) -> Self;

    /// Build from a JSON argument value: string (comma-separated) or array of strings.
    /// Returns `None` when the key is absent or null.
    pub fn from_json(value: Option<&serde_json::Value>) -> Self;

    /// Repository names to bind as `$repo_names`. Empty for `All`
    /// (DB layers treat "empty list" as "no filter").
    pub fn filter_names(&self) -> Vec<String>;

    /// `true` when no filter should be applied at the DB layer.
    pub fn is_unfiltered(&self) -> bool;
}
```

`filter_names()` mapping: `All → []`, `One(n) → vec![n]`, `Many(v) → v.clone()`.

### 4.3 DB-layer standardization: `Option<&str>` → `&[String]` (empty = unfiltered)

The DB layers adopt one convention — **an empty slice means "no repository filter"** —
replacing `Option<&str>`. This *deletes* the ~10 duplicated per-scope query branches in
`query.rs` (every method currently carries an `if repo_name.is_some() { } else { }` pair of
nearly identical Cypher strings) and makes `IN $repo_names` the single predicate form.
`IN` over a 1-element list is semantically and performance-wise equivalent to `=` (both hit
the same property index), so single-repo queries do not regress.

#### 4.3.1 Qdrant (`src/db/vector/search.rs`)

```rust
// BEFORE
async fn search(&self, vector: &[f32], limit: usize, repo_name: Option<&str>)
    -> Result<Vec<serde_json::Value>>;

// AFTER
async fn search(&self, vector: &[f32], limit: usize, repo_names: &[String])
    -> Result<Vec<serde_json::Value>>;
```

Filter construction is extracted into a pure, unit-testable helper:

```rust
/// Build the Qdrant payload filter for a repo scope.
/// Empty slice → `None` (unfiltered). 1 name → `MatchValue::Keyword`.
/// N names → `MatchValue::Keywords(RepeatedStrings)` ("value in set" OR semantics).
pub(crate) fn build_repo_filter(repo_names: &[String]) -> Option<qdrant_client::qdrant::Filter>;
```

`MatchValue::Keywords` is available in the pinned `qdrant-client = "1"` (verified: variant
`Keywords(RepeatedStrings)` exists in `qdrant-client-1.17.0/src/qdrant.rs`, tag 8 of
`r#match::MatchValue`). The `repo_name` payload field already has a **Keyword index**
(created by `src/db/vector/connection.rs:68-80`), so multi-value matching stays indexed.

#### 4.3.2 Neo4j (`src/db/graph/query.rs`)

Every `x.repo_name = $repo_name` becomes `x.repo_name IN $repo_names`; every
`.param("repo_name", repo)` becomes `.param("repo_names", repo_names.to_vec())`;
every `repo_name.is_some()` guard becomes `!repo_names.is_empty()`. Method-by-method:

| Method (query.rs) | Predicate today | Predicate after |
|---|---|---|
| `relationship_query` (L189) | `target.repo_name = $repo_name AND target.uuid IN $uuids` | `target.repo_name IN $repo_names AND target.uuid IN $uuids` |
| `overridden_by_query` (L209) | idem | idem |
| `overrides_query` (L231) | `entity.repo_name = $repo_name AND ...` | `entity.repo_name IN $repo_names AND ...` |
| `resolve_reference_targets` (L253) | `target.repo_name = $repo_name` per-tier branch | single branch, `IN` fragment appended when scoped |
| `collect_reference_rows` (L400) | binds `repo_name` when `Some` | binds `repo_names` when non-empty |
| `get_entities_with_dependencies` (L477) | two query branches | single branch + optional `AND m.repo_name IN $repo_names` |
| `find_references` (L547) | passes scope down | `repo_names: &[String]` |
| `find_callers` (L617) | `callee.repo_name = $repo_name` | `callee.repo_name IN $repo_names` |
| `get_file_entities` (L665) | `{file_path: $file_path, repo_name: $repo_name}` | map stays single-valued when scoped with 1 name; when scoped with N names → `WHERE e.file_path = $file_path AND e.repo_name IN $repo_names` (node-map form cannot express `IN`) |
| `find_entities_by_name_prefix` (L712) | `AND m.repo_name = $repo_name` | `AND m.repo_name IN $repo_names` |
| `get_file_outgoing_references` (L776) | `WHERE dst.file_path <> $fp OR dst.repo_name <> $repo_name` | `... OR NOT dst.repo_name IN $repo_names` |
| `find_files_by_suffix` (L834) | `AND e.repo_name = $repo_name` | `AND e.repo_name IN $repo_names` |

Trait `QueryExt` (L431-473): all seven trait methods change
`repo_name: Option<&str>` → `repo_names: &[String]`.

`neo4rs` supports `Vec<String>` as a Cypher list parameter (already used for
`target_uuids` / `$target_uuids` at query.rs:191), so no new dependency machinery.

#### 4.3.3 Untouched DB code

- `resolve_subgraph_root` (`query_subgraph.rs:331`, takes `&str`) — subgraph requires a
  single repo by contract (`.knot-agent.md` / graph skill); unchanged.
- `delete.rs`, `upsert.rs`, `connection.rs` — indexer-side, unchanged.
- `find_repo_dependencies` / `find_repo_dependents` (`query_repo.rs`) — `Repository` graph
  subject, unchanged.

### 4.4 Shared CLI-tools layer

Signatures change from `Option<&str>` to `&RepoScope`; conversion to `&[String]` happens at
the call into the DB layer:

```rust
// BEFORE
pub async fn run_search_hybrid_context(query: &str, max_results: usize,
    repo_name: Option<&str>, ctx: &SearchContext<'_>) -> anyhow::Result<serde_json::Value>;

// AFTER
pub async fn run_search_hybrid_context(query: &str, max_results: usize,
    repo: &RepoScope, ctx: &SearchContext<'_>) -> anyhow::Result<serde_json::Value>;
```

- `src/cli_tools/search_hybrid_context.rs` — `run_search_hybrid_context` (L31) and
  `enrich_with_relationships` (L110) take `&RepoScope`; both call
  `repo.filter_names()` before touching the DB.
- `src/cli_tools/find_callers.rs` — `run_find_callers` (L17) takes `&RepoScope`.
- `src/cli_tools/explore_file.rs` — `run_explore_file` (L85) takes `&RepoScope`. The suffix
  fallback (`find_files_by_suffix`, L111) receives the same scope; `ambiguous_path_candidates`
  therefore respects the selected scope with no extra work. **Resolved in review (§15):**
  candidates are deduplicated *within* the scope — the DB query filters by the already
  deduplicated `filter_names()` list, so `"a,a"` can never yield duplicate candidates.

No formatter changes: `format_file_line` (`src/cli_tools/mod.rs:94`) already annotates every
path with `(repo: name)` from per-row data, so cross-repo output is self-labeling.

### 4.5 MCP tools layer

New shared helper in **`src/mcp_tools/mod.rs`**:

```rust
/// Extract the repository scope from tool-call arguments.
/// Accepts: absent/null → All · string (comma-separated, "all"/"*" sentinel) → parse
/// · array of strings → joined then parsed.
pub(crate) fn repo_scope_from_args(
    args: &serde_json::Map<String, serde_json::Value>,
) -> RepoScope;
```

Each of the three tools replaces its one-line extraction:

```rust
// BEFORE (search_hybrid_context/mod.rs:105, find_callers.rs:96, explore_file.rs:88)
let repo_name = args.get("repo_name").and_then(|v| v.as_str());

// AFTER
let repo = repo_scope_from_args(&args);
```

Schema descriptions are updated so LLM clients discover the syntax (all three tools get the
same parameter description text):

> *"Optional but HIGHLY RECOMMENDED: repository scope. Accepts a single repository name
> (`'my-repo'`), a comma-separated list (`'repo-a,repo-b'`), or `'all'` (or `'*'`) to query
> every indexed repository. If you know the repository you are working on, include it in your
> FIRST query to avoid mixed results from other indexed projects. Omit to search across
> all repositories."*

`list_repo_dependencies` and `list_repositories` schemas are unchanged.

### 4.6 CLI layer

- `src/bin/knot.rs` (L53, 74, 86): the default stays `cfg.repo_name` **only when the flag is
  absent** — but `all` / `*` / lists now bypass it:

  ```rust
  // BEFORE
  let target_repo = repo.as_deref().unwrap_or(&cfg.repo_name);

  // AFTER
  let target_repo = repo.as_deref()
      .map(RepoScope::parse)                                  // explicit: "all", "a,b", "a"
      .unwrap_or_else(|| RepoScope::One(cfg.repo_name.clone())); // implicit default
  ```

- `src/models/cli_args.rs`: `repo: Option<String>` type is unchanged (clap needs no
  value_delimiter — `RepoScope::parse` owns splitting, so quoted
  `--repo "scope_alpha,scope_beta"` works and a hypothetical repo name containing a comma
  stays expressible via array in MCP only). Doc comments updated:
  `/// Repository scope: one name, comma-separated list, or 'all'/'*'`.
- Deliberately **no** separate `-a/--all` flag — `--repo all` (or `--repo '*'`) is the
  single spelling.

### 4.7 Tool coverage matrix

| Tool | `repo_name` role | v1 change |
|------|------------------|-----------|
| `search_hybrid_context` | filter | ✅ full scope support |
| `find_callers` | filter | ✅ full scope support |
| `explore_file` | filter + ambiguity disambiguator | ✅ full scope support |
| `list_repo_dependencies` | subject of query | ❌ unchanged (phase-2 candidate) |
| `list_repositories` | n/a (`filter` is a substring) | ❌ unchanged |

---

## 5. BDD Specification (E2E scenarios — written FIRST, must fail)

Gherkin is the *specification of record* for the E2E suite of §8. The suite must be created
and observed **red** before any production change lands (AGENTS.md: "E2E tests written
first, must fail before logic implemented").

```gherkin
Feature: Repository scope selection
  As an LLM agent or developer querying knot
  I want to select one, several, or all indexed repositories per query
  So that I can analyze cross-repository code without repeating per-repo calls

  Background:
    Given two indexed repositories "scope_alpha" and "scope_beta" in one collection
      And "scope_alpha" contains class "AlphaSearchService" and entity "SharedUtil"
       with method "work" called by "alphaCaller"
      And "scope_beta" contains class "BetaSearchService" and entity "SharedUtil"
       with method "work" called by "betaCaller"
      And both repos contain a file at relative path "src/index.ts"

  # --- Sentinel: all -------------------------------------------------------

  Scenario: Search with sentinel "all" returns hits from every indexed repo
    When MCP search_hybrid_context is called with query "SharedUtil" and repo_name "all"
    Then the response mentions "SharedUtil" in scope_alpha
     And the response mentions "SharedUtil" in scope_beta

  Scenario: Sentinel is case-insensitive
    When MCP search_hybrid_context is called with repo_name "ALL"
    Then the response contains hits from scope_alpha and scope_beta

  Scenario: Star sentinel "*" means all repositories
    When MCP search_hybrid_context is called with repo_name "*"
    Then the response contains hits from scope_alpha and scope_beta

  Scenario: Star wins over any other token in a list
    When MCP search_hybrid_context is called with repo_name "scope_alpha,*"
    Then the response contains hits from scope_alpha and scope_beta

  Scenario: Omitted repo_name searches across all repositories (MCP)
    When MCP search_hybrid_context is called without repo_name
    Then the response contains hits from scope_alpha and scope_beta

  # --- Comma-separated list ------------------------------------------------

  Scenario: Comma-separated list unions the listed repos
    When MCP search_hybrid_context is called with repo_name "scope_alpha,scope_beta"
    Then the response contains hits from scope_alpha and scope_beta

  Scenario: List restricts to exactly the listed repos
    When MCP search_hybrid_context is called with repo_name "scope_alpha"
    Then the response contains hits from scope_alpha
     And the response does not contain hits from scope_beta

  Scenario: Whitespace around tokens is tolerated
    When MCP search_hybrid_context is called with repo_name " scope_alpha , scope_beta "
    Then the response contains hits from scope_alpha and scope_beta

  Scenario: Unknown repo in a list degrades gracefully (no error, no rows for it)
    When MCP search_hybrid_context is called with repo_name "scope_alpha,scope_gamma"
    Then the call succeeds
     And the response contains hits from scope_alpha only

  Scenario: Duplicate tokens are collapsed
    When MCP search_hybrid_context is called with repo_name "scope_alpha,scope_alpha"
    Then the call succeeds and results equal the single-repo call for scope_alpha

  # --- find_callers across scopes ------------------------------------------

  Scenario: find_callers with "all" surfaces callers from every repo
    When MCP find_callers is called with entity_name "SharedUtil.work" and repo_name "all"
    Then the Calls bucket lists "alphaCaller" (scope_alpha) and "betaCaller" (scope_beta)

  Scenario: find_callers with a two-repo list matches the "all" result for these repos
    When MCP find_callers is called with repo_name "scope_alpha,scope_beta"
    Then the Calls bucket lists both "alphaCaller" and "betaCaller"

  # --- explore_file across scopes ------------------------------------------

  Scenario: explore_file ambiguous relative path without repo lists both candidates
    When MCP explore_file is called with file_path "src/index.ts" and no repo_name
    Then the response contains "ambiguous_path_candidates" with 2 entries
     And each entry names its repository

  Scenario: explore_file list scope disambiguates to the listed repos
    When MCP explore_file is called with file_path "src/index.ts"
      and repo_name "scope_beta"
    Then the response resolves to scope_beta "src/index.ts" without ambiguity

  # --- JSON array form (MCP only) -------------------------------------------

  Scenario: repo_name provided as a JSON array is accepted
    When MCP search_hybrid_context is called with repo_name ["scope_alpha","scope_beta"]
    Then the response contains hits from scope_alpha and scope_beta

  # --- CLI parity ------------------------------------------------------------

  Scenario: CLI --repo with a list unions repos
    When the CLI is run: knot search "SharedUtil" --repo "scope_alpha,scope_beta"
    Then the output contains hits from scope_alpha and scope_beta

  Scenario: CLI --repo all bypasses the working-directory default
    When the CLI is run: knot search "BetaSearchService" --repo all
    Then the output contains hits from scope_beta
     And no result is restricted to the auto-detected repo

  Scenario: CLI single --repo remains a strict filter (regression guard)
    When the CLI is run: knot search "SharedUtil" --repo scope_beta
    Then the output contains hits from scope_beta only

  Scenario: CLI without --repo keeps the auto-detected default (regression guard)
    When the CLI is run from scope_alpha's directory: knot search "SharedUtil"
    Then the output contains hits from scope_alpha only

  # --- Output labeling ---------------------------------------------------------

  Scenario: Multi-repo results are self-labeling
    When MCP search_hybrid_context is called with repo_name "all"
    Then every returned file path is annotated with its owning "(repo: <name>)"
```

---

## 6. TDD Implementation Phases

Each phase = red (write failing tests) → green (minimal implementation) → refactor.
Phases 1-6 are compile-coupled (a signature change ripples); the plan sequences them so
**unit tests for layer N are written before layer N's implementation**, and the crate is
kept compiling at every phase boundary by applying mechanical call-site updates together
with each signature change.

### Phase 0 — BDD red: E2E suite skeleton (no production changes)

**Deliverables**
- `tests/testing_files/repo_scope/scope_alpha/` and `scope_beta/` fixtures (§8.2).
- `tests/run_repo_scope_e2e.sh` implementing the §5 scenarios (§8.3-8.4).
- Registration in `tests/run_all_e2e_fast.sh` `SUITES` array (after
  `run_cross_repo_dep_e2e.sh`, index 15) and in `AGENTS.md` quick commands.

**Red criterion:** the suite fails at the sentinel/list scenarios (MCP currently treats
`"scope_alpha,scope_beta"` as one literal repo name → 0 hits; `all` → 0 hits). The
regression-guard scenarios (single repo, omitted repo) already pass — that is expected and
documents current behavior.

**Commit:** `test(e2e): add repo-scope BDD suite (red) — issue #19`

### Phase 1 — `RepoScope` model (pure unit TDD)

**Files:** `src/models/repo_scope.rs` (new), `src/models/mod.rs` (re-export).

**Tests first** (`src/models/repo_scope.rs`, `#[cfg(test)]`):

| Test name | Assertion |
|---|---|
| `parse_empty_is_all` | `RepoScope::parse("")` → `All` |
| `parse_whitespace_only_is_all` | `"   "` → `All` |
| `parse_all_sentinel` | `"all"` → `All` |
| `parse_all_sentinel_case_insensitive` | `"ALL"`, `"All"` → `All` |
| `parse_star_sentinel` | `"*"` → `All` |
| `parse_star_wins_over_list` | `"a,*"` → `All` |
| `parse_star_is_not_a_glob` | `"scope-*"` → `One("scope-*")` (literal, matches nothing) |
| `parse_all_wins_over_list` | `"all,repo-a"` → `All` |
| `parse_single_repo` | `"my-repo"` → `One("my-repo")` |
| `parse_multi_repo` | `"a,b,c"` → `Many(["a","b","c"])` |
| `parse_multi_trims_tokens` | `" a , b "` → `Many(["a","b"])` |
| `parse_multi_drops_empty_tokens` | `"a,,b,"` → `Many(["a","b"])` |
| `parse_multi_dedups_preserving_order` | `"b,a,b"` → `Many(["b","a"])` |
| `parse_preserves_repo_case` | `"MyRepo"` → `One("MyRepo")` (not lowercased) |
| `parse_optional_none_is_all` | `parse_optional(None)` → `All` |
| `parse_optional_some_parses` | `parse_optional(Some("a,b"))` → `Many` |
| `from_json_string` | `json!("a,b")` → `Many` |
| `from_json_array` | `json!(["a","b"])` → `Many` |
| `from_json_absent_is_all` | `None` / `json!(null)` → `All` |
| `from_json_non_string_items_skipped` | `json!(["a", 42])` → `One("a")` |
| `filter_names_empty_for_all` | `All.filter_names()` → `[]` |
| `filter_names_one_and_many` | mapping correctness |
| `is_unfiltered_only_for_all` | `All` → true; `One`/`Many` → false |

**Green:** implement §4.2 (~90 lines + tests).

**Commit:** `feat(models): RepoScope parser for all/list/single repo selection — issue #19`

### Phase 2 — Qdrant multi-value filter (unit TDD)

**Files:** `src/db/vector/search.rs`.

**Tests first** (same file; pure functions, no Qdrant needed):

| Test name | Assertion |
|---|---|
| `build_repo_filter_empty_is_none` | `&[]` → `None` |
| `build_repo_filter_single_keyword` | `["a"]` → `Some(Filter)` with `must[0]` = `MatchValue::Keyword("a")` on key `"repo_name"` |
| `build_repo_filter_multi_keywords` | `["a","b"]` → `MatchValue::Keywords(values == ["a","b"])` |
| `build_repo_filter_preserves_order` | `["b","a"]` → keywords in that order |

**Green:**
1. Add `build_repo_filter`.
2. Change `VectorSearchExt::search` signature to `repo_names: &[String]`
   (empty = no filter) and delegate filter construction to the helper.
3. Mechanically update the caller: `cli_tools/search_hybrid_context.rs:44-47`
   (compiles again at Phase 4; to keep the crate green *now*, temporarily pass
   `&repo.filter_names()` — Phase 4 finalizes).

Existing `#[ignore]` live-Qdrant tests in the file keep compiling with trivial arg updates
(`&[]`, `&["test-repo".into()]`).

**Commit:** `feat(db): multi-repo keyword filter for Qdrant search — issue #19`

### Phase 3 — Neo4j `IN $repo_names` (test-string TDD)

**Files:** `src/db/graph/query.rs` (+ trait in same file), call sites in
`cli_tools/*` updated mechanically to keep the crate green.

**Tests first** — update the existing query-string unit tests (query.rs `mod tests`,
~L896-1393) *before* touching the builders:

| Test (existing → renamed) | New assertion |
|---|---|
| `relationship_query_*` | contains `target.repo_name IN $repo_names` when scoped; no `$repo_names` when unscoped |
| `overridden_by_query_*` / `overrides_query_*` | idem for `entity.repo_name IN $repo_names` |
| `find_callers_query_*` | `callee.repo_name IN $repo_names` |
| `get_file_entities_query_*` | node-map form for single name; `IN` form for list |
| `get_file_outgoing_references_query_*` | `NOT dst.repo_name IN $repo_names` |
| `find_files_by_suffix_query_*` | `e.repo_name IN $repo_names` |
| **new** `param_binding_uses_repo_names_list` | `collect_reference_rows` binds `repo_names` as `Vec<String>` |

**Green:** apply the §4.3.2 table. Where a branch pair collapses
(`get_entities_with_dependencies`, `resolve_reference_targets`), delete the dead branch —
the dedup *is* the refactor step.

**Commit:** `refactor(db): unify repo filtering on repo_name IN $repo_names — issue #19`

### Phase 4 — Shared CLI-tools layer (signature ripple)

**Files:** `src/cli_tools/{search_hybrid_context.rs, find_callers.rs, explore_file.rs}`.

- Replace `Option<&str>` with `&RepoScope` in `run_search_hybrid_context`,
  `run_find_callers`, `run_explore_file`, `enrich_with_relationships`.
- Each fn computes `let repo_names = repo.filter_names();` once and passes `&repo_names`
  to every DB call.
- `run_search_hybrid_context` returns early (`Ok(Value::Null)`) unchanged when the combined
  result set is empty — now also correct for scope-filters that match nothing.
- Unit tests: the layer is passthrough; add one compilation-contract test per fn asserting
  `RepoScope::All.filter_names().is_empty()` flows through (cheap, no mocking — DB
  behavior is covered by Phase 3 unit tests + E2E).

**Commit:** `refactor(cli-tools): thread RepoScope through search/callers/explore — issue #19`

### Phase 5 — MCP tools (schema + parsing TDD)

**Files:** `src/mcp_tools/mod.rs` (helper + tests), the three tool modules.

**Tests first:**

| File | Test | Assertion |
|---|---|---|
| `mcp_tools/mod.rs` | `repo_scope_from_args_absent_is_all` | `{}` → `All` |
| | `repo_scope_from_args_string_all` | `{"repo_name":"all"}` → `All` |
| | `repo_scope_from_args_string_list` | `{"repo_name":"a,b"}` → `Many` |
| | `repo_scope_from_args_array` | `{"repo_name":["a","b"]}` → `Many` |
| | `repo_scope_from_args_null_is_all` | `{"repo_name":null}` → `All` |
| `mcp_tools/mod.rs` | `test_scope_descriptions_mention_all_and_lists` (extends `test_all_tools_have_optional_repo_name` L112) | each of the 3 schemas' `repo_name` description contains `"'all'"` and `"comma-separated"` |
| `search_hybrid_context/mod.rs` | `test_search_schema_repo_name_documents_scope` | description updated |
| `find_callers.rs` / `explore_file.rs` | idem | idem |

**Green:** add `repo_scope_from_args` (§4.5), swap the three extraction lines, update the
three `tool()` descriptions verbatim from §4.5.

**Commit:** `feat(mcp): all/list repo scope in search_hybrid_context, find_callers, explore_file — issue #19`

### Phase 6 — CLI (arg-parsing TDD)

**Files:** `src/models/cli_args.rs`, `src/bin/knot.rs`.

**Tests first** (cli_args.rs `mod tests`):

| Test name | Assertion |
|---|---|
| `test_cli_parser_search_with_repo_list` | `--repo "a,b"` → `Some("a,b")` (raw string; scope parsing is `RepoScope`'s job) |
| `test_cli_parser_search_with_repo_all` | `--repo all` → `Some("all")` |
| `test_cli_parser_callers_with_repo_list` / `explore` idem | raw passthrough |

Plus one dispatch-level unit test in `src/bin/knot.rs` `mod tests` (file already has an
inline test module):

| Test name | Assertion |
|---|---|
| `repo_flag_builds_scope_list` | flag `"a,b"` → `Many` via the new helper fn `build_repo_scope(repo: Option<&str>, default: &str)` |
| `repo_flag_builds_scope_all` | flag `"all"` → `All` (bypasses default) |
| `repo_flag_absent_uses_config_default` | `None` → `One(cfg.repo_name)` |

**Green:** extract `build_repo_scope` helper in `knot.rs` (tested) and use it at L53/74/86;
update `cli_args.rs` doc comments.

**Commit:** `feat(cli): --repo accepts comma-separated list and 'all' — issue #19`

### Phase 7 — E2E green

Run `./tests/run_repo_scope_e2e.sh`; fix integration fallout only (expected: none beyond
Qdrant eventual-consistency retries already handled by `retry_match`). Then:

- `./tests/run_all_e2e_fast.sh` — full regression (21 suites).
- Pay special attention to `run_groovy_e2e.sh` and `run_cross_repo_dep_e2e.sh` (multi-repo
  suites) and to `run_config_e2e.sh`/`run_k8s_helm_e2e.sh` (config entity kinds pass
  through the same `QueryExt` methods).

**Commit:** `test(e2e): repo-scope suite green; register as 21st suite — issue #19`

### Phase 8 — Documentation

| File | Change |
|---|---|
| `README.md` | §CLI examples (~L76-78, 345-357): add `--repo "a,b"` / `--repo all` rows; tools table (~L660s) unchanged |
| `.prompt` | L45-48 (explore disambiguation) + L83 ("All tools support optional `repo_name` …") — document sentinel & list syntax |
| `.knot-agent.md` | L54, 75, 108 (command signatures), L181-200 (usage examples), L236 (perf tip) |
| `AGENTS.md` | quick-commands section: add `./tests/run_repo_scope_e2e.sh` |
| `mcp_handler.rs` | server `instructions` string (L143-149): add one line documenting scope syntax |
| `docs/specs/multilanguage_roadmap.md` | new phase entry + changelog bullet (repo convention) |

**Commit:** `docs: repository scope selection (all / list) across tools — issue #19`

---

## 7. File-by-file change list (complete)

| # | File | Change type | Summary |
|---|------|-------------|---------|
| 1 | `src/models/repo_scope.rs` | **new** | `RepoScope` enum + parse/from_json/filter_names + ~20 unit tests |
| 2 | `src/models/mod.rs` | edit | `pub mod repo_scope;` + re-export |
| 3 | `src/db/vector/search.rs` | edit | signature `&[String]`; extract `build_repo_filter`; `MatchValue::Keywords` path; update `#[ignore]` tests |
| 4 | `src/db/graph/query.rs` | edit | `IN $repo_names` everywhere; `&[String]` on `QueryExt` (7 methods) + internals; collapse duplicate branches; update query-string tests |
| 5 | `src/cli_tools/search_hybrid_context.rs` | edit | `&RepoScope` on `run_search_hybrid_context` + `enrich_with_relationships` |
| 6 | `src/cli_tools/find_callers.rs` | edit | `&RepoScope` on `run_find_callers` |
| 7 | `src/cli_tools/explore_file.rs` | edit | `&RepoScope` on `run_explore_file` (suffix fallback inherits scope) |
| 8 | `src/mcp_tools/mod.rs` | edit | `repo_scope_from_args` helper + tests; extend `test_all_tools_have_optional_repo_name` |
| 9 | `src/mcp_tools/search_hybrid_context/mod.rs` | edit | parse via helper; schema description |
| 10 | `src/mcp_tools/find_callers.rs` | edit | idem |
| 11 | `src/mcp_tools/explore_file.rs` | edit | idem |
| 12 | `src/models/cli_args.rs` | edit | doc comments + parsing tests |
| 13 | `src/bin/knot.rs` | edit | `build_repo_scope` helper; use at 3 dispatch sites |
| 14 | `tests/testing_files/repo_scope/scope_alpha/*` | **new** | fixture repo (§8.2) |
| 15 | `tests/testing_files/repo_scope/scope_beta/*` | **new** | fixture repo (§8.2) |
| 16 | `tests/run_repo_scope_e2e.sh` | **new** | BDD suite (§8) |
| 17 | `tests/run_all_e2e_fast.sh` | edit | register suite in `SUITES` (L41) |
| 18 | `README.md`, `.prompt`, `.knot-agent.md`, `AGENTS.md`, `src/mcp_handler.rs`, `docs/specs/multilanguage_roadmap.md` | edit | §Phase 8 |

Not changed: `src/db/graph/{query_subgraph,query_repo,delete,upsert,connection}.rs`,
`src/cli_tools/{deps,repos,subgraph}.rs`, `src/mcp_tools/{list_repo_dependencies,list_repositories}.rs`,
`src/pipeline/**`, `src/config.rs`, indexer binaries.

---

## 8. E2E Suite Design (`tests/run_repo_scope_e2e.sh`)

### 8.1 Conventions

- **Template:** `tests/run_varnish_e2e.sh` (the KNOT_SKIP_BUILD-aware reference suite) with
  the `call_mcp` / `call_cli` / `retry_match` helpers copied from `tests/run_groovy_e2e.sh`
  (L137-198).
- **Shared ephemeral DB:** honours `KNOT_E2E_EXTERNAL_DB` like every suite; standalone runs
  use `docker-compose.e2e.yml` (ports 17687 / 16334 / 16333).
- **One collection, two repos:** `KNOT_QDRANT_COLLECTION="knot_repo_scope_e2e"`;
  `scope_alpha` and `scope_beta` indexed sequentially with `KNOT_REPO_NAME` set per run
  (identical to `run_cross_repo_dep_e2e.sh:192/247`).

### 8.2 Fixtures (TypeScript — no build system required)

```
tests/testing_files/repo_scope/
├── scope_alpha/
│   ├── index.ts                  # re-export (target of the explore ambiguity test)
│   └── src/
│       ├── alpha_service.ts      # class AlphaSearchService { find(): string }
│       └── shared_util.ts        # class SharedUtil { work(): number }
│                                 # function alphaCaller() { new SharedUtil().work() }
└── scope_beta/
    ├── index.ts                  # same relative path as alpha (ambiguity test)
    └── src/
        ├── beta_service.ts       # class BetaSearchService { save(): string }
        └── shared_util.ts        # class SharedUtil { work(): number }  (same names,
                                  #  different file body → distinct embeddings/UUIDs)
                                  # function betaCaller() { new SharedUtil().work() }
```

Design notes:
- Distinctive entity names (`AlphaSearchService`, `BetaSearchService`) make search
  assertions deterministic via the exact-name prefix boost
  (`find_entities_by_name_prefix`, query.rs:712).
- The **homonym pair** `SharedUtil.work` in both repos is what makes scope actually
  observable: with a single-repo filter only one caller appears; with `all`/list both do.
- Identical relative path `src/index.ts` in both repos exercises the
  `ambiguous_path_candidates` flow (`cli_tools/explore_file.rs:110-123`).

### 8.3 Assertion groups (map 1:1 to §5 scenarios)

| Group | Scenarios | Mechanism |
|---|---|---|
| A — sentinel | all / ALL / `*` / omitted | MCP + CLI, `retry_match` on both repo names |
| B — list | union, restriction, whitespace, unknown-repo, duplicates | MCP + CLI |
| C — find_callers | `SharedUtil.work` with all & list | MCP, assert both callers; single-repo guard asserts exactly one |
| D — explore_file | ambiguity without scope; resolution with scope | MCP, parse `ambiguous_path_candidates` |
| E — array form | JSON array `repo_name` | MCP only |
| F — CLI parity | `--repo list`, `--repo all`, single, default | `call_cli` |
| G — labeling | every result row carries `(repo: …)` | grep on formatted output |

Count: **~20 executable assertions** (MCP and CLI validated identically per repo
convention).

### 8.4 Determinism

- All search assertions go through `retry_match` (10 attempts, 1 s) for Qdrant eventual
  consistency (v1.3.5 stabilization pattern).
- Graph assertions (`find_callers` buckets) are eventually consistent only right after
  indexing → single `sleep 5` after the second `run_indexer`, then straight queries.
- Entity names are unique enough that no fuzzy tier can shadow assertions; if a name
  collision ever appears, switch the `find_callers` calls to FQN form
  (`scope_alpha::src::shared_util::SharedUtil::work`) which pins tier 1 of the resolution
  ladder.

---

## 9. Edge Cases & Decisions Register

| # | Decision | Rationale |
|---|---|---|
| D1 | Sentinel is the token `all` (case-insensitive) or exactly `*`, inside the existing `repo_name` string; sentinel wins over any other token in the same list | single param stays backward compatible with every current client & prompt; `*` added per review (§15). Caveat: a repo literally named `*` is unselectable (impossible in practice — names come from path components). `"scope-*"` is a literal name, not a glob |
| D2 | `,` separator parsed by `RepoScope`, not by clap `value_delimiter` | quoted `--repo "a,b"` keeps working; splitting logic lives in exactly one place |
| D3 | `all` wins over any other token in the same list | `"all,x"` narrower than `all` is never the caller's intent |
| D4 | Unknown repo names are silent no-rows | matches today's single-repo misspelling behavior; `list_repositories` exists for discovery |
| D5 | Repo names case-sensitive; only `all` is folded | preserves exact-match semantics of the current index |
| D6 | DB convention "empty slice = unfiltered" (not `Option`) | deletes ~10 duplicated Cypher branches; `IN` with 1 element ≡ `=` (same index plan) |
| D7 | Qdrant `MatchValue::Keywords` under a single `must` FieldCondition | "value in set" OR semantics; stays on the `repo_name` Keyword payload index |
| D8 | Global `max_results` across the union (no per-repo quotas) | v1 simplicity; documented follow-up (§14) |
| D9 | `list_repo_dependencies` unchanged | `repo_name` is the query subject, not a filter; a list would need per-repo result grouping (phase 2) |
| D10 | Subgraph traversal unchanged | single-repo contract by design (`SubgraphQueryParams.repo_name: &str`) |
| D11 | MCP also accepts a JSON array for `repo_name` | three lines; spares array-native clients |
| D12 | No dedicated `--all` CLI flag | one spelling (`--repo all`); smaller help surface |
| D13 | `repo_name: ""` → All at runtime (schema `minLength:1` still discourages it) | graceful degradation over hard error |
| D14 | `explore_file`'s `ambiguous_path_candidates` are filtered **and deduplicated** by the scope | resolved in review (§15, question 2): candidates come from the DB query, which filters by the already-deduplicated `filter_names()` list — `"a,a"` can never yield duplicate candidates |

---

## 10. Backward Compatibility

- **Wire format:** `repo_name` remains a single optional string in all tool schemas. Old
  clients sending one name behave identically (bit-for-bit responses modulo row order,
  which is already non-guaranteed).
- **No re-index:** no payload field, node property, index, or FQN change;
  `CURRENT_STATE_VERSION` untouched.
- **CLI:** positional args and `-r/--repo` unchanged; only *new* accepted values.
- **Prompts/instructions:** `.prompt` consumers (LLM agents) keep working; the updated
  descriptions are additive guidance.

## 11. Performance

- Qdrant: `MatchValue::Keywords` matches against the existing `repo_name` Keyword payload
  index (`connection.rs:68-80`) — no scan regression expected vs single keyword.
- Neo4j: `x.repo_name IN $list` is an index-supported predicate; `IN` with one element
  produces the same plan as `=` on all supported Neo4j versions.
- `get_entities_with_dependencies` remains a per-UUID loop — unchanged complexity; scope
  only narrows matches.
- Qdrant collection stays single (multi-tenant by `repo_name` payload) — the design that
  motivated the Keyword index in the first place.

## 12. Risks & Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Signature ripple breaks a suite only visible at E2E time | medium | Phase 3 keeps crate green mechanically; Phase 7 runs all 21 suites, not just the new one |
| Qdrant `Keywords` variant mismatch in an older client | low | pinned `qdrant-client = "1"` resolves to 1.17.x where the variant exists (verified); `build_repo_filter` unit tests pin the wire shape |
| LLM agents keep sending single repos (feature undiscovered) | medium | schema descriptions + `.prompt`/`.knot-agent.md` updated in Phase 8 (this is exactly the audience of issue #19) |
| Multi-repo search results feel biased toward one repo (global limit) | medium | documented in tool description ("increase `max_results` when using multi-repo scope") + §14 follow-up |

## 13. Out of Scope (follow-up issues)

1. **`list_repo_dependencies` multi-repo** — accept a list → grouped per-repo result
   object; `all` → iterate `list_repositories()`.
2. **Per-repo result fairness** — `max_results` per selected repo (fan-out + merge) for
   `search_hybrid_context`.
3. **Glob/regex repo selection** (`"scope-*"`).
4. **Subgraph multi-repo traversal.**
5. **`knot deps` accepting lists** (CLI symmetry with #1).

## 14. Validation Checklist (release gate)

```bash
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test                                   # includes ~30 new unit tests
./tests/run_repo_scope_e2e.sh                # new suite, ~20 assertions
./tests/run_all_e2e_fast.sh                  # full regression, 21 suites
```

No `unsafe`, no `#[allow(...)]`; any unavoidable lint gets `#[expect(..., reason = "...")]`
per repo policy. README updated in the same PR.

## 15. Resolved Questions (review 2026-08-31 — closed before Phase 1)

1. **Should the sentinel also accept `"*"`? → YES.** `RepoScope::parse` folds exactly `*`
   into `All` alongside case-insensitive `all`. The sentinel wins over any other token
   (`"a,*"` → All). `*` is not a glob (`"scope-*"` is a literal name). Folded into §4.1
   (rule 4), §4.5 (schema text), §4.6 (CLI), §5 (2 new scenarios), Phase 1 tests
   (`parse_star_sentinel`, `parse_star_wins_over_list`, `parse_star_is_not_a_glob`),
   §8.3 (group A) and D1.
2. **Should `explore_file`'s `ambiguous_path_candidates` deduplicate within the scope? → YES.**
   Guaranteed by construction: candidates come from `find_files_by_suffix`, which filters by
   the deduplicated `filter_names()` list. Recorded as D14; §4.4 updated.
3. **Fold into v1.8.0 with the next feature, or standalone point release? → STANDALONE.**
   Shipped as its own point release (v1.8.0). Rationale: the schema descriptions and docs
   (Phase 8) are what actually drive adoption by LLM agents — the release should carry
   nothing but this feature plus its documentation.
