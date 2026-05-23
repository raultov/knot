# Multi-Language Roadmap for Knot

This document outlines the planned expansion of `knot` to support Python and C/C++ codebases, building on the existing foundation for Java, TypeScript, JavaScript, Kotlin, Rust, HTML, CSS, and SCSS.

---

## Overview

**Current State (v1.3.4):**
- Java, Kotlin, TypeScript/TSX, JavaScript/Node.js, Rust, Python, Groovy, HTML, CSS, SCSS support
- Typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES, ValueReference)
- Build Systems: Maven (pom.xml), Gradle (build.gradle), Jenkinsfile, Cargo.toml extraction
- Configuration Files: YAML (.yml/.yaml), JSON (.json), Java Properties (.properties) with leaf-key granularity and package.json special handling
- Kubernetes + Helm: K8s manifest parsing (Deployment, Service, ConfigMap, Secret, Ingress, Namespace) and Helm chart indexing (Chart.yaml, values.yaml, templates)
- Dual-database architecture (Qdrant + Neo4j)
- Three MCP tools (search_hybrid_context, find_callers, explore_file) + list_repo_dependencies
- Cross-Repo Dependency Linking (v1.2.5) with auto-discovered DEPENDS_ON edges and retroactive linking
- Entity Subgraph Traversal (v1.3.2): `get_entity_subgraph` query with configurable depth, relationship filtering, and direction
- Standalone CLI Tool (`knot`) with full MCP parity
- Colorized table output, interactive pager, configurable output formats (table/json/markdown)
- Custom CA certificates support for corporate network downloads
- O(N) nested macro traversal optimization for large Rust codebases
- Consolidated `.knot/` directory: fastembed model cache now stored in `.knot/fastembed_cache/` (configurable via `KNOT_FASTEMBED_CACHE_DIR`)
- 550+ unit tests | 100+ E2E tests across all languages
- Fix Indexer Ambiguity & Context Deduplication (v1.3.4)

---

## Phase 8: Python Support (v0.9.x)


### Objective
Enable `knot` to index Python codebases with full semantic understanding of AST, classes, decorators, and module dependencies.

#### Planned → ✅ Implemented
- ✅ tree-sitter-python integration
- ✅ Class, function, method, constant, and module extraction
- ✅ Import resolution and cross-module dependency graph (`TypeReference`, `REFERENCES` edges)
- ✅ `ValueReference` tracking for keyword argument patterns (`action=ClassName`)
- ✅ Class inheritance (`EXTENDS` relationships via `argument_list` traversal)
- ✅ Decorator extraction (`@staticmethod`, `@property`, `@dataclass`, `@route(...)`) with `CALLS` relationships
- ✅ Generic type hints (`List[str]`, `Optional[Dict]`), `*args`/`**kwargs` parameter extraction
- ✅ Py2/Py3 exception syntax compatibility testing
- ✅ 5 Phase 6 unit tests, 4 Phase 6 E2E tests (23 total Python E2E tests)

---

## Phase 9: Build Systems & CI/CD Support (v0.10.0)

### Objective
Enable `knot` to index project infrastructure and build configurations. By understanding `build.gradle`, `pom.xml`, and `Jenkinsfile`, the MCP server will be able to answer semantic questions about project dependencies, custom build tasks, and deployment pipeline stages.

#### Planned → ✅ Completed
- ✅ Maven pom.xml extraction via roxmltree (dependencies + plugins)
- ✅ Gradle build.gradle extraction via custom Groovy parser (deps + plugins + tasks)
- ✅ Jenkinsfile pipeline extraction via custom Groovy parser (stages + steps)
- ✅ BuildDependency, BuildPlugin, BuildTask, PipelineStage, PipelineStep entity kinds
- ✅ 22 unit tests (6 XML + 12 Gradle + 4 Jenkins)
- ✅ 8 E2E tests (Maven search, pom.xml explore, Gradle dep/task search, Jenkins stage/step search)
- ✅ Explore_file output formatting for all 5 new entity kinds

---

## Phase 10: Full Groovy Language Support (v0.10.1 — ✅ Completed)

### Objective
Enable `knot` to parse and semantically understand standard Groovy source files beyond build scripts, integrating `tree-sitter-groovy` into the Rust pipeline.

