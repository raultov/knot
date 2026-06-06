# Changelog

All notable changes to **knot** are documented here, ordered from most recent to oldest.
For the upcoming roadmap see [README.md → Upcoming](README.md#-roadmap).

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
- ✅ **cargo fmt** clean | **cargo clippy** clean | **604+ unit tests** passing
- ✅ **11/11 E2E test suites pass**

---

## v1.3.10 — Prefix Name Match Boost & TypeScript Value/Emitter

- ✅ **Fixed Subgraph Disconnection**: Automatically injects `CONTAINS` relationships in traversal paths when kind-filtering is active, ensuring class-to-class paths through methods are discovered.
- ✅ **Fixed Edge Extraction Bug**: Replaced parameter binding for UUID lists with direct Cypher interpolation to bypass a driver-level serialization bug that caused missing edges (0 edges found).
- ✅ **Constrained Relationship Output**: Constrains direct edges to the requested types, preventing internal structural edges from leaking into the result.

---

## v1.3.7 — Kind-Aware Subgraph Traversal

- ✅ **Kind-Aware Subgraph Traversal**: New `visible_kinds` parameter for `get_entity_subgraph`.
- ✅ **Synthetic Edge Roll-up**: Automatically connects visible nodes through hidden intermediaries (e.g., methods/functions) when filtering by kind.
- ✅ **Improved Graph Connectivity**: Prevents disconnected subgraphs when focusing on specific entity kinds.

---

## v1.3.6 — Java Indexing Enhancement

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
