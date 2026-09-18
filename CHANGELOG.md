# Changelog

All notable changes to **knot** are documented here, ordered from most recent to oldest.
For the upcoming roadmap see [README.md → Upcoming](README.md#-roadmap).

---

## v1.10.0

- **Mandatory Re-index (State v6)**: `IndexState` bumped from v5 to **v6**. Embed text schema now leads with a natural-language role sentence (`<identifier_phrase>: <first_sentence(docstring)>` or `<identifier_phrase> — calls <top_callee_names>`). Older indexes are incompatible and force `knot-indexer --clean`.
- **Embedding Model Selection (`KNOT_EMBED_MODEL`)**: Configurable embedding model support via `KNOT_EMBED_MODEL` (supports `AllMiniLML6V2` [default], `BGESmallENV15`, `BGEBaseENV15`, `MultilingualE5Small`, `JinaEmbeddingsV2BaseCode`, `NomicEmbedTextV15`).
- **Asymmetric Prefix Management**: Automatic query/passage instruction prefixing for asymmetric models (e.g. `BGE`, `E5`, `Nomic`). Fixed `embed_query` in fastembed 6 to correctly prepend model query prefixes.
- **Model Dimension Guard**: `validate_embed_pair` ensures `KNOT_EMBED_DIM` matches the selected model's native dimension at startup, preventing mid-query Qdrant dimension mismatches.
- **Consumer Library Warning**: Consumers built against previous versions of `knot` produce old-model query vectors against v6 passages. Dimensions match, so there is **no error — only silent recall loss**. Re-index and rebuild together.
- **Entry-Point Cosine Harness**: Added `tests/measure_entrypoint_cosine.sh` for exact measurement of raw cosine similarity vs. window cutoff across benchmark queries. Detailed evaluation docs committed in `docs/measurements/` and TDD plan in `docs/specs/entrypoint_recall_tdd_plan.md`.

---

## v1.9.8

- **Fixed**: residual entry-point recall failures after the v1.9.7 fix — the
  cases where the ranker was promised the right inputs but the search
  **pipeline** did not produce them. The v1.9.7 unit regressions fed the
  ranker corrected annotations by construction; five of the seven reported
  rows were therefore still failing live (measured with
  `RUST_LOG=search_hybrid_context::rank=debug` before the fix: knot
  `run_search_hybrid_context` and C# `GetCallersAsync` absent from the
  whole pool, TS `screenshot` swept out by neutral-kind helpers, Java
  `getConnection` at pool #9 behind config accessors, JS `login` halved by
  its own UI wrapper). Query-time only; no re-indexing.
- **Definition channel**: the cosine window is not proportional to its
  candidate *mix*. On documentation-heavy repositories (indexing-prose
  repos ran as high as 85% Markdown rows) the scanned window filled with
  Markdown before a single definition entered, leaving the root-set seed
  (`TOP_ROOTS = 8`) nothing but two trivial roots and the entry-point
  signal dead — the v1.9.7 fix silently weakened exactly where it was
  needed. `CandidatePool::collect` now runs a second bounded Qdrant pass
  excluding all non-code kinds (`PROSE` + `CONFIG_BUILD` + `K8S_HELM`
  deny-list, exported as `kinds::non_code_kinds()`); skipped when the
  caller's `kinds` filter is already code-only (identical query) or
  docs-only (a documentation-scoped search must never be shadowed by a
  code channel).
- **Root seed = union of channels**: the bridge seed was the raw cosine
  order, so config accessors and prose outranked the behavioral helpers a
  paraphrase actually names. Seeds are now selected by cosine order from
  the *merged* pool (cosine ∪ definition channels) with a probe back-fill
  capped at [`PROBE_ROOTS_MAX = 3`], so a lexically plausible but
  unverified near-lock helper cannot crowd out the verified roots.
- **Caller bridge, depth 2**: a new bounded Cypher
  (`caller_links_depth2_query`) recalls callers-of-callers
  (`(caller)-[:CALLS]->(helper)-[:CALLS]->(root)`) when the direct
  round left pool capacity unused — entry points reaching a top root
  through exactly one helper now surface at all. The Cypher stays anchored
  on the seed UUID set at both ends (no fan-out-driven expansion) and the
  Neo4j row budget is `TOP_ROOTS * 16`.
- **Neutral-kind behavioral boost**: a definition the taxonomy scores
  neutral (`constant`, …) whose graph node orchestrates ≥ 2 outgoing CALLS
  edges earns +0.10 — language-agnostic (kind neutrality + call degree;
  nothing per-language or per-kind). Reason it must exist (measured,
  chrome-devtools-mcp): a TypeScript MCP tool is
  `export const screenshot = defineTool({...})` — neutral kind, zero
  boost — yet it held the *highest code cosine* of its window and was
  swept below a method helper purely on the callables' kind advantage.
  Prose/config/test kinds and sub-degree nodes never take the boost.
- **Provenance strictly better, not equal-or-bigger**: the coverage boost
  can only be attenuated by a production caller that covers *strictly
  more* roots AND carries the entry-point signature itself (≥ 2 distinct
  roots). Previously an equal-coverage caller halved the boost (job-watch-ui:
  the `LoginPage` view displacing the `login` function it calls), and any
  single-rung caller claimed provenance.
- **Name-prefix contract scoped to definitions**: prose, config/build and
  k8s/helm *prefix hits* no longer take leading slots the ranker never
  gave them (measured: a Markdown section titled exactly `take screenshot`
  ranked #1 before the re-rank ran). They enter the candidate pool with
  their true cosine and compete under the standard kind-aware re-rank;
  documentation-only topics (no competing definition) still surface their
  best section — provenance changed, not reachability (new Markdown E2E
  case). Test-path prefix hits keep their slot: the demotion is a
  metadata decision, and re-applying the test-path penalty there would
  punish legitimate fixtures living under `tests/…`-named trees (the web
  E2E fixture `test_angular.html` is the measured case); the structural
  test-path gates inside the re-rank are unchanged.
- **Name/token probe widened to 48 results**: at 24 the probe's lexical
  set truncated before deep-cosine interface *implementations*
  (csharp-code-map `GetCallersAsync` in `QueryEngine`), so only the
  interface declaration (zero CALLS edges) ever entered the pool.
- **Coverage rows now carry `out_degree`** (a third quantity in the same
  bounded round trip): the total outgoing CALLS degree of the entity, the
  input of the neutral-kind boost.
- **Diagnostic provenance**: every pool row is tagged with its recall
  channel (`cosine`/`definition`/`probe`/`bridge`) and the rank trace
  exposes `channel` + `out_degree`; the internal marker is stripped before
  results leave the search.
- **`tests/run_rank_recall_live.sh`** (opt-in, NOT part of
  `run_all_e2e_fast.sh`): measures the reported rows' positions against a
  *live* indexed knot instance (skipped cleanly when credentials/repositories
  are absent). This is the harness that exposed that the previous E2E
 /unit fixtures could pass while the live index failed: unit regressions
  validate the ranker given correct inputs; the live harness validates the
  pipeline produces them.
- **Documented limits measured, not fixed**: two reported rows remain
  unreached and are recorded here with their numeric evidence — Rust knot
  `run_search_hybrid_context` and C# `QueryEngine.GetCallersAsync` under
  `find who invokes…` are both outside the cosine window and have no
  call-path to the seeded roots; the entities' indexed docstrings are
  decorative (`─── Name ───`) or absent. Closing them requires an
  *indexing-time* change (inheriting a container/module docstring when the
  entity's own is empty or decorative), deliberately out of scope for a
  query-time fix.

## v1.9.7

- **Fixed**: entry-point recall for loose paraphrases in `search_hybrid_context`.
  For a natural-language query that shares no tokens with a definition's
  name, the definition previously ranked 3rd–4th — or dropped out of the
  top-6 entirely — behind its own helpers, constants, long docstrings or
  prose; an entity merely *named* after a generic verb in the query
  (`ToolRegistry.Find` for "find who invokes…", `SuspendResumeLock.acquire`
  for "acquire a client…", `ConcurrentBag.borrow` for "borrow a
  connection…") could take #1. Query-time only; no re-indexing.
- **Root cause 1 (silent coverage void)**: `CandidatePool::collect` hashed
  every cosine hit into `seen_uuids` *before* the caller-recall bridge ran,
  and `CallerBridge::run` skipped any caller already in that set — so
  `caller_roots` was only ever annotated on entities the vector search had
  NOT returned. The shared entry point of the top-ranked helpers (which the
  cosine pool does return) received exactly `0.0` root evidence. The bridge
  is now pure recall, and a new bounded Neo4j query
  (`QueryExt::fetch_root_coverage`, depth fixed at 2 hops, both patterns
  anchored on UUID sets) annotates the whole pool: FQN (absent from the
  Qdrant payload and needed by the generic-verb guard), direct root
  coverage and helper-hop coverage.
- **Root cause 2**: seed too narrow for the wide ladder. `TOP_ROOTS` was 3;
  the entry point's strongest helpers often ranked 4th–8th in pure cosine
  — widening to 8 (capped by the pool window) gives the coverage signal its
  full root set, with the caller-link Cypher budget scaled accordingly.
  Scoring rewards root-set *coverage* over raw counts: weights per root
  (saturating at 3), an entry-point signature bonus at ≥ 2 distinct roots,
  a transitive term for orchestration-through-one-helper, and a
  covered-fraction tie-break ("2 of 3 beats 1 of 5"), halved when a pool
  caller covers a superset of the same root set (internal steps of an outer
  orchestrator must not ride the signature).
- **Root cause 3**: `EXACT_NAME_BOOST` (0.30) paid the full boost to any
  entity whose whole name equals a *generic* query verb/noun. The boost is
  now attenuated (0.08) unless the entity's FQN corroborates a second query
  token — which is exactly why `LookupMaps::build` and
  `ChatClient.create` keep full strength. No re-indexing: the FQN is
  fetched at query time by the same coverage annotation.
- Performance: one search now costs up to 2 extra bounded Neo4j round
  trips; the E2E benchmark medians stay under 200 ms total per search.
- TypeScript E2E fixture expectations updated to the corrected contract
  (`submitChangePassword` — the orchestrator calling the three top-ranked
  helpers — ranks #1; the doc-less hook `useChangePassword` stays within
  position 2). Historically the hook ranked #1 only while the caller-root
  bridge silently skipped every entity the cosine pool already returned.

## v1.9.6

- **Fixed**: `find_callers` fuzzy target resolution no longer surfaces
  documentation/config/build/infra metadata as resolved targets. Target
  resolution is code-only by default (`markdown_section`, `config_property`,
  `build_dependency`, `cargo_package`, `cargo_feature`, `project_identity`,
  `k8s_*`, `helm_*` … can never be presented as "Resolved to N targets").
  `knot callers cargo` no longer fills the list with Cargo.toml metadata.
  Every non-code match is *disclosed* (`Non-code matches hidden — N entities …`)
  instead of silently dropped; `kinds=all` restores the unfiltered view and an
  explicit allow-list scopes it. New optional `kinds` parameter on both the
  CLI (`knot callers … --kinds`) and the MCP tool.
- **Fixed**: fuzzy matching is case-insensitive — `find_callers hikari` now
  matches `Hikari`-named artifacts (exact tiers stay exact).
- **Fixed**: 11 relationship types the pipeline produces were never queried by
  `find_references` and thus never shown by `find_callers`: `MACRO_CALLS`,
  `REFERENCES_DOM`, `USES_CSS_CLASS`, `IMPORTS_SCRIPT`, `IMPORTS_STYLESHEET`,
  `USES_BACKEND`, `USES_PROBE`, `USES_ACL`, `INCLUDES`, `IMPORTS_VMOD`,
  `DECLARED_UNUSED` — a macro crate reporting "This entity may be unused"
  while its call sites existed. Buckets are now filled from a single
  multi-labelled query (also fewer Neo4j round-trips); the outgoing-reference
  query of `explore_file` gained the same edge types. `CONTAINS` (containment)
  and `DEPENDS_ON` (repository-level, served by `knot deps`) stay excluded,
  and `GENERIC_BOUND` was never built by the pipeline — noted as dead enum
  variant.
- **Fixed (JS parser)**: the `dom.element_id` / `css.class_name` captures of
  `javascript.scm` fire on the string argument alone, so the per-match
  mechanism could never attach them to an entity and the graph received zero
  `REFERENCES_DOM` / `USES_CSS_CLASS` edges (#web). A new JavaScript post
  pass (`collect_dom_css_references`) detects `getElementById` /
  `querySelector*` / `classList.*` / `className =` call sites from the AST
  and attaches the intents to the enclosing entity (or the synthetic
  `<module>` for top-level statements).
- **Fixed**: `explore_file` no longer returns "No entities found in this file"
  for a path that is not in the index. Two honest outcomes: no indexed file
  matched (with suffix-fallback candidates listed as "Did you mean" when
  available), or explicitly "file indexed, zero entities". The JSON payload
  carries `file_matched: bool` for programmatic consumers.
- Shared entity-kind taxonomy module (`cli_tools::kinds`) shared by
  `search_hybrid_context` ranking and the `find_callers` kind scope.

---

## v1.9.5

- Search `max_results` (1–100) and Deps `max_depth` (1–10) bound enforcement across CLI & MCP.
- `list_files` read-only file enumeration MCP tool & `knot files` CLI subcommand.
- `path` filter support on `search_hybrid_context` and CLI `knot search`.
- `search_hybrid_context` recall improvements: embed text carries FQN, identifier tokens, and call names; token-level lexical probe & caller-recall bridge.
- Search ranking improvements: definition entities outrank prose, tests, and config; optional `kinds` filter added.
- `knot deps` empty-result diagnostics enhanced and reverse lookups made transitive while honoring `--depth`.
- `find_callers` target attribution bug fix for callerless homonyms; npm cross-repo linking made bidirectional with persistent repository identity.
- Truncation explicit & quantified notices in `find_callers` and `search_hybrid_context`.
- Fixed Qdrant delete filter substring bug preventing sibling repository deletion.
- Fixed deterministic target ordering in `find_callers` Markdown output.

---

## v1.9.4 — Deterministic `find_callers` Target Grouping

Fix release: ensures `find_callers` Markdown output rendered by the CLI and MCP tools is fully deterministic across process runs and cluster nodes.

- **Fixed**: `find_callers` Markdown output is now deterministic. The
  `### Target:` sections inside each relationship bucket were grouped in a
  `HashMap` and rendered in process-random order (SipHash keys are seeded per
  process), so the same query against the same graph could render targets in
  a different order across runs and across nodes. The grouping now uses a
  `BTreeMap` keyed by `target_file_path:start_line`, pinning a stable sorted
  order. Pinned by `test_format_references_result_target_order_is_deterministic`.

---

## v1.9.3 — Runtime-free MCP Tool Surface for Embedders

Additive release: the MCP tool table and tool dispatch become public,
runtime-free API on `KnotMcpHandler`, so embedders (knot-server's HTTP
`/mcp` endpoint) can serve the exact same tool surface as `knot-mcp`
over stdio without duplicating it. No behavioural change for MCP clients;
existing indexes remain valid.

- **Added**: `KnotMcpHandler::tools()` and `KnotMcpHandler::dispatch()`:
  runtime-free access to the MCP tool surface for library embedders.
  `tools()` is a free function of no state (usable before any database
  connection exists); `dispatch()` behaves identically to
  `handle_call_tool_request`, whose `Arc<dyn McpServer>` runtime argument
  was never read by any tool.
- **Added**: `DRY_RUN_MESSAGE` public constant — the error text returned by
  `dispatch` when the handler was built with `new_dry_run()`.
- **Changed**: the `ServerHandler` methods now delegate to the new API
  (`handle_list_tools_request` → `Self::tools()`,
  `handle_call_tool_request` → `self.dispatch()`); no behavioural change
  for MCP clients. Adding a tool to knot now makes it visible on every
  surface at once, by construction rather than by convention.
- **Tests**: new unit tests pin the tool surface, the delegation invariants
  (source-level drift guards), dry-run refusal for every tool, unknown-tool
  error mapping, and guard ordering (dry-run checked before the offline
  database guard).

---

## v1.9.2 — Dependency Upgrades + MCP SDK 1.1 LTS Migration

Maintenance release: bring the dependency tree onto current upstream
crates and migrate `knot-mcp` to `rust-mcp-sdk` 1.1 (LTS, MCP protocol
2025-11-25). No public-API or behaviour change; existing indexes remain
valid.

- **Chore(deps)**: bumped transitive dependencies to current versions:
  - `tree-sitter` 0.26 → 0.27, `tree-sitter-css` 0.23 → 0.25,
    `tree-sitter-python` 0.23 → 0.25, `tree-sitter-c` 0.23 → 0.24,
    `sha2` 0.10 → 0.11, `fastembed` 5 → 6, `toml` 0.8 → 1.1,
    `criterion` 0.4 → 0.8.
  - `notify-debouncer-mini` 0.4.1 → 0.7.0. Removed the pinned
    `notify = "6.1.1"` direct dep and rely on the SDK re-export
    `notify_debouncer_mini::notify` to avoid two concurrent copies of
    `notify` (v6 + v8) in the dependency graph.
- **Fix(parser)**: tree-sitter 0.27 turned `QueryMatch::captures` from a
  public field into an accessor method (`QueryMatch::captures()`).
  Updated both call sites in `src/pipeline/parser/extractor/mod.rs`
  and `src/pipeline/parser/languages/cpp.rs` (test helper). Removed
  unnecessary `i as u32` casts at the parser sites now that the
  indexing into `Node::child(...)` accepts `usize` directly; clippy
  flagged these as `unnecessary_cast`.
- **Fix(hash)**: `sha2` 0.11 / `digest` 0.11 swapped `generic-array`
  for `hybrid-array`, whose `Output` no longer implements
  `std::fmt::LowerHex`. Replaced `format!("{:x}", hash)` with an
  explicit `std::fmt::Write` loop in `src/pipeline/state.rs` so the
  persisted hash format (64-char lowercase hex) is byte-for-byte
  unchanged — existing `.knot/index_state.json` files remain valid.
- **Refactor(mcp)**: migrated `knot-mcp` to `rust-mcp-sdk = "1.1"`.
  Replaced the hand-rolled `HashMap` + `serde_json::json!` JSON schemas
  for the five tools (`search_hybrid_context`, `find_callers`,
  `explore_file`, `list_repositories`, `list_repo_dependencies`) with
  the SDK's `#[mcp_tool]` procedural macro and `derive(JsonSchema)`.
  The macro generates `Tool::tool()` and the schema from the struct's
  field-level `#[json_schema(...)]` attributes, keeping the advertised
  schema and the documented behaviour in lockstep. Added MCP tool
  annotations: `read_only_hint = true`, `idempotent_hint = true`,
  `destructive_hint = false`, `open_world_hint = false` for all five
  tools. The schema crate upgrade (0.13 → 1.0) also moved
  `ToolInputSchema::properties` from `HashMap` to `BTreeMap` for
  deterministic JSON output; the macro generates the right map type
  directly, so no manual conversion is required.
- **Refactor(benches)**: `criterion` 0.8 deprecated
  `criterion::black_box` in favour of `std::hint::black_box`. Updated
  the four benchmarks to use the stdlib re-export.
- **Chore(lint)**: retired 8 `#[expect(clippy::cognitive_complexity)]`
  attributes whose functions had already been simplified below the
  cognitive-complexity threshold (20) by previous refactors. The two
  remaining `#[expect(clippy::too_many_arguments)]` markers
  (`update_index_state`, `clean_stale_data`) are preserved — extraction
  is on the Phase-3 backlog.

---

## v1.9.1 — Refactor: Duplicate-Code Backlog & `cargo-dupes` Quality Gate

Internal code-health release: cleared 8 of the 16 genuine-duplication
items from the §11 backlog of `docs/specs/duplication_quality_gate.md`,
and shipped a fifth quality gate (`cargo-dupes` 0.2.1) that blocks new
duplicate code at PR time. No public-API or behaviour change; existing
indexes remain valid.

- **Refactor(parser)**: Extracted shared helpers that collapse 8 pairs of
  near-identical functions into single implementations. `parser/utils.rs`
  gained `convert_call_intents_to_reference_intents`,
  `collect_recursive_call_intents`, `find_node_value_recursive`;
  `extractor/captures.rs` gained `promote_captured_entity`; `gradle.rs`
  gained `extract_gradle_property(key)`; `utils/mod.rs` gained
  `setup_logging_subscriber` and `format_output_with_handlers`;
  `calls.rs` `lookup_fqn_by_fqn` now delegates to `lookup_fqn`. Affected
  call-sites: Java, Kotlin, C++, TypeScript, JavaScript, Gradle,
  CSS/SCSS/HTML captures, ingest FQN lookup, CLI output formatting.
- **Feat(quality)**: Added `cargo-dupes` duplication quality gate.
  `dupes.toml` defines a ratcheted baseline (`max_exact_duplicates = 8`,
  `max_near_duplicates = 9`, `min_lines = 10`, `exclude = tests/ +
  benches/ + tests.rs`) so the gate is green on the current tree and may
  only ever be lowered. `.dupes-ignore.toml` documents 5 known false
  positives (3 Cypher query builders in `db/graph/query.rs`, 2
  `rust-mcp-sdk` `Tool::tool` schema declarations). `Makefile` adds
  `make check` (fmt + clippy + test + dupes) and `make dupes-cleanup`
  targets. `.github/workflows/ci.yml` and `.github/workflows/release.yml`
  cache, install, run `cargo dupes check`, and surface stale ignores via
  `cargo dupes cleanup --dry-run` inside the existing `test-unit` job.
- **Docs**: Added `docs/specs/duplication_quality_gate.md` (full design
  spec, measured curves, Phase 2 backlog); `AGENTS.md` documents the gate
  and the duplicate-suppression policy next to the `#[expect(..., reason)]`
  rule; `CONTRIBUTING.md` adds the gate to the contributor checklist and
  the install one-liner to prerequisites; `README.md` documents the gate
  in the development section.

---

## v1.9.0 — Refactor: Cognitive Complexity Reduction & Threshold Tradeoff

Internal code-health release: refactored 11 high cognitive complexity functions into modular helpers, fixed a Groovy non-ASCII byte boundary panic, raised `cognitive-complexity-threshold` from 15 to 20, retired 20 obsolete `#[expect(clippy::cognitive_complexity)]` attributes, and documented an auditable expectation policy rule (`score - 7 * tracing_macros <= 20`).

- **Refactor(complexity)**: Refactored 11 complex parser and ingest functions into clean, focused helpers: `collect_declarations` (16→5), `parse_acl` (27→3), `k8s_spec_summary` & `extract_k8s_references` (16/16→4/3), `extract_single_call_intent_javascript` (17→5), `extract_method_calls` (18→3), `strip_comments_line` (17→8), `extract_decorators` (29→2), `link_method_overrides` (20→6), `extract_brace_block` (20→6), `parse_if_statement` (25→3).
- **Fix(groovy)**: Corrected character boundary indexing in `read_word` (`groovy/refs.rs`) for words ending in non-ASCII characters (e.g. `// está`), resolving a panic during Groovy source scanning.
- **Test**: Added unit test coverage for Java, TypeScript, C# decorators, VTC embedded brace blocks, `.bind(this)` & `this.prop` JavaScript call intents, Groovy non-ASCII comment handling, and unnamed Kubernetes ports. Fixed three JavaScript call intent unit tests that were silently passing.
- **Chore(config)**: Raised `cognitive-complexity-threshold` from 15 to 20 in `clippy.toml`. Documented the exact `+7` score inflation per `tracing` macro call and established the auditable policy rule. Cleaned up 20 obsolete `#[expect]` attributes and updated reasons for the remaining 8 macro-dominated functions.

---

## v1.8.3 — Refactor: Split Oversized Functions into Helpers

Internal code-health release: every function exceeding 100 lines was
split into helpers (no public-API or behaviour change), the
`too_many_lines` clippy threshold was raised to the upstream default
of 100, and all 41 `#[expect(clippy::too_many_lines, …)]` opt-outs were
retired. Reasoning is in `clippy.toml` (measured curves under the
thresholds). No schema changes, no re-index required for already
indexed repositories.

- **Refactor(extractor)**: `process_capture` (421 lines) — the central
  tree-sitter capture dispatcher — split into one private handler per
  capture-name family (`handle_class_captures`, `handle_method_captures`,
  `handle_kotlin_decl_captures`, `handle_kotlin_function_capture`,
  `handle_function_captures`, `handle_css_scss_capture`,
  `handle_html_capture`, `handle_rust_capture`, `handle_python_capture`,
  `handle_markdown_capture`, `handle_dom_css_ref_capture`,
  `handle_groovy_capture`, `handle_csharp_capture`,
  `handle_cpp_captures`). Dispatcher drops to a flat match; each
  handler holds ≤5 params and ≤100 lines. `tracing/log`-induced
  lifetime noise resolved with named lifetimes on the shared capture
  context.
- **Refactor(parser)**: `parse_single_file` (298 lines) — the per-file
  format dispatcher — split into `FileCtx` state + per-grammar
  dispatchers (`dispatch_query_lang`, `dispatch_html_lang`,
  `dispatch_config_lang`, `dispatch_lexical_lang`). Dispatch is now
  data-driven via the shared context; early returns for filename-based
  detection (`Jenkinsfile`, `Directory.Packages.props`) preserved.
- **Refactor(ingest)**: `resolve_reference_intents_with_context` and its
  per-intent resolution moved into phase helpers:
  `LookupMaps::build` / `context()` for the 7 lookup tables;
  `resolve_intent` (the central dispatch), `resolve_plain`,
  `resolve_typed` (the dot-`::`-path-qualified family), plus the
  container-related helpers and the `push_relationship` filter for
  nested-decl self-references + dedup. The alias-redirection step was
  pulled into the caller so `resolve_intent` stays pure. `aliases.rs`'s
  `build_alias_map` split into `index_file_entities`,
  `build_file_defaults`, `resolve_entity_aliases`, and the cycle
  collapser `collapse_alias_cycles` (its terminal calculation lifted to
  `chain_terminal`). `calls.rs`'s `resolve_single_call_intent` split
  into a dispatcher that chains `resolve_super_call`,
  `resolve_implicit_this_call`, `resolve_static_receiver_call`,
  `resolve_receiver_call`, `resolve_bare_call` — with the `super`
  branch kept terminal so unresolved `super()` never falls through to
  the bare-name fallback (regression test
  `test_python_super_init_no_parent_returns_no_edge`).
- **Refactor(pipeline)**: `run_pipeline_inner` (264 lines) — the indexing
  pipeline orchestrator — split into `classify_pipeline_inputs` (file
  discovery + classification), `run_streaming_ingest` (the parse →
  embed → ingest stages), `spawn_parse_stage`,
  `spawn_embed_task` + `embed_and_forward` + `maybe_reset_session`,
  `spawn_ingest_task`, plus two log-only helpers
  (`log_nothing_to_index`, `log_file_classification`,
  `log_parse_and_ingest_complete`) extracted to keep the
  `tracing/log` macro cost out of the control-flow score. `PipelineInputs`
  carries the per-file pack.
- **Refactor(db)**: `get_entity_subgraph` (268 lines) split into
  `is_query_unsupported_check`, `resolve_subgraph_root_node`,
  `find_root_by_uuid`, `collect_related_nodes`, `finalize_nodes`,
  `collect_subgraph_edges`, `build_edge_query`,
  `plain_edge_query`. `upsert_entities` split into
  `build_entity_params` and `auto_link_contains` (placed at module
  scope so they aren't picked up as trait methods). `kind_to_label`
  simplified to `format!("{kind:?}")` — every variant's Neo4j label
  equals its own name, so the 110-arm match collapses to a `Debug`-derived
  string.
- **Refactor(models)**: `Display for EntityKind` (110 arms) — replaced
  with a module-level `KIND_LABELS: &[(EntityKind, &str)]` table plus a
  6-line `fmt` that looks up the table and falls back to `Debug` for any
  future variant. The unknown-arm fallback also guarantees a future
  variant can never render empty.
- **Refactor(parser): per-language extractors split into helpers.**
  `toml.rs`: `extract_cargo_package` / `extract_cargo_features` /
  `extract_workspace_members`. `groovy/mod.rs`: `GroovyScan` with
  `try_type_declaration` / `try_methods` / `try_property` / `push_property`
  and a top-level `collect_instance_names` helper. `groovy/properties.rs`:
  `try_extract_property` split into `strip_leading_annotations`,
  `try_extract_initialized_property`, `try_extract_bare_property`,
  `find_bare_property_type`. `varnish/vcl.rs`: `parse_sub_body` +
  `parse_call_statement` + `parse_dotted_path_statement` +
  `parse_ident_statement` + `record_method_call`. `varnish/vcc.rs`:
  `VccState` + `handle_module` / `handle_function` / `handle_object` /
  `handle_method` / `handle_other` + `take_prose` + `process_line`.
  `varnish/vtc.rs`: `VtcContext` + `handle_test_case` / `handle_server`
  / `handle_client` / `handle_varnish` / `handle_logexpect` /
  `handle_barrier` + `dispatch` + `collect_instance_names`.
  `rust/types.rs`: `collect_type_nodes` split into `capture_type_*` /
  `handle_*` / `walk_ancestors_until` / `recurse_children` /
  `handle_token_tree` (with the O(N) skip-check preserved).
  `typescript.rs`: `process_export_clause_specifiers` +
  `extract_export_statement_intents`. `cpp.rs`: `extract_call_expr_intent`
  + `extract_type_identifier_intent` + dispatch. `helm.rs`:
  `values_ref_parts` + `scoped_var_ref_parts`. `html.rs`:
  `extract_script_imports` + `extract_link_stylesheet_imports` +
  `find_tag_name` + `collect_link_attributes` (which removed the inner
  `excessive_nesting` expects). `parser/context.rs`:
  `compute_fqn_and_context` split into `dot_qualified`,
  `path_qualified`, plus an `FqnShape` enum classifier (`DotQualified` /
  `PathQualified` / `Prefixed(&'static str)` / `MacroInvoke` / `BareName`)
  that keeps the match exhaustive over `EntityKind`. `enrich.rs`: the
  docstring/decorators/class-inheritance/CssPath/CSharp FQN/type-references
  blocks split into focused helpers around a `FqnEnrichment` struct
  (`apply_java_package` / `apply_cpp_namespace` / `apply_csharp_scope`)
  + `set_markdown_embed_text` + `apply_js_require_alias` +
  `record_covered_range` + `extract_entity_decorators` +
  `retain_non_self_references`. `mcp_tools/.../format.rs`:
  `format_entity` split into `format_string_list` (shared by
  `subclasses` / `implementers` / `dependencies`) and `append_samples`
  (used by both the type-usage and the callers summary).
- **Test(parser)**: Three oversized tests split. `groovy/tests.rs`
  `test_groovy_parse_sample_full_file` (124 lines) split into
  `test_groovy_parse_sample_full_file_types`,
  `test_groovy_parse_sample_full_file_methods_and_properties`,
  `test_groovy_parse_sample_full_file_docstrings`.
  `extractor/tests.rs` `test_extract_entities_markdown_section_body_in_embed_text`
  (119 lines) split into `test_extract_entities_markdown_section_fqn_chain`
  plus the slimmed body test. `models/entity.rs` gains a `test_display_labels`
  test (20 spot-checks across language families — the new `KIND_LABELS`
  table had no test coverage).
- **Chore(ingest)**: Plain-expect-removal touches — `bin/knot.rs`
  `main()`, `pipeline/watch.rs` `setup_watch_mode_with_progress` — plus
  the paired-attribution files in the language extractors
  (`groovy/{accessors,methods}.rs`, `parser/javascript/jsx.rs`,
  `parser/json_config.rs`, `parser/rust/tests.rs`,
  `extractor/post_passes.rs`). All unfulfilled at the new 100-line
  threshold; now unenforced.
- **Clippy config**: `clippy.toml` raised `too-many-lines-threshold`
  from 80 → 100 (upstream default) and documented the measured curves
  under both `too_many_lines` and `cognitive_complexity`. The
  `cognitive_complexity` annotation now records the measured
  `tracing/log` macro inflation observed across `axum → tower/log →
  tracing/log`.

- **cargo fmt** clean | **cargo clippy --all-targets --all-features -D warnings** clean
- ✅ Unit tests passing (1250 lib + 3 + 4 binary tests) | **E2E suites** status:
  21/21 (typescript, java, javascript, web, kotlin, rust, rust_reference_resolution,
  python, rust_test_module, build_systems, config, k8s_helm, groovy,
  cross_repo_dep, repo_scope, cross_lang_ref, cpp, markdown,
  contains_autolink_index, varnish, csharp) — see CI

---

## v1.8.2 — Refactor: Code Deduplication and Typo Cleanup

Internal code-health release: deduplication, modularization of the
groovy and javascript parsers, type consolidations in the db/models
layer, and a stack of typo / UK→US spelling fixes. A new
disambiguation fallback ladder lands in the ingest reference resolver.
No schema changes, no re-index required for already-indexed
repositories.

- **Refactor(parser)**: Modularize the groovy parser into a `groovy/`
  directory module (submodules: `mod`, `capture`, `inheritance`,
  `methods`, `properties`, `accessors`, `refs`, `utils`, `tests`).
  Mirrors the csharp / rust / varnish directory-module style. Pure
  file move; no behaviour change.
- **Refactor(parser)**: Modularize the javascript parser into a
  `javascript/` directory module (submodules: `mod`, `refs`, `imports`,
  `jsx`, `inheritance`, `dom_css`, `tests`). `pub(crate)` re-exports
  in `mod.rs` preserve the public API for `captures`, `enrich`,
  `post_passes`, `orphans`, `test_utils` and `typescript`.
- **Refactor(db, models)**: `models::RootCandidateLite` collapses to a
  type alias of `db::graph::RootCandidate` so the wire payload and the
  db projection cannot drift apart. `ResolvedSubgraphRoot` extracted
  from a tuple return. Cypher → cipher rename in the reference-target
  resolution ladder.
- **Refactor(cli)**: `filter_repos_by_name` extracted as a pure helper;
  CLI table formatter, `explore_file`, `find_callers` and
  `search_hybrid_context` cleaned of redundant branches.
- **Refactor(mcp, parser)**: Inline `mcp_tools/search_hybrid_context/enrich.rs`
  into the surrounding `mod.rs`; extract `extract_call_reference_intents`
  in `extractor/captures.rs`.
- **Refactor(parser)**: Tighten varnish (lexer/vcc/vcl/vtc),
  csharp, rust, java, markdown, typescript and msbuild parsers —
  dead-code removal, helper extraction, `#[cfg(test)]` gating on
  test-only re-exports so `cargo clippy` (lib profile) stays
  warning-free.
- **Refactor(docs, utils, bin)**: Normalize British → American English
  across doc-comments (`maximise` → `maximize`, `behaviour` → `behavior`,
  `initialise` → `initialize`, etc.); tighten utility modules and
  CLI entry points.
- **Feat(ingest)**: New `resolve_homonym_fallback` ladder that
  disambiguates homonym callees (same method name, different targets)
  when neither FQN nor receiver matches. 4-step escalation:
  arg-count + same-file, arg-count, same-file, receiver-chain,
  single-unambiguous. The hot FQN / receiver lookups remain unchanged
  and the ladder only kicks in when they fail.
- **Test(parser)**: Add `collect_value_references` helper to
  `test_utils` (matching the existing `collect_extends` /
  `collect_implements` convention); dedup 4 inline filter-map blocks in
  the python ValueReference tests.
- **Chore(docs)**: `AGENTS.md` groovy entry refreshed to reflect the
  new directory layout.
- **cargo fmt** clean | **cargo clippy --all-targets --all-features -D warnings** clean
- ✅ Unit tests passing (lib + binaries) | **E2E suites** status: see CI

---

## v1.8.1 — Reference Repo Attribution (`find_callers` rows carry their repository)

Purely additive projection + rendering change: every row returned by
`find_references` (MCP `find_callers` / CLI `knot callers`) now identifies the
repository it belongs to. No schema, no API signature, no re-index.

- **Feat(query)**: `relationship_query`, `overridden_by_query`, `overrides_query`
  and the reference-target resolution ladder now project `repo_name`
  (referencing entity) and `target_repo_name` (referenced entity);
  `overrides_query` swaps the aliases to match its mirrored projection. The
  deprecated `QueryExt::find_callers` path projects `caller.repo_name` too.
  New pure `reference_target_query` helper extracted from
  `resolve_reference_targets` for testability.
- **Feat(models)**: `TargetRow.repo_name` — `resolution.targets[]` is now
  self-labeling. `parse_reference_row` reads `repo_name` / `target_repo_name`
  (`.ok()` → `null` when the node predates repo attribution), keeping the JSON
  contract additive.
- **Feat(ui)**: caller entries, target group headers, resolution bullets and
  the CLI table render `(repo: <name>)` via the shared `format_file_line`
  helper (no more inline path formatting in `format_reference_entry`); the
  table labels the Target column only for genuine cross-repo references.
  Unlike search in v1.8.0, the annotation is emitted whenever the field is a
  non-empty string — single-repo callers output gains the label too
  (intentional, consistent with search).
- **Test(e2e)**: `run_repo_scope_e2e.sh` Group C extended with 6 attribution
  assertions (MCP Markdown, CLI parity, JSON raw fields, resolution targets,
  single-repo regression guard) — observed red before implementation
  (6 red / 27 green).
- **Docs**: README, `.prompt`, `.knot-agent.md`, callers skill
  (`.knot-agent-skills/callers.md` + `knot-callers/` variant) and the
  `find_callers` MCP tool description.
- **cargo fmt** clean | **cargo clippy --all-targets --all-features -D warnings** clean
- ✅ 1256 unit tests passing | **21/21 E2E suites** passing | repo-scope suite 33/33

---

## v1.8.0 — Repository Scope Selection (`all` / Multi-Repo Filtering)

Closes [#19 — Add new search across all repos option](https://github.com/raultov/knot/issues/19).
`repo_name` (MCP) and `--repo` (CLI) now accept a single repository
(unchanged), a comma-separated union list, or the sentinel `all`
(case-insensitive) / `*` — identical semantics on both surfaces.

- **Feat(models)**: New `RepoScope` enum (`All` / `One` / `Many`) with
  `parse` / `parse_optional` / `from_json` / `filter_names` /
  `is_unfiltered`. Normative rules: trim, split on `,`, drop empty
  tokens, dedupe preserving first-occurrence order; the sentinel wins
  over any other token in the same list; repo names stay case-sensitive;
  unknown names are silent no-rows. MCP additionally accepts a JSON
  string array (`"repo_name": ["a", "b"]`).
- **Feat(db)**: Qdrant multi-value filter — new pure `build_repo_filter`
  helper emits `MatchValue::Keyword` for one repo and
  `MatchValue::Keywords` for N, staying on the existing `repo_name`
  Keyword payload index; an empty scope means no filter.
- **Refactor(db)**: Neo4j predicates unified on `x.repo_name IN
  $repo_names` with `Vec<String>` list parameters; ~10 duplicated
  per-scope query branches in `query.rs` collapsed; `QueryExt` (7
  methods) and `VectorSearchExt::search` now take `repo_names: &[String]`
  (empty = unfiltered). `IN` with one element is plan-equivalent to `=`,
  so single-repo queries do not regress.
- **Feat(mcp)**: New `repo_scope_from_args` helper; `search_hybrid_context`,
  `find_callers`, and `explore_file` accept single / comma-list / `all` /
  `*` / JSON array, and their schema descriptions teach the syntax.
  `list_repo_dependencies`, `list_repositories`, and subgraph traversal
  are unchanged (single-repo contract).
- **Feat(cli)**: `knot search/callers/explore --repo` accepts lists and
  the sentinel via the tested `build_repo_scope` helper; an absent flag
  keeps the auto-detected repo default.
- **Feat(ui)**: Every search row is annotated with `(repo: <name>)` in
  the CLI table and MCP Markdown output, so multi-repo results are
  self-labeling.
- **Fix(explore)**: `explore_file` now surfaces `ambiguous_path_candidates`
  whenever ≥ 2 repositories match the same relative path (not only on an
  empty match), renders them in CLI/Markdown output, and the suffix
  fallback query matches repo-relative paths.
- **Test(e2e)**: New BDD suite `tests/run_repo_scope_e2e.sh` (27
  assertions: sentinel, lists, whitespace/unknown/duplicates,
  find_callers, explore_file ambiguity, JSON array, CLI parity, output
  labeling) registered as the 21st suite; observed red before
  implementation (17 red / 10 regression guards green).
- **Docs**: README, `.prompt`, `.knot-agent.md`, MCP server instructions,
  and the roadmap updated with the scope syntax and the global
  `max_results` caveat for multi-repo scopes.
- **cargo fmt** clean | **cargo clippy --all-targets --all-features -D warnings** clean
- ✅ 1232 unit tests passing | **21/21 E2E suites** passing | repo-scope suite 27/27

---

## v1.7.2 — Deterministic Subgraph Root Resolution & MSBuild/NuGet Build-System Support

Part A: deterministic subgraph root resolution (the `get_entity_subgraph`
root pick is no longer arbitrary on a bare-name query). Part B: MSBuild
(`.csproj` + `Directory.Packages.props`) is now a first-class build system
— C# repos report `build_system: "nuget"` and produce cross-repo
`DEPENDS_ON` edges. Plan:
`docs/specs/subgraph_root_resolution_and_msbuild_support_plan.md`.

### Part A — Subgraph

- **Feat(query)**: New pure `root_kind_rank(kind)` precedence in
  `src/db/graph/query.rs` — types (`csharp_class`, `rust_struct`,
  `python_class`, etc.) outrank callables (the constructor homonym in C#,
  for instance); callables outrank members; namespaces / modules /
  markdown rank below callables; everything else lands at rank 4. None
  → rank 4. Used by `rank_root_candidates` to order candidates within
  a tier by `(rank, file_path, start_line, uuid)` — the trailing tuple
  is a total order because neither `name` nor `fqn` is unique (`<module>`
  entities and partial classes share both).
- **Feat(query)**: New `resolve_subgraph_root(name, repo_name)` method on
  `GraphDb` walks the existing `target_resolution_tiers` ladder
  (`query.rs:46-77`) with early stop, then applies `rank_root_candidates`
  to the winning tier. First non-empty tier wins; result is a
  `(winner, tier, total_candidates)` triple.
- **Feat(query)**: `get_entity_subgraph` now anchors the traversal on the
  resolved UUID, not on the bare name — eliminates the prior
  homonym-union defect (where `?entity=McpServer` returned the union of
  the class and the constructor neighbourhoods under a single
  `root_id`). The `root_match_clause` Cypher splicer has been deleted.
- **Feat(query)**: `get_entity_subgraph` now sorts the collected nodes
  by `uuid` before truncating under `max_nodes`, so the retained subset
  is identical across repeated calls.
- **Feat(models)**: New optional `root_resolution: Option<RootResolution>`
  field on `SubgraphResult` (`src/models/subgraph.rs`). When the
  subgraph was queried by name, it carries `{ query, tier,
  total_candidates, chosen, candidates[≤10] }` so consumers can
  understand how the root was picked. `serde(default)` keeps older
  payloads deserialisable; knot-server is not required to surface it.
- **Behavior Change**: Callers that previously passed a bare name and
  received the union of all homonyms' neighbourhoods now receive the
  chosen root's neighbourhood only — result sets get **smaller**, not
  just differently rooted. Fully-qualified `?entity=<fqn>` queries now
  resolve via the `ExactFqn` tier instead of returning an empty
  subgraph.

### Part B — MSBuild/NuGet

- **Feat(input)**: `csproj` is now in both `CORE_EXTENSIONS` and
  `SUPPORTED_EXTENSIONS` (extension-based discovery, indexed regardless
  of `--include-config-files`); `Directory.Packages.props` is in
  `BUILD_SYSTEM_NAMES` (exact-filename match — the `props` extension is
  in no table, so only the exact name is admitted). No new crates; no
  CURRENT_STATE_VERSION bump.
- **Feat(parser)**: New `src/pipeline/parser/languages/msbuild.rs`
  module. Mirrors the Maven `xml.rs` parser shape (roxmltree, local-name
  helper, BOM-tolerant string source). Emits one `ProjectIdentity` per
  `.csproj` and one `BuildDependency` per `<PackageReference>`.
  Identity fallback chain: `<PackageId>` → `<AssemblyName>` → file
  stem. `<ProjectReference>` is intentionally skipped (see
  `§10.4 / §15` of the plan). UTF-8 BOMs are stripped defensively
  before parsing — pinned by a unit test against the literal BOM bytes
  rather than relying on `roxmltree` tolerance (the spec notes the
  library's BOM behaviour is reported inconsistently across versions).
- **Feat(parser)**: Central Package Management (CPM) resolution.
  Version-less `<PackageReference>` elements look up their version in
  the nearest ancestor `Directory.Packages.props`, walking up from the
  csproj's directory to the repo root. The map is built and cached
  once per props file in a process-local `OnceLock<Mutex<HashMap>>` so
  N csproj projects sharing the same props file produce 1 read. Cache
  lifetime = process (build files do not change mid-run).
- **Feat(ingest)**: `parse_build_system_from_fqn` and
  `parse_artifact_identity` recognise the `nuget:` prefix (NuGet IDs
  are flat, like Cargo). `match_dependency_to_repository` gains a
  NuGet arm **before** the Maven-style branch (the `nuget:` prefix has
  no dot — without the early arm, `parse_maven_style_dep` would
  mis-read `nuget:Acme.Auth.Lib:1.0.0` as group="Acme.Auth.Lib",
  artifact="1.0.0"); the Cargo fallback also gains a
  `crate_name != "nuget"` guard.
- **Feat(ingest)**: `link_cross_repo_dependencies` primary selection
  now prefers NuGet identities carrying the `identity: package_id`
  marker over depth-tied unmarked candidates — partition by marker
  first; fall back to the unmodified shallowest-`min_by_key` when no
  marker is present. The marker is emitted only by the MSBuild parser,
  so Maven / Gradle / Cargo / npm selection is bit-for-bit unchanged
  (pinned by cross-repo e2e Test 8). The known consequence: a repo
  with no `<PackageId>` (e.g. `openlogi-net`) falls through to the
  alphabetically-first depth-2 stem — harmless for cross-repo because
  nothing consumes it as a package; solution-level identity is
  deferred.
- **Docs(mcp)**: `list_repo_dependencies` tool description now lists
  NuGet among the supported build systems and notes the `--clean`
  re-index recommendation for repos previously reported as
  `build_system: "none"`.
- **Test**: 11 new pure unit tests (4 for kind ranking,
  4 for candidate ordering, 3 for disclosure serialisation), 3 new
  discovery sync-guard tests, 18 new MSBuild parser tests (csproj
  core, CPM resolution, BOM handling), 6 new cross_repo wiring tests
  (NuGet parser, marker detection, ordering-hazard regression marker).
- **Docs**: `docs/specs/csharp_support_plan.md` §14.1 is now marked
  superseded by this plan (its "suffix-based discovery" blocker
  premise was wrong — `Path::extension()` yields `"csproj"`, the
  ordinary extension branch suffices).
---

## v1.7.1 — find_callers Target Resolution & Substring Noise Reduction

- **Feat(query)**: Implement a two-stage target resolution ladder for `find_references` / `find_callers`. Matching is precedence-based: exact FQN → FQN suffix (`Type.member`) → exact name → signature prefix (`accept(List`) → fuzzy substring. An exact match now suppresses all fuzzy substring noise. **Behavior Change**: Queries that previously returned fuzzy noise alongside an exact hit now return only the exact hit.
- **Feat(db)**: Add `entity_name_text` and `entity_fqn_text` `TEXT` indexes to Neo4j to speed up `ENDS WITH` and `CONTAINS` queries.
- **Feat(cli)**: Surface resolution metadata (tier matched, fuzzy warning, truncation notice) in both the CLI table formatter and markdown formatter.
---

## v1.7.0 — C# Language Support

Closes [#5 — help wanted: add C# support](https://github.com/raultov/knot/issues/5).
C#/.NET codebases now get the same indexing fidelity as Java and Kotlin: entity
extraction, namespace-qualified FQNs, `CALLS` / `EXTENDS` / `IMPLEMENTS` /
`REFERENCES` / `CONTAINS` / `OVERRIDES` edges, XML doc comments, and attributes.
Plan: `docs/specs/csharp_support_plan.md`.

- **Feat(parser)**: Full C# extraction (`.cs`) via `tree-sitter-c-sharp 0.23`.
  Sixteen new `CSharp*` entity kinds: `CSharpClass`, `CSharpInterface`,
  `CSharpStruct`, `CSharpRecord` (both `record class` and `record struct`),
  `CSharpEnum`, `CSharpMethod`, `CSharpConstructor`, `CSharpProperty`,
  `CSharpField` (with `const` → `CSharpConstant` promotion),
  `CSharpDelegate`, `CSharpEvent`, `CSharpIndexer` (`this[]`),
  `CSharpOperator` (`operator +`), `CSharpNamespace`, and
  `CSharpLocalFunction`. Grammar gaps are handled in Rust: `field_declaration`
  has no `name` field (the declarator identifier is resolved up the tree),
  and indexer/operator declarations get synthesised names.
- **Feat(parser)**: Namespace-qualified FQNs — `<namespace>.<Type>.<member>`
  across both namespace forms. A file-level pre-pass handles file-scoped
  namespaces (C# 10, no `body` in the grammar, so types are siblings and
  unreachable by a parent walk); an ancestor walk collects block-form
  namespaces and containing types. Methods also persist
  `enclosing_class_fqn`, so Neo4j `CONTAINS` auto-linking matches by exact
  FQN (Rust parity).
- **Feat(parser)**: `base_list` disambiguation heuristic — interfaces always
  emit `EXTENDS`, structs (and record-structs) always `IMPLEMENTS`, and
  classes/record-classes emit the first entry as `EXTENDS` unless it matches
  the `^I[A-Z]` interface convention. Generic arguments are stripped
  (`IRepository<User>` → `IRepository`).
- **Feat(parser)**: Reference extraction with two C#-specific refinements:
  field-typed receivers are substituted with the declared field type
  (`_repository.FindByIdAsync()` resolves to the exact implementation
  method rather than staying ambiguous), and `base.Method()` is emitted with
  the resolver's `super` receiver so it walks the extends map. Attributes
  (`[Obsolete]`) are captured as decorators **and** as call intents; `using`
  directives and top-level statements (C# 9) are handled by the orphan pass
  with a `CSharpNamespace` synthetic `<module>` entity.
- **Feat(overrides)**: `OVERRIDES` linking extended beyond the JVM. C# joins
  via the `.cs` extension guard plus the `CSharp*` kind allowlists
  (`CSharpMethod`/`CSharpProperty` method-like; the five C# type kinds
  type-like). The module's `JVM_*` vocabulary was renamed to
  `OVERRIDE_CAPABLE_*` to match its widened scope.
- **Feat(tools)**: `explore_file` gains C# kind buckets (Classes, Interfaces,
  Structs, Records, Enums, Methods, Properties & Fields, Delegates & Events,
  Operators & Indexers, Namespaces); `search_hybrid_context`, `find_callers`,
  and `explore_file` descriptions now list C#.
- **Test**: 42 C# parser unit tests (entity kinds, grammar gaps, FQN shapes
  across namespace forms, inheritance heuristic, receiver substitution,
  OVERRIDES end-to-end through resolution, top-level statement orphaning) and
  a new `tests/run_csharp_e2e.sh` suite (registered as the 20th E2E suite)
  covering entity extraction, FQNs, relationships, OVERRIDES, doc comments,
  attributes, semantic search, and find_callers — validated through both the
  MCP server and the CLI.
- **Docs**: README language list + C# section, AGENTS.md parser table and
  entity kinds, Phase 15 marked complete in `docs/specs/multilanguage_roadmap.md`.
---

## v1.6.2 — Accurate Indexing Progress

Indexing progress now reflects the **whole pipeline**, not just file reading.
The percentage used to jump to 100% within ~6 seconds on a 3,713-file repository
and then freeze for several minutes while embedding and ingestion were still
running. v1.6.2 fixes this with a banded formula and a new entity-total signal
from the parser.

- **Fix(progress)**: The percentage now spans the entire run via weighted bands:
  `0–10%` for parsing (`parsed_files / total_files`), `10–90%` for embedding +
  ingestion (`entities_ingested / total_entities`), `95%` during reference
  resolution (no sub-counters available), and `100%` only on `Completed`.
  The bar is monotonically non-decreasing across a full run, and a `Failed`
  state freezes the bar at the last computed value rather than snapping to 0%
  or 100%.
- **Feat(progress)**: `IndexingProgress` exposes `total_entities: u64` (and the
  equivalent JSON field) so downstream consumers can render the entity-level
  counter alongside the file-level one.
- **Feat(parser)**: New `ParseCallbacks` struct replaces the v1.6.1
  `FileParsedCallback` parameter on `parse_files_stream`. It carries the
  existing per-file hook plus a new `on_entities_extracted` hook that fires
  exactly once, after post-parse aggregation and **before** any entity is
  pushed into the bounded channel. This is the exact handoff point that lets
  the percentage transition from the parse band to the ingest band without
  ever saturating at 100% while the channel is still full. Passing `None` is
  unchanged from previous versions, so every existing `None` call site compiles
  untouched.
- **Test(progress)**: New unit tests cover the banded formula edge cases
  (`zero entities`, over-counting, `ResolvingReferences`, `Failed`),
  the monotonicity property over the full pipeline sequence, and the parser's
  publish-before-blocking invariant (`given_a_saturated_channel_when_parsing_completes_then_total_is_published_before_blocking`).
- **Docs**: Updated the `Indexing Progress` section of `README.md` to describe
  the band table and the new `total_entities` field. The `[Progress]` log
  format now also prints `entities <ingested>/<total>` so the curve is
  informative during the long ingestion phase.

> **Semver caveat (deliberate, not accidental):** v1.6.2 is a patch release
> that contains two technically-breaking changes for downstream crates:
>
> 1. Adding `pub total_entities: u64` to `IndexingProgress` breaks any
>    downstream code that constructs `IndexingProgress` with a struct
>    literal. `knot-server` (the only known consumer) is updated in lockstep
>    to `0.3.2`.
> 2. The 5th parameter of `parse_files_stream` changes type from
>    `Option<FileParsedCallback>` to `Option<ParseCallbacks>`. Callers passing
>    `None` are unaffected; callers passing `Some(cb)` must wrap the callback
>    in `ParseCallbacks { on_file_parsed: Some(cb), on_entities_extracted: None }`.
>
> A strict reading of semver would call for `1.7.0`. We are shipping this as
> `1.6.2` because the only known consumer is updated in lockstep. If a third
> party pins `knot = "1.6"` they will get a compile error on upgrade.

See [`docs/specs/indexing_progress_accuracy_plan.md`](docs/specs/indexing_progress_accuracy_plan.md)
for the full design rationale, including the rejected alternatives.
---

## v1.6.1 — Varnish VCL Include Resolution

- **Fix(varnish)**: Resolved an issue where Varnish `include` directives with absolute paths failed to map to their target files. The parser now preserves raw path strings and the resolver uses a multi-strategy approach (repo-root fallback, relative path fallback, and filename fuzzy match) to reliably build the `INCLUDES` relationship.
- **Test(varnish)**: Expanded Varnish E2E integration tests with fixtures for absolute path resolution (`/etc/varnish/language.vcl`).
- **Docs**: Removed completed items ("Varnish VCL support" and CLI commands) from the README roadmap. Added the `varnish_include_resolution_plan.md` spec.
---

## v1.6.0 — Unsafe Elimination & Code Quality Enforcement

- **Fix(css)**: Replaced three `mem::zeroed::<Node>()` unsafe blocks with real tree-sitter-css parse nodes. The zeroed nodes dereferenced null pointers in `start_position()` — genuine undefined behaviour, not a lint technicality. Tests now additionally assert `start_line`, closing a real coverage gap.
- **Refactor(unsafe)**: Eliminated 17 of 18 `unsafe` blocks across the codebase. Environment mutation in tests (`std::env::set_var`/`remove_var`) replaced with `temp_env::with_var()` (panic-safe), `dotenvy::from_path_iter()` (non-mutating), and dependency injection for `knot_env_path()` (HashMap-driven tests). A single audited exception survives in `src/utils/mod.rs` for `SSL_CERT_FILE` injection behind corporate proxies, annotated with `#[expect(unsafe_code, reason = "...")]`.
- **Refactor(lint)**: Converted all bare `#[allow(...)]` attributes to `#[expect(lint, reason = "...")]` with documented justifications across 55 source files. Added documented expects for `too_many_lines` (threshold 80), `cognitive_complexity` (threshold 15), `too_many_arguments` (threshold 5), `excessive_nesting` (threshold 6), and `type_complexity` (threshold 200).
- **Refactor(cli)**: Extracted `SubgraphQueryParams` and `SearchContext` structs to reduce argument counts below the 5-arg threshold. Rewrote `format_file_entities` with a data-driven `KIND_BUCKETS` table, shrinking from 467 to ~30 lines.
- **Refactor(lexer)**: Removed unused `source` field and `current()` helper from the Varnish lexer, eliminating the `Lexer<'a>` lifetime parameter. Test-only `tokenize_hash_comments` gated behind `#[cfg(test)]`.
- **Chore(config)**: Added `clippy.toml` with readability thresholds measured against current production code. Enabled `too_many_lines`, `cognitive_complexity`, `allow_attributes`, and `allow_attributes_without_reason` as warnings in `[lints.clippy]`.
- **Chore(deps)**: Added `temp-env = "0.3"` as a dev-dependency for panic-safe environment mutation in test helpers.
- **Chore(lints)**: Enforced `unsafe_code = "deny"` at crate level. Any newly introduced `unsafe` block (without a documented `#[expect]`) is a compilation error.
- **Docs**: Removed obsolete implementation specs (`unsafe_removal.md`, `varnish_support.md`) — both are now fully implemented and covered by the codebase and CHANGELOG.
- **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean | **19/19 E2E suites** passing | **1037 unit tests** passing
---

## v1.5.7 — Varnish Cache Language Support

- **Feat(parser)**: Full **Varnish Cache** support via hand-written parsers for `.vcl` (configuration), `.vtc` (test cases), and `.vcc` (VMOD C source). No tree-sitter grammars required — all three formats are decoded by a single hand-rolled lexer that handles VCL's 15 documented gotchas (duration maximal-munch, adjacent string concatenation, ACL mask literals, identifier hyphens, `${...}` macro tokens, version markers, dotted paths, quoted header names, comment forms, etc.).
- **Feat(vcl)**: VCL extraction emits `vcl_version`, `vcl_subroutine` (custom), `vcl_builtin_sub` (with `vcl_*` name + multi-part aggregator), `vcl_backend`, `vcl_probe` (named and inline), `vcl_acl`, `vcl_import` (with `as` alias + `from` path), `vcl_object_instance` (`new x = directors.round_robin()`), plus declarations for `include`, `unused`, VMOD method calls, and `req.backend_hint = X;` assignments (resolved to `USES_BACKEND` edges). Bodies of `if/elseif/else` blocks are scanned recursively so `set req.backend_hint = …` inside conditionals still emits the edge. The Fastly VCL dialect is detected and skipped (returns empty entities, logs `debug`).
- **Feat(vtc)**: VTC extraction emits `vtc_test_case` (from `varnishtest`/`vtest`), `vtc_server`, `vtc_client`, `vtc_varnish_instance`, `vtc_logexpect`, `vtc_barrier`. Embedded VCL inside `varnish vX { … }` blocks is delegated to the VCL parser with line offsets so cross-references resolve. `-errvcl` blocks are skipped. `-vcl+backend` synthesises `vcl_backend` entities per `server` declaration with `is_test_context = true` and `ValueReference` to `vtc:server:<name>`.
- **Feat(vcc)**: VMOD C source extraction emits `vcc_module`, `vcc_function`, `vcc_object`, `vcc_method`, plus `$Event`, `$Restrict`, ENUMs, and default parameters. Methods are bound to their owning object via `enclosing_class`.
- **Feat(relationships)**: 7 new `ReferenceIntent` variants (`VclSubCall`, `VclBackendRef`, `VclProbeRef`, `VclAclRef`, `VclInclude`, `VclVmodImport`, `VclUnusedRef`) and 6 new `RelationshipType` variants with directed `Display` forms (`UsesBackend`, `UsesProbe`, `UsesAcl`, `Includes`, `ImportsVmod`, `DeclaredUnused`). Three-way match enforced across `entity.rs` (Display), `db/graph/utils.rs` (kind_to_label), `pipeline/parser/context.rs` (compute_fqn_and_context).
- **Feat(parser-orchestrator)**: Varnish built-in sub aggregators (`vcl_recv_aggregator`, etc.) are now emitted globally in `parse_files_stream` via a new `aggregate_varnish_builtin_subs` post-parse step in `languages/varnish/mod.rs`, ensuring one aggregator per sub name across the repo (with `file_path` = lex first match per `discover_files` sort order). Wired into `src/pipeline/parser/mod.rs` via a shared `Arc<Mutex<Vec<ParsedEntity>>>` buffer.
- **Feat(cli)**: `explore_file` now displays Varnish entities via a fallback `## Other Entities` bucket so all 18 Varnish kinds remain visible to LLMs (previously fell through `_ => {}` and were silently dropped).
- **Test(unit)**: 68 unit tests in `pipeline::parser::languages::varnish` covering lexer gotchas, dialect guard, VCL sub/backend/probe/acl/import/include/unused/aggregate, VTC server/client/varnish/logexpect/barrier/errvcl/vcl+backend synthesis, VCC module/function/object/method/default params.
- **Test(e2e)**: New `tests/run_varnish_e2e.sh` with **25 assertions** covering entity counts, all 6 relationship types, VCL/VTC/VCC extraction, multi-part sub aggregation, Fastly suppression, unique-token semantic search, and `explore_file` listing. Registered as the 19th suite in `tests/run_all_e2e_fast.sh`.
- **Docs**: New spec `docs/specs/varnish_support.md` (1072 lines covering scope, data model, hard problems, lexing gotchas, phases, gotcha catalog). README updated with Varnish language section + E2E command.
- **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean | **1037 unit tests** passing | **19/19 E2E suites** passing
---

## v1.5.6 — Groovy Property Accessors & Parser Hardening

- **Fix(groovy)**: Javadoc block-comment continuation lines no longer produce phantom method entities or corrupt scope tracking. New `strip_comments_line` helper tracks multi-line `/* */` state across lines, and brace counting operates on the code-bearing remainder only.
- **Feat(groovy)**: Bare property declarations (`Path baseDir`, `boolean cacheable`, `private final Path ROOT`) are now indexed as `GroovyProperty` entities. Previously only initialized properties (`String name = 'test'`) were detected.
- **Feat(groovy)**: Compiler-generated property accessors (`getX`/`setX`/`isX`) are synthesised as first-class `GroovyMethod` entities, enabling `OVERRIDES` linking between Groovy properties and interface getter declarations. Explicit getters/setters suppress synthetic ones, and `final` properties emit getters only.
- **Fix(groovy-scm)**: Fixed `queries/groovy.scm` to compile against tree-sitter-groovy v0.1.2 by replacing `variable_declaration` with `local_variable_declaration`. Added `function_definition` capture patterns for `def`-style methods.
- **Test(unit)**: 30+ new unit tests covering comment stripping, bare property detection, synthetic accessor generation, override linking for property accessors vs interface getters, and the tree-sitter query compilation assertion.
- **Test(e2e)**: Added Group G in `tests/run_groovy_e2e.sh` — validates `find_callers` Overridden-by/Overrides for properties, `explore_file` lists properties, Javadoc phantom regression guard, and Neo4j dedup/no-override invariants.
- **Docs**: Updated README Groovy section with property accessor synthesis details.
- **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean | **969 unit tests** passing
---

## v1.5.5 — JVM Method Override Relationships

- ✅ **Feat(resolve)**: Implemented JVM method-level `OVERRIDES` relationships. The graph now links a method in a subtype directly to the method it overrides/implements in a supertype, enabling reverse-dependency queries to surface implementations and declarations bidirectionally.
- ✅ **Feat(query)**: `find_callers` now returns two new directed buckets for JVM entities: **Overridden by** (implementations/descendants of the queried method) and **Overrides** (declarations/ancestors the queried method overrides).
- ✅ **Refactor(query)**: Extracted duplicate Neo4j row parsing logic in `query.rs` into a shared `parse_reference_row` function.
- ✅ **Test(e2e)**: Added Group F in the Groovy E2E suite to verify method overrides bidirectionally. Added cleanup for autolink test artifacts.
- ✅ **Docs**: Marked the `method_override_relationships.md` spec as implemented and updated the README with the new override discovery use cases.
- ✅ **cargo fmt** clean | **cargo clippy** clean | Unit tests passing
---

## v1.5.4 — Groovy Docstring Extraction

- ✅ **Fix(groovy)**: The Groovy lexical parser now extracts GroovyDoc/comment blocks as entity `docstring` for classes, interfaces, enums, traits, methods (`def`, typed single-line and multi-line signatures) and properties. Previously all 5 `ParsedEntity::new` call sites passed `None`, so semantic search could not match Groovy entities by the concepts described in their GroovyDoc (e.g. nextflow's `PluginExtensionPoint.init` was invisible to a "channel factory initialization" query despite being indexed).
- ✅ **Feat(parser)**: New `extract_preceding_docstring` in `src/pipeline/parser/languages/groovy.rs` walks backwards from each declaration: skips annotations (`@PackageScope`, `@Override`) and tolerates one blank line; captures the adjacent `/** ... */` / `/* ... */` block or a burst of `//` lines; stops at `package`/`import`/code lines so license headers never leak into the first class of a file. Markers are stripped via the shared `strip_comment_markers`.
- ✅ **Test(unit)**: 18 new tests — 11 for the backwards-walk policy (adjacent block, annotations, `//` bursts, blank-line tolerance, license-header guard, empty/malformed comments, file start) and 7 for the wiring into `extract_entities_groovy`, including the literal nextflow `PluginExtensionPoint` fragment; `test_groovy_parse_sample_full_file` now asserts extracted docstrings; `prepare.rs` gains a contract test that the docstring reaches `embed_text`.
- ✅ **Test(e2e)**: `tests/run_groovy_e2e.sh` gains **Suite E — Docstrings**: the synthetic `PluginExtensionPoint.groovy` fixture now carries the verbatim nextflow GroovyDoc and `@PackageScope`, with Cypher assertions on the `init`/`checkInit` docstrings, a non-empty-docstring entity count, and a Qdrant scroll parity check (3 points for the file) via the REST port.
- ✅ **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean | **912 unit tests** passing | `./tests/run_groovy_e2e.sh` green
---

## v1.5.3 — Groovy Inheritance & Perf Optimization

- ✅ **Feat(groovy)**: The Groovy parser now emits `EXTENDS`/`IMPLEMENTS` reference intents from `class`, `interface`, `trait`, and `enum` declarations, enabling accurate Nextflow-style hierarchy traversal via `find_callers`.
- ✅ **Chore(config)**: Increased default batch size for Rayon parallel ingestion from 64 to 128.
- ✅ **Fix(e2e)**: Stabilized Neo4j healthchecks and Cypher `EXPLAIN` plan checks to prevent flapping timeouts in integration tests.
- ✅ **cargo fmt** clean | **cargo clippy** clean | **894 unit tests** passing
---

## v1.5.2 — Composite Index for CONTAINS Auto-Link

### Performance

- **Composite index `(repo_name, fqn)` on `:Entity`** — the CONTAINS
  auto-link query no longer degrades to O(n²) per-row label scans on
  every entity in the repository.  Large repos (~50K entities) no
  longer timeout at the end of indexing.  The index is created with
  `IF NOT EXISTS` so it migrates automatically into existing
  deployments.

### Internal

- Extracted `index_statements()` from `ensure_indexes()` for unit
  testing (same pattern already used by `build_contains_auto_link_cypher()`).
- Added unit tests covering the new composite index, idempotency of
  `IF NOT EXISTS`, and preservation of all existing index statements.
- Added integration test (`#[ignore]`) that verifies the index appears
  in `SHOW INDEXES` and that `EXPLAIN` of the auto-link Cypher succeeds.
- Added e2e regression script (`run_contains_autolink_index_e2e.sh`)
  with a synthetic Java fixture of 5,200 entities, verifying index
  presence, plan index-seek, correct CONTAINS edge counts, and a time
  budget canary against O(n²) regression.
---

## v1.5.1 — Machine-Independent (Repo-Relative) File Paths

- ✅ **Feat(pipeline)**: All persisted `file_path` values are now stored as **repo-relative** paths with POSIX separators (e.g. `src/pipeline/embed.rs`). New `to_repo_relative` choke point in `src/pipeline/files.rs` (with `ParseConfig.repo_root` canonicalized once at pipeline start) enforces the format: relative to repo root, POSIX separators, no leading `./`, no trailing `/`, R5 warn-and-passthrough for the degenerate out-of-root case. I/O continues to use absolute paths — only the persisted string changes.
- ✅ **Feat(pipeline)**: Index state version bumps from 3 → 4. Existing v3 state files are rejected by `IndexState::load`; the existing stale-version mechanism triggers a one-time full re-index on upgrade with no manual steps. `file_hashes` keys are now the canonical relative path.
- ✅ **Feat(pipeline)**: Entity UUIDs (`Uuid::new_v5` over `repo_name:file_path:fqn:start_line`) become **machine-independent**: the same repo indexed on two hosts now produces identical UUIDs. Reinforced by the new `test_uuid_stable_across_machines` unit test in `src/models/entity.rs`.
- ✅ **Feat(cli)**: `explore_file` (shared by CLI and MCP) now accepts repo-relative paths (preferred), absolute paths under `KNOT_REPO_PATH` / CWD (auto-stripped), and falls back to a path-boundary `ENDS WITH` suffix query. Ambiguous matches across multiple repos surface a `ambiguous_path_candidates` list instead of a silent miss.
- ✅ **Feat(cli)**: New shared `format_file_line` renderer annotates every file mention with `(repo: <name>)` when the owning repo is known, used by both the CLI and MCP answers.
- ✅ **Feat(graph)**: New `QueryExt::find_files_by_suffix` powers the disambiguation fallback — a single Cypher query returning distinct `(file_path, repo_name)` pairs bounded by the indexed `repo_name` when provided.
- ✅ **Feat(mcp)**: `explore_file` tool description updated to state the preferred relative-path input, the absolute-path fallback, and the disambiguation contract.
- ✅ **Feat(parser)**: Rust crate discovery (`CrateDiscovery::crate_for_file`, `compute_rust_file_kind`) still keys on the absolute path — the relative entity path is reconstructed against `repo_root` only when the parser is invoked with a relative path. FQNs are unaffected (asserted by `tests/run_rust_reference_resolution_e2e.sh`).
- ✅ **Test(unit)**: 6 new tests on `to_repo_relative` (nested file, root file, trailing-slash root, backslash normalization, out-of-root R5, leading-dot-slash guard); 3 tests on `IndexState` for relative keys + v3 rejection; 2 tests on the parser for relative `file_path`; 5 tests on input normalization and the suffix query; 1 test on `format_file_line`.
- ✅ **Test(e2e)**: `tests/run_rust_reference_resolution_e2e.sh` queries updated to expect repo-relative fixture paths and to strip cypher-shell's plain-format quoting.
- ✅ **Fix(e2e)**: `tests/docker-compose.e2e.yml` and `tests/run_all_e2e_fast.sh` hardened — Qdrant 1.16+ removed `/health` so the compose healthcheck never reports healthy; `wait_for_port` now probes the actual port via `nc -z` for non-Neo4j services. Pre-flight also frees any foreign container holding the e2e high ports (e.g. a sibling knot-server setup that would silently steal our bind).
- ✅ **Chore(docs)**: Removed two obsolete spec files (`docs/specs/indexing_progress_api.md`, `docs/specs/performance_fix_bfcarena_and_contains.md`) — their designs are now covered by the codebase and CHANGELOG.
- ✅ **Docs**: README upgrade note, `.prompt`, and `.knot-agent.md` updated to teach the relative-paths contract to humans and LLMs alike.
- ⚠️ **Breaking change**: Upgrading from v1.5.0 triggers an automatic full re-index on first run. `.knot/index_state.json` carries a version field that the loader rejects when stale, and `knot-indexer` wipes the repo from both databases before rebuilding. No manual steps required.
- ✅ **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean
- ✅ Unit tests passing.
---

## v1.5.0 — File-Based Indexing Progress Tracking

- ✅ **Feat(pipeline)**: New `ProgressTracker` API (`src/pipeline/progress.rs`) — thread-safe, pollable struct exposing `snapshot()` as a `Serialize`able `IndexingProgress` so `knot-server` can implement `GET /repos/{name}/progress` without a mapping layer. Counters use lock-free atomics; stage/error live behind an `RwLock`.
- ✅ **Feat(pipeline)**: Indexer logs a `[Progress] [<repo>] X/Y files (Z%) — batch #N ingested (M entities)` line after every ingested batch, and a final `100.0%` line before reference resolution. Format pinned by `tests/run_rust_e2e.sh` grep assertions.
- ✅ **Feat(pipeline)**: `run_indexing_pipeline_with_progress()` and `setup_watch_mode_with_progress()` keep the legacy signatures, creating an internal throwaway tracker so CLI (`knot-indexer`) gets the log lines for free without opting into the API.
- ✅ **Feat(parser)**: New `FileParsedCallback` parameter on `parse_files_stream` invoked exactly once per file (success or parse error), keeping the parser decoupled from the tracker.
- ✅ **Test(progress)**: 10 unit tests on `ProgressTracker` (lifecycle, percent rules, concurrent atomicity, JSON serialization); 3 unit tests on `parse_files_stream` callback (once-per-file invariant, error-path counting, `None` regression).
- ✅ **Test(e2e)**: `tests/run_rust_e2e.sh` now asserts the `[Progress]` log format and the 100.0% final line.
- ✅ **Docs(readme)**: New "Indexing Progress" subsection with log-format example and library API sample.
- ✅ **Docs(specs)**: New specification `docs/specs/indexing_progress_api.md` covering the design, thread-safety, and knot-server integration sketch.
- ✅ **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean
- ✅ 854 unit tests passing.
---

## v1.4.13 — Python `super()` and Chained Attribute Resolution

- ✅ **Fix(python)**: `super().__init__()` calls inside subclasses now resolve to the parent class's `__init__` instead of being misattributed to the enclosing class's own `__init__` (or dropped when the parent was unindexed). Reported against LlamaFactory `webui/chatter.py::WebChatModel`.
- ✅ **Fix(python)**: Chained calls like `engine.chatter.method(...)` now resolve via a receiver-chain disambiguator that scores each candidate by how many receiver segments appear in its FQN, picking the unique winner and dropping ties (no guessing). Fixes the case where a homonymous module-level function in another file would otherwise swallow the call.
- ✅ **Feat(python)**: Python parser now emits `ValueReference` for chained attribute access used as a value (e.g. `engine.chatter.loaded`, `load_btn.click(engine.chatter.load_model, ...)`). Trailing identifier of every `attribute` node is captured, except when it is the function of a `call` (already a `Call` intent) or the `object` of a wider attribute chain (avoids duplicate intermediate segments).
- ✅ **Test(e2e)**: Added 6 E2E assertions in `tests/run_python_e2e.sh` covering `super().__init__()` parent resolution (CLI + MCP), self-misattribution guard, and the three chained-attribute patterns (chained call, chained property, method-as-value).
- ✅ **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean
- ✅ 827 unit tests passing.
---

## v1.4.12 — Python Constructor Call Resolution & Agent Skills Packaging

- ✅ **Feat(python)**: Automatically redirect class instantiation (`ClassName(...)`) to constructor (`ClassName.__init__`) in reference resolution, allowing `find_callers` to accurately list class instantiation sites as callers of `__init__`.
- ✅ **Chore(scripts)**: Packaged agent skills into `.knot-agent-skills.sh` installer and `.tar.gz` archive, replacing the previous python-based generation script.
- ✅ **Docs(specs)**: Added specification for Python constructor call resolution.
- ✅ **Docs**: Updated `knot repos` agent skill documentation to include the `--filter` substring parameter.
- ✅ **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean
---

## v1.4.11 — list_repositories MCP Tool & CLI Filter

- ✅ **Feat(mcp)**: Added `list_repositories` MCP tool to list all indexed repositories with optional name filtering (TDQS-optimized description with sibling tool alternatives).
- ✅ **Feat(cli)**: Added `--filter` flag to `knot repos` for case-insensitive repository name filtering (substring match).
- ✅ **Test(e2e)**: Added 5 E2E tests for `list_repositories` covering CLI list, CLI filter, CLI no-match, MCP list, and MCP filter.
- ✅ **Docs(readme)**: Documented `--filter` flag and MCP tool in README.
- ✅ **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean
---

## v1.4.10 — Pipeline Trace Repo Identification & Docs Cleanup

- ✅ **Feat(pipeline)**: All major pipeline trace lines (embed, batch ingest, vector upsert, graph upsert, embedder/ingester worker logs) now include the originating `repo_name` as a `[repo]` prefix, so `knot-server` users can attribute each log line to the repository being indexed.
- ✅ **Docs(readme)**: New "Install Agent Skills (For AI Agents)" section with "Let an LLM do it" prompt for one-line installation via Claude Code, OpenCode, Cursor, etc.
- ✅ **Docs(readme)**: Replaced manual `tar -xz` instructions for the agent-skills bundle with `curl | bash scripts/install-agent-skills.sh`.
- ✅ **Docs(roadmap)**: Added Phase 13 (Markdown Documentation Indexing) to `docs/specs/multilanguage_roadmap.md` with implementation files, design notes, and intentional limitations.
- ✅ **Chore(docs)**: Removed three obsolete E2E isolation specs (`e2e_per_language_split.md`, `e2e_per_suite_fixture_architecture.md`, `kotlin_python_e2e_isolation_fix.md`) already covered by current implementations.
- ✅ **Chore(scripts)**: Corrected `scripts/install-agent-skills.sh` repository URL (`user/` → `raultov/`) and dropped legacy `alias knot-docs` snippet.
- ✅ **cargo fmt** clean | **cargo clippy --all-targets -- -D warnings** clean
- ✅ 802 unit tests passing.
---

## v1.4.9 — Markdown Documentation Indexing

- ✅ **Feat(parser)**: Added Markdown support (`.md`) with `MarkdownDocument` and `MarkdownSection` entities. Section bodies — paragraphs, fenced code blocks, lists, and tables — are captured into `embed_text` for full semantic search over documentation content, not just heading titles.
- ✅ **Feat(parser)**: Hierarchical, file-scoped FQNs (e.g. `README.md::Setup > Installation > Linux`) prevent cross-file and within-file heading collisions.
- ✅ **Test(e2e)**: Added `run_markdown_e2e.sh` (body searchability, cross-file disambiguation, deep nesting, special-character headings) and wired it into `run_all_e2e_fast.sh`.
- ✅ **Docs(readme)**: Documented Markdown language support.
- ✅ Credit: @sdi2200246 (PR #17, closes #8).
- ✅ **cargo fmt** clean | **cargo clippy** clean
---

## v1.4.8 — Parser Refactor & Version Bump

 - ✅ **Refactor(parser)**: Extracted `set_module_default_export` helper in `post_passes.rs` to unify JS and TS default export logic.
 - ✅ **Refactor(parser)**: Simplified `Node` imports and type usage in `utils.rs`.
 - ✅ **cargo fmt** clean | **cargo clippy** clean
---

## v1.4.7 — Benchmark Fixes & CI Dependencies

 - ✅ **Fix(benchmark)**: Added `/usr/bin/time` check and fixed local Neo4j password environment variable.
 - ✅ **Chore(ci)**: Install `time` package in GitHub Actions to support performance benchmarks.
---

## v1.4.6 — CI Quality Gate Refinement

 - ✅ **Docs(agents)**: Documented `allow-dirty` mechanism for `release.yml`.
---

## v1.4.5 — Release Workflow Modularization

 - ✅ **Chore(workflows)**: Gated GitHub Release on unit tests.
 - ✅ **Docs**: Documented CI/Release split and `dist-init` maintenance warning.
---

## v1.4.4 — Search Precision & E2E Suites

 - ✅ **Fix(graph)**: Added deterministic tie-breaker to `find_entities_by_name_prefix`.
 - ✅ **Test(e2e)**: Added 4 new per-language suites and ported Kotlin signature tests.
---

## v1.4.3 — E2E Isolation & Cleanup

 - ✅ **Refactor(e2e)**: Adopted per-suite fixture directory architecture for better test isolation.
 - ✅ **Chore**: Ignore knot state files in fixture directories.
---

## v1.4.2 — Lightweight Mode Removal
 - ✅ **Removed Lightweight Mode**: Deprecated and removed the "only-clients" mode and the `only-clients` feature flag. All builds now include semantic search capabilities (ONNX Runtime + fastembed) by default, as modern deployment environments now provide the necessary GLIBC version.
 - ✅ **Simplified `Embedder`**: Eliminated stub implementations in favor of the full embedding pipeline.
 - ✅ **Cleaned Documentation**: Updated README, Dockerfiles, and tool descriptions to reflect that semantic search is now a standard feature.
 - ✅ **cargo fmt** clean | **cargo clippy** clean
---

## v1.4.1 — Repository Management, CLI Modularization & Optimized Indexing
 - ✅ **Repository Management**: Added `knot repos` command to list all indexed repositories with entity/file counts and primary language detection.
 - ✅ **Optimized Initial Indexing**: The pipeline now detects full indexing runs and short-circuits stale data cleanup using a single bulk repository wipe, significantly speeding up first-time indexing on populated databases.
 - ✅ **CLI Modularization**: Refactored argument models and query logic into dedicated submodules for better maintainability.
 - ✅ **Enhanced E2E Infrastructure**: Added support for `KNOT_E2E_EXTERNAL_DB` to allow running test suites against a shared database. Introduced `run_all_e2e_fast.sh` for parallel-safe test execution.
 - ✅ **cargo fmt** clean | **cargo clippy** clean
---

## v1.4.0 — Major Refactor, Cleanup & Specialized Build System Parsers
 - ✅ **Specialized Build System Extraction**: Activated the dedicated Gradle (`.gradle`) and Jenkinsfile parsers. These now extract project identities, dependencies, plugins, and pipeline stages/steps with higher precision than the generic Groovy parser.
 - ✅ **Major Code Deduplication**: Consolidated redundant logic across the parser pipeline.
   - Unified `extract_type_references` across TypeScript, Java, and Kotlin.
   - Extracted shared string and AST utilities into `pipeline::parser::utils`.
   - Centralized repo-path and dependency-list resolution in `Config`.
 - ✅ **Enhanced Test Infrastructure**: Added comprehensive AST node finders and assertions to `test_utils.rs`, significantly reducing boilerplate in parser unit tests.
 - ✅ **Bug Fixes**: Repaired indentation-sensitive Python test fixtures that were failing due to malformed raw strings.
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **760 unit tests** passing
---

## v1.3.13 — Rust Macro Call Resolution & Test Context Tracking
 - ✅ **Fixed AST Blind Spot (Macro Calls)**: The AST extractor now descends into `token_tree` nodes to recover function calls wrapped in macros (e.g., `assert!(...)`, `vec![...]`, `println!(...)`). This rescues thousands of missing references across codebases and drastically improves `find_callers` accuracy for both test suites and production code.
 - ✅ **Test Context Tracking**: Added the `is_test_context` boolean flag to entities. The indexer now tracks `#[cfg(test)]` and `#[cfg_attr(test, ...)]` module boundaries, propagating this flag to Neo4j so MCP clients can visually distinguish test-driven references from production usages.
 - ✅ **Inline Module FQNs**: Entities declared inside inline modules (`mod tests { ... }`) now include the inline path in their FQN (e.g., `crate::module::tests::test_foo`). This stops name collisions between identical test functions defined in different files.
 - ✅ **1 New E2E Test Suite**: `run_rust_test_module_e2e.sh` validates FQN isolation, test flags, and macro-wrapped call resolutions end-to-end.
 - ✅ **cargo fmt** clean | **cargo clippy** clean (repaired 4 warnings) | **717 unit tests** passing
 - ✅ **12/12 E2E test suites pass**
---

## v1.3.12 — Rust Qualified-Call Resolution & Import/Use Relationship Capture
 - ✅ **Rust method FQN is now `Type::method`**: Methods inside `impl Foo { ... }`
   and `impl Bar for Foo { ... }` blocks are indexed with the qualified FQN
   `Foo::method` (e.g., `KnotMcpHandler::new`, `WidgetA::new`,
   `Logger::new`). Two structs sharing a `new` method can now be
   disambiguated; Strategy 2 of the call resolver (uppercase receiver) lands
   the `Calls` edge on the right `Type::method` target.
 - ✅ **Receiver preserved in `Type::method()` calls**: `KnotMcpHandler::new(...)`
   from a top-level function is now reported as a caller of
   `KnotMcpHandler::new`. Multi-segment paths like
   `crate::mcp_handler::KnotMcpHandler::new` correctly use the penultimate
   segment as receiver.
 - ✅ **`Self::method()` translated to enclosing class** and
   **`impl Trait for Type` self-type extraction**: The class context for
   methods uses the self-type (`Foo`), not the trait (`LogSink`). Generics in
   the impl head (`impl<T> Foo<T>`) are dropped, producing `Foo::method`
   regardless of generic parameters. `Self::helper` inside `impl Foo`
   resolves to `Foo::helper` via the local-call strategy.
 - ✅ **`find_references` returns `target_fqn`**: The CLI/MCP now displays
   `WidgetA::new` (or whatever the FQN is) instead of just `new` when there
   are homonymous targets. Improves disambiguation in `find_callers`
   output.
 - ✅ **Cross-language import capture** — every `use`/`import` statement
   produces explicit REFERENCES edges in the graph:
   - **Rust**: `use foo::{Bar, Baz}` (nested braces), `use foo::Bar as Baz`
     (emits `Bar`, not `Baz`), glob imports `use foo::*` (skipped).
   - **TypeScript/JavaScript**: `import { Foo } from './x'`, `import Foo as
     Bar`, destructured `require` (`const { Foo } = require('./m')`).
   - **Java**: `import com.example.Foo` (TypeReference), `import static
     Util.helper` (TypeReference + ValueReference). Wildcard imports skipped.
   - **Kotlin**: `import com.example.Foo` (TypeReference), aliased imports
     (`as Bar`) emit original name. Wildcard imports skipped.
 - ✅ **`knot explore` enhancement**: New "Imports / Referenced Types"
   section shows outgoing cross-file REFERENCES/CALLS/EXTENDS/IMPLEMENTS
   edges for any file.
 - ✅ **3 new E2E tests** in `run_rust_e2e.sh` covering qualified-call
   resolution, homonymous `new` disambiguation, and `impl Trait for Type`
   FQN correctness.
 - ✅ **18 new unit tests** for language import scenarios + **6 new unit
   tests** in `rust.rs` (scoped-call receiver extraction, `Self::method`
   translation, FQN re-computation) + **2 new unit tests** in `context.rs`
   (`impl_item` class context, `impl Trait for Type` self-type) + **2 new
   unit tests** in `resolve.rs` (qualified call homonym disambiguation,
   `Self::method` resolution).
 - ⚠️ **Breaking change**: Run `knot-indexer --clean` once after upgrading.
   Rust method FQNs are now stored as `Type::method` instead of bare
   `method`; existing entries in Neo4j are not auto-migrated.
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **633 unit tests**
   passing | **11/11 E2E test suites pass**
---

## v1.3.11 — Cross-File Alias Resolution & Circular Require Fix
 - ✅ **JS/TS Cross-File Alias Extraction**: New extractor pass resolves `require()` aliases (CommonJS) and `import { X as Y }` aliases (ES Modules) across file boundaries. `module.exports = X` and `export default X` targets are tracked via default export metadata, enabling `find_callers` to trace through aliases to the original definition.
 - ✅ **Circular Require Busy-Loop Fix**: Indexing repositories with circular `require()` chains (e.g., `a.js` requires `b.js` which requires `a.js`) previously caused 100% CPU busy-loops in the reference resolution phase. Now detects cycles deterministically (picking the smallest UUID as canonical representative) and collapses the alias chain to a single hop, eliminating infinite loops.
 - ✅ **E2E Suite Port Contention Fix**: Suite cleanup (`docker compose down -v`) now runs before the failure bail-out path, preventing stale port 17687 from causing cascading failures in downstream test suites. Port pre-flight check in `run_all_e2e.sh` forces teardown of orphaned containers between suites.
 - ✅ **3 new fields** on `ParsedEntity`/`ResolutionEntity`: `alias_module_path`, `original_export_name`, `default_export` — persisted to Neo4j and wired through all db/test/benchmark layers.
 - ✅ **5 new unit tests** for alias cycle detection and resolution correctness + **4 new E2E tests** covering JS alias, TS alias, and circular require scenarios.
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **604+ unit tests passing**
 - ✅ **11/11 E2E test suites pass**
---

## v1.3.10 — Prefix Name Match Boost & TypeScript Value/Emitter
 - ✅ **Fixed Subgraph Disconnection**: Automatically injects `CONTAINS` relationships in traversal paths when kind-filtering is active, ensuring class-to-class paths through methods are discovered.
 - ✅ **Fixed Edge Extraction Bug**: Replaced parameter binding for UUID lists with direct Cypher interpolation to bypass a driver-level serialization bug that caused missing edges (0 edges found).
 - ✅ **Constrained Relationship Output**: Constrains direct edges to the requested types, preventing internal structural edges from leaking into the result.
---

## v1.3.9 — Kotlin Enhancements & Lint
- **Feat(kotlin)**: Enum/interface classification and inheritance extraction (E2E + unit tests).
- **Chore(lint)**: Clippy fixes (`vec![]` over `Vec::new()` + push).
---

## v1.3.8 — Subgraph Connectivity & Edge Extraction Fixes
- **Fix(subgraph)**: `CONTAINS` relationships are injected into traversal paths when kind-filtering is active, keeping class-to-class paths connected.
- **Fix(graph)**: UUID list parameters are interpolated directly into Cypher to bypass a driver-level serialization bug that dropped edges.
---

## v1.3.7 — Kind-Aware Subgraph Traversal
 - ✅ **Kind-Aware Subgraph Traversal**: New `visible_kinds` parameter for `get_entity_subgraph`.
 - ✅ **Synthetic Edge Roll-up**: Automatically connects visible nodes through hidden intermediaries (e.g., methods/functions) when filtering by kind.
 - ✅ **Improved Graph Connectivity**: Prevents disconnected subgraphs when focusing on specific entity kinds.
---

## v1.3.6 — Java Indexing Enhancement
---

## v1.3.5 — E2E Stabilization
- **Test(e2e)**: Stabilized E2E tests (Qdrant eventual-consistency retry pattern).
---

## v1.3.4 — Indexer Ambiguity & E2E Resilience
- **Fix(indexer)**: Fixed indexer ambiguity handling.
- **Test(e2e)**: Improved E2E test resilience.
---

## v1.3.3 — Fix Custom CA Certs behind Proxy
 - ✅ **Fix Custom CA Certs behind Proxy**: Switched `fastembed` feature from `hf-hub` to `hf-hub-native-tls`. This ensures that model downloads respect `SSL_CERT_FILE` and the system's CA trust store by using OpenSSL/native-tls instead of the static Mozilla bundle (webpki-roots) bundled with rustls.
 - ✅ **Fixed `inject_custom_ca_certs`**: Removed incorrect setting of `SSL_CERT_DIR` to a file path, ensuring proper TLS initialization.
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **548 unit tests** passing
 - ✅ **12/12 E2E test suites pass**
---

## v1.3.2 — Entity Subgraph Traversal
 - ✅ **Entity Subgraph Retrieval**: New `get_entity_subgraph` query method that traverses the entity graph starting from a root entity and returns all reachable nodes and edges within a configurable depth (1–5). Supports filtering by relationship type (`CALLS`, `EXTENDS`, `IMPLEMENTS`, etc.) and direction (`Outgoing`, `Incoming`, `Both`). Includes deduplication, truncation at configurable `max_nodes`, and edge extraction between collected nodes. Available via the library API (`QueryExt::get_entity_subgraph`) and `cli_tools::run_get_subgraph` wrapper.
 - ✅ **New Data Models**: `SubgraphNode`, `SubgraphEdge`, `SubgraphResult`, and `SubgraphDirection` enums exported from `knot::models`
 - ✅ **6 new Neo4j integration tests** for the subgraph functionality
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **548 unit tests** passing
 - ✅ **12/12 E2E test suites pass**
---

## v1.3.1 — Housekeeping & Roadmap
- **Chore**: Removed the completed migration spec, updated the roadmap for v1.3.0, and gitignored `plan.md`. (Entity-subgraph feature notes are recorded under v1.3.2.)
---

## v1.3.0 — Consolidated `.knot/` Directory
---

## v1.2.8 — MCP Stdout Log Fix
 - ✅ **Bug Fix: MCP Server Logging to stdout**: Fixed `init_logging()` in `src/utils/mod.rs` — log output was written to stdout (default `tracing_subscriber::fmt` behavior), which corrupted MCP JSON-RPC communication over stdio transport since MCP clients read JSON from stdout. Added `.with_writer(std::io::stderr)` to redirect all tracing output to stderr, matching the existing `init_logging_for_cli()` function that already had this fix.
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **548 unit tests** passing
 - ✅ **12/12 E2E test suites pass**: JS/TS/Java, Kotlin, Rust, Python, Build Systems, Config Files, K8s/Helm, Groovy, Cross-Language Ref, C/C++, Cross-Repo Dependencies
---

## v1.2.7 — Cargo Cross-Repo Dependency Fixes
 - ✅ **Bug Fix: Cargo Cross-Repo DEPENDS_ON Edges**: Fixed `match_dependency_to_repository` in `src/pipeline/ingest/resolve.rs` — the Cargo branch was checking for `"scope: compile"` in `dep_name`, but that text lives in `entity.signature` (not `entity.name`). Cargo dependency names are formatted as `"crate_name:version"` by the parser. The condition silently failed for all Cargo dependencies, preventing `DEPENDS_ON` edges from being created. Now correctly extracts the crate name by splitting on `:` and taking the first part.
 - ✅ **Bug Fix: Test Fixtures Overwriting Repository Identity**: Fixed `link_cross_repo_dependencies` in `src/pipeline/ingest/resolve.rs` — when a repository contains multiple `ProjectIdentity` entities (e.g., `Cargo.toml` at root + `tests/testing_files/sample_build.gradle` as a test fixture), `upsert_repository` was called for ALL of them. Since it uses `MERGE + SET`, the last identity processed would overwrite `build_system`, `group_id`, and `artifact_id` with test-fixture data. Now selects only the `ProjectIdentity` closest to the repository root (minimum directory depth), preventing test fixtures in subdirectories from corrupting the repository identity.
 - ✅ **E2E Test: Multi-ProjectIdentity Scenario**: Added test that creates a Cargo library crate with both `Cargo.toml` at root and a Gradle build file in `tests/fixtures/`, then verifies the `:Repository` node retains `build_system = "cargo"` and `DEPENDS_ON` edges are created correctly.
 - ✅ **E2E Test: Cargo Cross-Repo Dependency Linking**: Validated `DEPENDS_ON` edges are created for Cargo projects — library crate `rust-lib-a` indexed first, binary crate `rust-bin-b` depending on it indexed second, verified via `knot deps`, MCP `list_repo_dependencies`, and Neo4j Cypher queries.
---

## v1.2.6 — Optional Config Indexing & Bug Fixes
 - ✅ **Optional Config Indexing**: Added `--include-config-files` flag (disabled by default) to skip indexing generic configuration files (YAML, JSON, .properties) and Kubernetes/Helm manifests, improving performance and avoiding indexing secrets. Build-system files (`package.json`, `tsconfig.json`, `pom.xml`, `Cargo.toml`) remain always indexed.
 - ✅ **Bug Fixes**: Fixed `.env` loading to only respect knot's own config directory (`~/.config/knot/.env`) and ignore `.env` files in target repositories to prevent configuration hijacking.
---

## v1.2.5 — Cargo.toml, Config Files, Kubernetes + Helm, Cross-Repo Linking
 - ✅ **Phase 12A — Cargo.toml Parser**: Package metadata, dependencies (simple/table/git/path), features, workspace members via `toml = "0.8"`
 - ✅ **Phase 12B — Configuration Files**: YAML (.yml/.yaml), JSON (.json), Java Properties (.properties) with recursive walk, depth limit 10, leaf-key granularity, lock file exclusions, 500KB file size limit. package.json special handling: npm deps as BuildDependency, scripts as ConfigProperty, ProjectIdentity emission
 - ✅ **Phase 12C — Kubernetes + Helm**: 10 new EntityKind variants (K8sDeployment, K8sService, K8sConfigMap, K8sSecret, K8sIngress, K8sNamespace, K8sResource, HelmChart, HelmValue, HelmTemplateVar). K8s manifest parsing with label/annotation/reference extraction, Helm Chart.yaml/values.yaml/templates support with {{ .Values.X }} variable tracking
 - ✅ **Phase 12D — Cross-Repo Dependency Linking**: Automatic inter-repository call resolution via `:Repository` graph model with `DEPENDS_ON` edges. `ProjectIdentity` marker entity from build files (Maven GAV, Cargo package, npm name). `knot deps` CLI subcommand + `list_repo_dependencies` MCP tool for dependency graph visualization. Retroactive linking for out-of-order indexing
 - ✅ **74+ new unit tests** across 6 parser modules + cross-repo integration tests
 - ✅ **11/11 E2E test suites pass**: JS/TS/Java, Kotlin, Rust, Python, Build Systems (extended), Config Files, K8s/Helm, Groovy, Cross-Language Ref, C/C++, Cross-Repo Dependencies
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **520+ unit tests** passing
---

## v1.2.0 — Config/K8s E2E & Phase 12 Prep
- **Test(e2e)**: Config Files and K8s/Helm E2E suites with fixtures, added to CI; Kotlin companion-object tests moved to `explore_file`.
- **Docs**: Added the unified Phase 12 plan document.
---

## v1.1.0 — Performance Optimization
 - ✅ **Neo4j UNWIND Batching** (Phase 1-2): Replaced N individual `MERGE` queries with single `UNWIND $entities` batch queries — 10-50x speedup on entity/relationship writes
 - ✅ **Bounded Channels** (Phase 3): Parse/embed/res channels bounded with backpressure — peak memory <400MB (was 500MB unbounded)
 - ✅ **Concurrent Ingestion** (Phase 4): JoinSet + Semaphore for parallel Neo4j/Qdrant writes — 2-3x ingestion throughput
 - ✅ **Rayon Thread Pool Config** (Phase 5): Configurable `KNOT_RAYON_THREADS` env var (default N-1 cores)
 - ✅ **Parallel Relationship Resolution** (Phase 6): `par_iter_mut()` for O(N/num_cpus) resolution
 - ✅ **Three-Level Benchmarking Framework** (Section 9):
   - Criterion unit benchmarks: `pipeline_bench`, `graph_upsert_bench`, `channel_backpressure_bench`
   - E2E benchmark script: `tests/benchmark_e2e.sh` with metrics capture
   - CI regression tracking: `scripts/compare_perf_metrics.sh` + `test-performance` job
 - ✅ **Memory targets**: ~300-400MB peak (well below 2GB nice-to-have, far from 5GB hard limit)
 - ✅ **Criterion benchmarks** at `benches/` | **Baseline metrics** at `.perf_metrics/baseline.json`
 - ✅ **cargo fmt** clean | **cargo clippy** clean | **521 unit tests** passing
---

## v1.0.0 — C/C++ Support
 - ✅ Support `.c`, `.cpp`, `.cc`, `.cxx`, `.h`, `.hpp`, `.hh`, `.hxx` files via tree-sitter-c and tree-sitter-cpp
 - ✅ Intelligent auto-detection of `.h` files to parse them as C++ if they contain classes, namespaces, or templates
 - ✅ Namespace-aware FQN resolution (`Engine::MyClass::start`)
 - ✅ Class, struct, function, and method extraction with full signatures
 - ✅ Macro definition and usage tracking (uppercase identifier heuristic)
 - ✅ Type reference tracking (declarations, `new` expressions, qualified types)
 - ✅ Call graph analysis including method calls, field access (`obj->method()`), and scope resolution (`std::vector::size()`)
 - ✅ 3 unit tests for C++ entity and reference extraction
 - ✅ 4 end-to-end integration tests covering FQN, call graphs, macro usage, and type references
---

## v0.10.3 — Groovy Private Methods, Nested Closures & UUID Collision Fix
 - ✅ **UUID Collision Fix**: `ParsedEntity` identity now includes `start_line`
 - ✅ **Multi-line Method Extraction**: `try_extract_typed_method_multiline` handles closure default params
 - ✅ **Innermost Assignment**: method calls in nested closures go to the innermost method
 - ✅ **10 E2E test cases**: typed/`def`/no-paren callers, multi-line closures, innermost assignment
 - ✅ **441 unit tests | clippy clean | fmt applied**
---

## v0.10.2 — Groovy Parsing Fixes
- **Fix(groovy)**: No-paren call detection, entity deduplication, and reference-intent merging.
---

## v0.10.1 — Groovy Language Support (Phase 10)
- **Feat(parser)**: Full Groovy language support with ad-hoc reference extraction and a dedicated E2E suite.
---

## v0.10.0 — Build Systems & CI/CD Support
 - ✅ **Build Systems Support (Phase 9)**: Maven `pom.xml` (dependencies + plugins via roxmltree), Gradle `build.gradle` (deps + plugins + tasks), and Jenkinsfile pipeline (stages + steps) extraction
 - ✅ **22 unit tests + 8 E2E tests** (Maven search, pom.xml explore, Gradle dep/task search, Jenkins stage/step search)
 - ✅ **BuildDependency, BuildPlugin, BuildTask, PipelineStage, PipelineStep entity kinds** with explore_file formatting
---

## v0.9.3 — Python Search Stability & CI Fixes
 - ✅ Fixed CLI `explore` & `search` queries that queried the default collection instead of test collection by appending `-r "$REPO_NAME"`
 - ✅ Python CLI search bug handled; resolved `knot search` queries failing in specific collection bounds
 - ✅ Replaced unreliable `nc -z` network checks with Neo4j-specific Docker health checks (`docker inspect`)
 - ✅ 426 unit tests | 23 Python E2E | 22 Rust E2E | 10 Kotlin E2E
---

## v0.9.2 — Python Resolution Fixes
- **Fix(python)**: `self.method()` resolution and inherited-method resolution; CI fixes.
---

## v0.9.1 — Python References (Phases 3-6)
- **Feat(parser)**: Python calls, imports, inheritance, decorators, and type hints.
---

## v0.9.0 — Python Extraction (Phase 2)
- **Feat(parser)**: Python class, function, method, async, and lambda extraction.
---

## v0.8.12 — Python Support (Phase 1)
- **Feat(parser)**: Python (Phase 1) via `tree-sitter-python` — entity kinds, pipeline wiring, and CI E2E coverage.
---

## v0.8.11 — Rust Support
 - ✅ Support `.rs` files with tree-sitter-rust parser
 - ✅ Struct, enum, union, trait, and impl block extraction
 - ✅ Function, method, macro definition and invocation tracking
 - ✅ Type alias, constant, static, and module extraction with signatures
 - ✅ Docstring extraction for all Rust entity types
 - ✅ O(N) nested macro traversal optimization for large Rust codebases
 - ✅ 17 unit tests for Rust entity and reference extraction
 - ✅ 22 end-to-end integration tests covering all Rust language constructs
---

## v0.8.10 — CLI UX & Corporate Network Support
 - ✅ **Human-friendly output formatting**: Colorized table output as default with per-entity-kind ANSI colors
 - ✅ **Interactive result navigation**: Pager support via `less -R -e` with auto-exit at end of content
 - ✅ **Configurable output formats**: `--output` flag supports `table` (default), `json`, and `markdown`
 - ✅ **Custom CA Certificates**: `--custom-ca-certs` / `KNOT_CUSTOM_CA_CERTS` for corporate SSL-inspecting proxies
 - ✅ **O(N) Macro Traversal Optimization** (v0.8.11): Substring skipping for deeply nested `token_tree` nodes
---

## v0.8.7 — Enhanced Rust Type Reference Detection in Macros
 - ✅ **Macro Type Reference Extraction**: Type references inside macro invocations (`vec![]`, `println!()`, `assert!()`, `format!()`, etc.) are now correctly captured
 - ✅ **Intelligent String Filtering**: Filters out false positives from string literals using quote-counting heuristics
 - ✅ **Comprehensive Edge Case Handling**: Validates identifiers, handles nested macros, supports `macro_rules!` definitions
 - ✅ **Improved Accuracy**: EntityKind references increased by +95.7% (46→90 references), now captures test function usage
 - ✅ **Enhanced Test Coverage**: Added 4 new tests for token_tree extraction covering various macro types and edge cases
---

## v0.8.6 — Rust Type Aliases, Constants, and Docstrings
 - ✅ **Rust Type Alias Extraction**: Extracts type alias declarations with full signature (e.g., `type Callback = fn(u32) -> u32`)
 - ✅ **Rust Constant/Static Extraction**: Captures `const` and `static mut` declarations with type signatures
 - ✅ **Rust Docstring Support**: Full doc comment extraction for Rust entities (handles nested `doc_comment` nodes in tree-sitter-rust)
 - ✅ **Rich Vector Embeddings**: Type signatures and documentation are now included in embeddings for better semantic search
 - ✅ **Improved Search Ranking**: Rust entities like `Callback` now rank in top 5 search results when querying by name
---

## v0.8.5 — Rust Module Refactoring & Clippy Fixes
 - ✅ **Rust Module Refactoring**: Extracted Rust parsing logic into dedicated `src/pipeline/parser/languages/rust.rs` for better maintainability and mirroring existing language module architecture.
 - ✅ **Clippy Compliance**: Fixed unused import (`uuid::Uuid`) and unnecessary `mut` warning in Rust module tests.
 - ✅ **Rust Support Complete**: Phase 8 implementation fully integrated with 17 unit tests and 22 E2E test cases passing.
---

## v0.8.4 — Agent-Skills Documentation Installer & Lightweight Clients
 - ✅ **Dry-Run Mode**: MCP server can run in offline mode for quality checks on deployment platforms.
 - ✅ **Platform-Agnostic**: Removed all platform-specific references; compatible with any deployment platform.
 - ✅ **Enhanced Reliability**: Graceful handling of missing database connections for validation scenarios.
---

## v0.8.3 — MCP Dry-Run Mode for Deployment Quality Checks
- **Feat(mcp)**: Dry-run mode for running the MCP server in deployment-platform quality checks without live databases.
---

## v0.8.2 — Quality & Doc Refactor
 - ✅ **MCP Quality**: Enhanced tool descriptions for better agent discovery and usage safety.
 - ✅ **Token-Efficient Docs**: Modularized agent skill guide into `docs/agent-skills/` for on-demand loading.
 - ✅ **Rust Phase 1**: Infrastructure prepared for Rust 2024 integration.
 - ✅ **Rust Phase 2-5**: Complete Rust language support including entity extraction, macro tracking, and comprehensive E2E testing (v0.8.x).
---

## v0.8.1 — CLI UX & Docker Integration
 - ✅ **Silenced CLI Logs**: Default log level set to `error` for `knot` CLI (cleaner Markdown output).
 - ✅ **100% E2E Dual-Testing**: All 35 integration tests simultaneously verify both MCP and CLI.
 - ✅ **Docker CLI Support**: Official Docker image now includes the `knot` binary.
 - ✅ **Agent Guidance**: Enhanced `.knot-agent.md` with signature-based search warnings.