#### Planned → ✅ Completed
- ✅ tree-sitter-groovy v0.1.2 integration
- ✅ 7 new EntityKind variants: GroovyClass, GroovyInterface, GroovyTrait, GroovyMethod, GroovyFunction, GroovyEnum, GroovyProperty
- ✅ groovy.scm tree-sitter queries for class/interface/enum/method/field/constructor extraction
- ✅ Standard .groovy file parser (extract_entities_groovy_standard) sharing JVM reference extraction with Java
- ✅ Hybrid tree-sitter + ad-hoc lexical parser for `def`-keyword, `trait`, and Spock-quoted method names
- ✅ Ad-hoc reference extraction: `extract_method_calls()` populates `reference_intents` from Groovy method bodies via line-span filtering
- ✅ Cross-language references: Java→Groovy and Groovy→Groovy `find_callers` (via Neo4j CALLS edges)
- ✅ Explore_file Markdown sections (Classes/Interfaces/Traits/Enums/Methods/Functions/Properties Groovy)
- ✅ 10 new unit tests + 5 embedding tests + 8 FQN/scope/resilience tests + 8 E2E tests (cross-lang + groovy-only)
- ✅ E2E suites: `run_groovy_e2e.sh` (5/5), `run_cross_lang_ref_e2e.sh` (6/6), `run_groovy_cross_ref_e2e.sh` (4/4)

---

## Phase 11: C/C++ Support (v1.0.0 — ✅ Completed)

### Objective
Enable `knot` to index C and C++ codebases with full support for namespaces, classes, methods, pointers, and macros.

#### Planned → ✅ Completed
- ✅ tree-sitter-c v0.23 and tree-sitter-cpp v0.23 integration
- ✅ 6 new EntityKind variants: CppClass, CStruct, CppMethod, CFunction, CppNamespace, MacroDefinition
- ✅ Namespace-aware FQN resolution (`Engine::MyClass::start`)
- ✅ cpp.scm and c.scm tree-sitter queries for entity extraction
- ✅ `build_cpp_fqn()` for dynamic FQN construction via AST parent traversal
- ✅ `extract_reference_intents_cpp()` and `extract_call_intents_cpp()` for call graph analysis
- ✅ Call expression handling: direct calls, object/pointer access, scope resolution (`std::vector::size()`)
- ✅ Type reference tracking: declarations, `new` expressions, qualified types
- ✅ Macro usage detection via uppercase identifier heuristic
- ✅ C/C++ reference integration in `orphans.rs` for `collect_all_reference_intents_with_byte_pos`
- ✅ 3 new C++ unit tests (307 lines in cpp.rs)
- ✅ 4 E2E tests in `run_cpp_e2e.sh`: FQN extraction, call graphs, macro usage, type references
- ✅ CI updated with C/C++ and Cross-Language Ref E2E tests
- ✅ Master E2E script (`run_all_e2e.sh`) to run all 10 test suites locally

#### Implementation Files
- `src/pipeline/parser/languages/cpp.rs` — main C++ parser module
- `queries/c.scm` — C tree-sitter queries
- `queries/cpp.scm` — C++ tree-sitter queries
- `tests/run_cpp_e2e.sh` — C++ E2E test suite
- `tests/run_all_e2e.sh` — master E2E runner

---

---

## Phase 12: Build Systems, Config Files & Kubernetes/Helm (v1.2.0 — ✅ Completed)

### 12A — Cargo.toml Parser
- ✅ `toml = "0.8"` integration with package metadata, multi-format dependency parsing, features, workspace members
- ✅ 4 new EntityKind variants: CargoPackage, CargoFeature, WorkspaceMember, ProjectIdentity
- ✅ 8 unit tests + 6 E2E tests (extended `run_build_systems_e2e.sh`)

### 12B — Configuration Files (YAML, JSON, .properties)
- ✅ `serde_yaml = "0.9"` integration. Recursive walk with depth limit 10, multi-document YAML
- ✅ package.json special handling: deps → BuildDependency, scripts → ConfigProperty, ProjectIdentity
- ✅ .properties: line-by-line parser with `=`/`:`/space delimiters, comments, line continuation
- ✅ ConfigProperty EntityKind. 500KB file size limit + lock file exclusions
- ✅ 18 unit tests (6/6/6) + 6 E2E tests (`run_config_e2e.sh`)

### 12C — Kubernetes + Helm
- ✅ 10 new EntityKind variants (7 K8s + 3 Helm)
- ✅ Multi-resource K8s YAML with label/annotation/spec extraction and cross-resource references
- ✅ Helm: Chart.yaml, values.yaml, template {{ .Values.X }} extraction with `{{-` whitespace trim
- ✅ `dispatch_yaml()` cascade: Chart.yaml → Helm directory → K8s (apiVersion+kind) → generic YAML
- ✅ 15 unit tests (7+5+3) + 9 E2E tests (`run_k8s_helm_e2e.sh`)

**Total Phase 12: 6 new parsers, 16 EntityKind variants, 41 unit tests, 29 E2E tests**

---

## Implementation Priority & Timeline

| Phase | Complexity | Status |
|-------|-----------|--------|
| Phase 1-6: JS/HTML/CSS/Kotlin/CLI | - | ✅ Completed |
| Phase 7: Rust | High | ✅ Completed (v0.8.11) |
| Phase 8: Python | High | ✅ Completed (v0.9.3) |
| Phase 9: Build Systems (Maven/Gradle/Jenkins) | Medium | ✅ Completed (v0.10.0) |
| Phase 10: Groovy Language Support | Medium | ✅ Completed (v0.10.3) |
| Phase 11: C/C++ | High | ✅ Completed (v1.0.0) |
| Phase 12: Performance Optimization | High | ✅ Completed (v1.1.0) |
| Phase 12A-C: Cargo.toml, Config, K8s/Helm | Medium | ✅ Completed (v1.2.0) |

---

## Backward Compatibility

- All new language phases are backward compatible
- No database migration needed: new entity types added dynamically
- MCP tools and CLI work seamlessly with existing indexed data

---

## Changelog

### v1.3.4 - Fix Indexer Ambiguity & Context Deduplication

**Relationship Resolution Accuracy:**
- ✅ **Fix Indexer Ambiguity**: Implemented strict uniqueness guards in relationship resolution. Global fallbacks now only create links if exactly one unambiguous candidate exists.
- ✅ **Context Deduplication**: Added sorting and deduplication for UUID mappings in `name_to_uuids`, preventing redundant entries from interfering with uniqueness checks.
- ✅ **Improved Resolution Priority**: Reordered fallback logic to prioritize local same-file matches before global filters.
- ✅ **Enhanced Test Coverage**: Added comprehensive unit tests in `resolve.rs` for uniqueness guards and deduplication.
- ✅ 12/12 E2E test suites pass (C++, Groovy, and Cross-Language suites fixed).

### v1.3.3 - Fix Custom CA Certs behind Proxy

**TLS & Proxy Support:**
- ✅ Switched `fastembed` feature to `hf-hub-native-tls` to support corporate proxies via OpenSSL/native-tls.
- ✅ Fixed `inject_custom_ca_certs` to only set `SSL_CERT_FILE`, avoiding incorrect `SSL_CERT_DIR` file-path assignments.
- ✅ 548 unit tests passing | cargo fmt + clippy clean
- ✅ 12/12 E2E test suites pass

### v1.3.2 - Entity Subgraph Traversal

**Entity Subgraph Retrieval (`get_entity_subgraph`):**
- ✅ New `get_entity_subgraph` query method in `QueryExt` trait + full `GraphDb` implementation
- ✅ Traverses the entity graph from a root entity, returning reachable nodes and edges
- ✅ Configurable depth (1–5), relationship type filtering (any combination of CALLS, EXTENDS, IMPLEMENTS, REFERENCES, etc.), and direction (Outgoing/Incoming/Both)
- ✅ Cypher validation: rejects invalid relationship types with a clear error message
- ✅ Deduplication via UUID-based `HashMap`, truncation at configurable `max_nodes` with `truncated` flag
- ✅ Edge extraction between collected nodes for graph visualization
- ✅ New data models: `SubgraphNode`, `SubgraphEdge`, `SubgraphResult`, `SubgraphDirection` (with `#[derive(Default)]`)
- ✅ CLI/MCP wrapper: `cli_tools::run_get_subgraph` with `DEFAULT_MAX_NODES = 500`
- ✅ 6 new Neo4j integration tests: not_found, valid_entity, invalid_relationship, outgoing_only, multiple_relationships, truncation
- ✅ 548 unit tests | cargo fmt + clippy clean

### v1.3.0 - Consolidated `.knot/` Directory & Test Resilience

**Fastembed Cache Consolidation:**
- ✅ The `fastembed` model cache (previously `.fastembed_cache/` in CWD) now stores inside `.knot/fastembed_cache/` by default
- ✅ Configurable via `KNOT_FASTEMBED_CACHE_DIR` env var for shared caching across multiple repos
- ✅ All three binaries (knot-indexer, knot, knot-mcp) pass the cache dir to `Embedder::init(cache_dir: PathBuf)`
- ✅ New `fastembed_cache_dir()` helper in `src/pipeline/state.rs` with env var override support

**Test Resilience:**
- ✅ Fixed 3 unit tests that failed when `KNOT_INGEST_CONCURRENCY` / `KNOT_RAYON_THREADS` env vars were set in the parent shell
- ✅ Introduced serialized env-access pattern via `Mutex` with save/restore semantics for process-wide env var modifications
- ✅ 548 unit tests passing | cargo fmt + clippy clean
- ✅ 12/12 E2E test suites pass

### v1.2.0 - Cargo.toml, Configuration Files, Kubernetes + Helm

**Phase 12A — Cargo.toml Parser:**
- ✅ `toml = "0.8"` integration with package metadata, multi-format deps (simple/table/git/path), features, workspace members
- ✅ 4 new EntityKind variants: CargoPackage, CargoFeature, WorkspaceMember, ProjectIdentity
- ✅ 8 unit tests + 6 E2E tests (extended `run_build_systems_e2e.sh`)

**Phase 12B — Configuration Files:**
- ✅ `serde_yaml = "0.9"` integration. YAML/JSON recursive walk with depth limit 10, multi-document support
- ✅ package.json special handling: npm deps → BuildDependency, scripts → ConfigProperty, ProjectIdentity
- ✅ .properties: line-by-line parser with comment-as-docstring heuristic, line continuation
- ✅ ConfigProperty EntityKind. 500KB file size limit + lock file exclusions
- ✅ 18 unit tests + 6 E2E tests (`run_config_e2e.sh`)

**Phase 12C — Kubernetes + Helm:**
- ✅ 10 new EntityKind variants (K8sDeployment, K8sService, K8sConfigMap, K8sSecret, K8sIngress, K8sNamespace, K8sResource, HelmChart, HelmValue, HelmTemplateVar)
- ✅ Multi-resource K8s YAML with label/annotation/spec extraction and cross-resource references
- ✅ Helm: Chart.yaml, values.yaml, template {{ .Values.X }} extraction with `{{-` whitespace trim
- ✅ `dispatch_yaml()` cascade: Chart.yaml → Helm directory → K8s (apiVersion+kind) → generic YAML
- ✅ 15 unit tests + 9 E2E tests (`run_k8s_helm_e2e.sh`)
- ✅ All 10 E2E test suites pass (Config Files + K8s/Helm added to CI and `run_all_e2e.sh`)
- ✅ 520 unit tests passing | cargo fmt + clippy clean

### v1.1.0 - Performance Optimization (Phase 12)

**Bottleneck Resolution:**
- ✅ Replaced N individual Neo4j `MERGE` queries with batched `UNWIND` — 10-50x entity write speedup, 50,560 queries → <100
- ✅ Replaced O(N) relationship inserts with `UNWIND`-batched queries per type — O(N) → O(8) queries
- ✅ Bounded all pipeline channels — peak memory ~300-400MB (vs 500MB unbounded potential)
- ✅ Concurrent ingestion via JoinSet + Semaphore — 2-3x ingestion throughput improvement
- ✅ Rayon thread pool configurable via `KNOT_RAYON_THREADS` (default N-1 cores)
- ✅ Parallel relationship resolution via `par_iter_mut()` — linear speedup with core count

**Benchmarking Framework:**
- ✅ Three-level performance validation: Criterion unit benchmarks + E2E metrics + CI baseline tracking
- ✅ `benches/pipeline_bench.rs` — parse/prepare throughput per language
- ✅ `benches/graph_upsert_bench.rs` — Neo4j UNWIND batching validation
- ✅ `benches/channel_backpressure_bench.rs` — bounded channel overhead
- ✅ `tests/benchmark_e2e.sh` — full pipeline metrics with `/usr/bin/time` RSS capture
- ✅ `scripts/compare_perf_metrics.sh` — baseline comparison with tolerance gates (±5% time, ±10% memory)
- ✅ `test-performance` job in CI — runs after E2E, fails on regression, updates baseline on main/master merges
- ✅ `.perf_metrics/baseline.json` — committed baseline (updated on main/master only)
- ✅ `.perf_metrics/threshold_tolerances.json` — configurable tolerance thresholds
- ✅ 521 unit tests passing | clippy clean | fmt applied

### v1.0.0 - C/C++ Language Support
- ✅ Phase 11 complete: tree-sitter-c v0.23 and tree-sitter-cpp v0.23 integration
- ✅ 6 new EntityKind variants: CppClass, CStruct, CppMethod, CFunction, CppNamespace, MacroDefinition
- ✅ Namespace-aware FQN resolution: `Engine::MyClass::start` format via AST parent traversal
- ✅ Call graph analysis: direct calls, method calls (`obj.bar()`, `ptr->baz()`), scope resolution (`std::vector::size()`)
- ✅ Type reference tracking: declarations, `new` expressions, qualified types
- ✅ Macro usage detection via uppercase identifier heuristic (e.g., `MAX_BUF`)
- ✅ 3 new C++ unit tests (307 lines in cpp.rs) | 443 total unit tests
- ✅ 4 E2E tests in `run_cpp_e2e.sh`: FQN extraction, call graphs, macro usage, type references
- ✅ 10/10 E2E test suites pass (C/C++ E2E included, Build Systems E2E fixed)
- ✅ Master E2E script (`run_all_e2e.sh`) for local validation of all test suites
- ✅ CI updated with C/C++ and Cross-Language Ref E2E tests
- ✅ Groovy Cross-Ref E2E fixed: pass neo4j env vars to knot-indexer
- ✅ `cargo fmt` clean | `cargo clippy --all-targets -- -D warnings` clean
- ✅ Published to crates.io as v1.0.0

### v0.10.3 - Groovy Nested Methods, Innermost Assignment & UUID Collision Fix
- ✅ Fixed UUID collision: `ParsedEntity` identity now includes `start_line` to prevent entities with identical name/FQN in same file from colliding (e.g., multiple `actionPerformed` in `new AnAction` closures)
- ✅ Innermost assignment: method calls in nested closures go to the innermost method, not the outer container (fixes `showGrabbingFinishedMessage`/`hyperlinkUpdate` and `createActionsOnHistoryFile`/`actionPerformed` patterns)
- ✅ Multi-line method signatures: `try_extract_typed_method_multiline` handles closure default params spanning multiple lines
- ✅ Assignment-vs-declaration disambiguation: `=` check and `new Type(...)` constructor rejection
- ✅ Brace-scope `end_line` fix: `find_method_body_end` with string-aware brace matching
- ✅ Removed Java tree-sitter ref extraction for Groovy (`method_invocation` nodes unreliable for closures)
- ✅ `tests/run_groovy_private_method_e2e.sh`: 10 tests (typed/`def`/no-paren callers, multi-line closures, innermost assignment, UIPattern duplicate-name)
- ✅ CI updated: all 3 Groovy E2E suites run on every push/PR
- ✅ 441 unit tests (3 new Groovy tests) | clippy clean

### v0.10.2 - Groovy No-Paren Calls & Private Method Tracking
- ✅ Fixed no-paren Groovy call detection: `extract_method_calls()` recognizes `runAnalyzer "abc", 123` style calls with string literal skipping
- ✅ Fixed entity deduplication: `known_lines` from tree-sitter now gates ad-hoc extraction, preventing duplicate entities
- ✅ Fixed reference intent overwrite: ad-hoc intents use `extend()` instead of `= collect()`, preserving tree-sitter intents
- ✅ Private methods now correctly tracked: `find_callers` finds callers of private typed/`def` Groovy methods
- ✅ 438 unit tests (3 new) + new `run_groovy_private_method_e2e.sh` (5/5)

### v0.10.1 - Groovy Reference Extraction Fix & E2E Test Suite
- ✅ Fixed ad-hoc Groovy parser: `extract_method_calls()` now populates `reference_intents` from method bodies via line-span filtering, enabling CALLS relationships for `def`-based Groovy methods in Neo4j
- ✅ Cross-language `find_callers`: Java→Groovy (Helper.greet, Helper.add, Parser.parse) fully functional
- ✅ Groovy→Groovy cross-class `find_callers`: ClientA.groovy and ClientB.groovy both found as callers of Calculator.add/multiply
- ✅ Added `run_cross_lang_ref_e2e.sh`: 6/6 tests (Java→Groovy)
- ✅ Added `run_groovy_cross_ref_e2e.sh`: 4/4 tests (Groovy→Groovy cross-class refs)
- ✅ E2E suites: all 15 Groovy E2E tests pass across 3 test scripts
- ✅ 435 unit tests passing, clippy clean, fmt applied

### v0.10.0 - Build Systems & CI/CD Support
- ✅ Phase 9 complete: Maven pom.xml, Gradle build.gradle, Jenkinsfile extraction
- ✅ 22 unit tests + 8 E2E tests

### v0.9.3 - Python Search Stability & CI Enhancements
- ✅ Fixed Rust/Kotlin CLI `explore` & `search` queries that queried the default collection instead of test collection by appending `-r "$REPO_NAME"`
- ✅ Python CLI search bug handled; resolved `knot search` queries failing in specific collection bounds
- ✅ Replaced unreliable `nc -z` network checks with Neo4j-specific Docker health checks (`docker inspect`) in CI scripts, eliminating `Connection reset by peer` errors
- ✅ Enforced strict separation and 5s sleep between consecutive container suites in CI

### v0.9.2 - Python self.method() Resolution & CI Fixes
- ✅ `class_definition` recognized by `extract_class_contexts` → `enclosing_class` now set for Python methods
- ✅ `"self"` receiver handled in Strategy 1 (local call resolution) alongside `"this"`
- ✅ EXTENDS walking: inherited `self.method()` calls resolve through parent class chain
- ✅ Unit test: `test_resolve_self_method_inherited_from_parent_class`
- ✅ CI: Python E2E added to GitHub Actions workflow
- ✅ CI: Docker cleanup + sleep 5s between E2E test suites to prevent `Connection reset by peer`
- ✅ 376 unit tests | 23 Python E2E | 22 Rust E2E | 10 Kotlin E2E

### v0.9.1 - Python Phase 6: Advanced Testing & Type Hints
- ✅ Phase 6: Generic type hints (`List[str]`, `Optional[Dict]`), `*args`/`**kwargs` parameter extraction
- ✅ Phase 6: Py2/Py3 exception syntax compatibility verification (`except ValueError, e:` / `except ValueError as e:`)
- ✅ 5 new unit tests for type hints, var args, exception syntax
- ✅ 4 new E2E tests (tests 20-23): process_items, find_user, log_message, handle_exception_py2_style
- ✅ Python support complete: 375 unit tests, 23 E2E tests

### v0.9.0 - Python Support (Phases 1-5)
- ✅ Phase 1: Base configuration, tree-sitter-python integration, `PythonClass`/`PythonFunction`/`PythonMethod` EntityKinds
- ✅ Phase 2: Structural extraction, docstrings, signatures, lambda support
- ✅ Phase 3: Call graph — direct and method calls, `print_statement` (Py2), `CALLS` edges
- ✅ Phase 4: Imports — `import`/`from` detection, `PythonConstant`, `PythonModule`, `REFERENCES` edges
- ✅ Phase 4.5: ValueReferences — `action=ClassName` pattern via `keyword_argument` detection
- ✅ Phase 5: Inheritance (`EXTENDS` edges) and decorator extraction (`CALLS` edges for `@staticmethod`, `@property`, `@dataclass`, `@route(...)`)
- ✅ 19 Python E2E tests, 9 Phase 5 unit tests, 370 total tests passing

### Roadmap Reorganization
- ➕ Added Phase 9: Groovy Support (v0.10.x) — Gradle, Jenkinsfile indexing
- 🔀 Shifted C/C++ to Phase 10 (v0.11.x)
- ✅ O(N) nested macro traversal: Substring skipping eliminates redundant string operations for deeply nested `token_tree` nodes in Rust macros

### v0.8.10 - CLI UX Enhancements & Custom CA Certificates
- ✅ Colorized table output as default format with per-entity-kind ANSI colors
- ✅ Interactive pager support via `less -R -e` with auto-exit at end of content
- ✅ Configurable output formats via `--output` flag (`table` default, `json`, `markdown`)
- ✅ Custom CA certificates support for corporate SSL-inspecting proxies

### v0.8.8 - Corporate Network Support
- ✅ Custom CA certificates support for corporate SSL-inspecting proxies
- ✅ `KNOT_CUSTOM_CA_CERTS` environment variable and `--custom-ca-certs` CLI flag

### v0.8.7 - Enhanced Rust Type Reference Detection
- ✅ token_tree extraction for macro invocations (`vec!`, `println!`, `assert!`, etc.)
- ✅ String literal filtering to avoid false positives in macro bodies
- ✅ Improved accuracy for EntityKind detection (+95.7%)

### v0.8.6 - Rust Initial Support
- ✅ tree-sitter-rust integration
- ✅ Basic entity extraction for Rust codebases

### v0.8.3 - Dry-Run Mode for Deployment Platform Quality Checks
- ✅ Offline/dry-run mode for MCP server without database dependencies

### v0.8.0 - CLI Interface
- ✅ Standalone CLI binary with `search`, `callers`, and `explore` commands
- ✅ Unified core shared between CLI and MCP
