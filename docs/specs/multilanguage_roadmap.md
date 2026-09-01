# Multi-Language Roadmap for Knot

This document tracks the language and capability expansion of `knot` beyond the original Java/TypeScript/JavaScript/Kotlin/Rust/HTML/CSS/SCSS foundation. Phases 8-16 are all delivered; the Current State section below reflects v1.8.0.

---

## Overview

**Current State (v1.8.0):**
- Java (v1.3.6), Kotlin, TypeScript/TSX, JavaScript/Node.js, Rust, Python, Groovy, C/C++, C# (v1.7.0), HTML, CSS, SCSS support
- C# (v1.7.0): 16 `CSharp*` entity kinds, namespace-qualified FQNs (`<namespace>.<Type>.<member>`), `OVERRIDES` linking beyond the JVM, XML doc comments, and attributes
- Markdown (v1.4.9): `MarkdownDocument` (one per `.md`/`.markdown` file) and `MarkdownSection` (one per ATX heading H1–H6) with section bodies embedded for full semantic search over docs
- Varnish Cache (v1.5.7, include resolution v1.6.1): `.vcl` / `.vtc` / `.vcc` via a hand-written lexer (no tree-sitter grammar exists), 18 entity kinds and 6 dedicated relationship types (USES_BACKEND, USES_PROBE, USES_ACL, INCLUDES, IMPORTS_VMOD, DECLARED_UNUSED)
- Typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES, CONTAINS, OVERRIDES, ValueReference)
- Build Systems: Maven (pom.xml), Gradle (build.gradle), Jenkinsfile, Cargo.toml, and MSBuild (v1.7.2: `.csproj` + `Directory.Packages.props` CPM → `build_system: "nuget"` with cross-repo DEPENDS_ON edges)
- Configuration Files: YAML (.yml/.yaml), JSON (.json), Java Properties (.properties) with leaf-key granularity and package.json special handling
- Kubernetes + Helm: K8s manifest parsing (Deployment, Service, ConfigMap, Secret, Ingress, Namespace) and Helm chart indexing (Chart.yaml, values.yaml, templates)
- Repository Scope Selection (v1.8.0): `repo_name` (MCP) and `--repo` (CLI) accept a single repository, a comma-separated union list, or the `all` / `*` sentinel — identical semantics on both surfaces, with `(repo: …)` self-labeling on multi-repo results
- Dual-database architecture (Qdrant + Neo4j)
- Five MCP tools (search_hybrid_context, find_callers, explore_file, list_repo_dependencies, list_repositories)
- find_callers target resolution ladder (v1.7.1): exact FQN → FQN suffix → exact name → signature prefix → fuzzy, with exact hits suppressing fuzzy noise
- Deterministic Subgraph Root Resolution (v1.7.2): `resolve_subgraph_root` with typed root-kind precedence; `get_entity_subgraph` anchors on the resolved UUID and discloses `root_resolution`
- Cross-Repo Dependency Linking (v1.2.5) with auto-discovered DEPENDS_ON edges (Maven/Gradle/Cargo/npm/NuGet) and retroactive linking
- Entity Subgraph Traversal (v1.3.2) + Kind-Aware Filtering (v1.3.7) + Connectivity Fix (v1.3.8)
- Standalone CLI Tool (`knot`) with full MCP parity
- Colorized table output, interactive pager, configurable output formats (table/json/markdown)
- Custom CA certificates support for corporate network downloads
- O(N) nested macro traversal optimization for large Rust codebases
- Consolidated `.knot/` directory: fastembed model cache now stored in `.knot/fastembed_cache/` (configurable via `KNOT_FASTEMBED_CACHE_DIR`)
- 1239 unit tests | 100+ E2E assertions across 21 suites (`tests/run_all_e2e_fast.sh`)

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

## Phase 13: Markdown Documentation Indexing (v1.4.9 — ✅ Completed)

### Objective
Enable `knot` to index Markdown (`.md`/`.markdown`) documentation files so that documentation content — not just heading titles — is searchable semantically, with hierarchical FQN resolution that disambiguates same-named sections across files and nesting levels.

### Planned → ✅ Completed
- ✅ `tree-sitter-md = "0.5.3"` integration via the unified tree-sitter parser dispatcher (`md`/`markdown` extension)
- ✅ 2 new EntityKind variants: `MarkdownDocument` (one per file) and `MarkdownSection` (one per ATX heading H1–H6)
- ✅ `queries/markdown.scm` tree-sitter query with two captures: `@markdown.document.name` (document root) and `@markdown.section` (each section spans its heading + body)
- ✅ `src/pipeline/parser/languages/markdown.rs` (524 lines) — `handle_markdown_capture`, `build_markdown_fqn` (ancestor-section chain), `clean_heading_name` (strip inline syntax), `extract_document_intro`, `extract_section_body`
- ✅ Hierarchical, file-scoped FQNs (`README.md::Setup > Installation > Linux`) so same-named headings in different files or under different parents disambiguate cleanly
- ✅ Section bodies — paragraphs, fenced code blocks, lists, and tables — captured into `embed_text` so semantic search returns documentation prose, not only heading text
- ✅ Section boundaries respect heading depth: a section extends until the next same-or-higher-level heading (prevents `### Linux` under `## Installation` from bleeding into a sibling `## Configuration`)
- ✅ Headings with inline markdown (backticks, em-dash, links, emoji, escaped chars) parse without losing their bodies or breaking FQN construction
- ✅ Real `start_line` / `end_line` positions computed via tree-sitter for each section (no heuristic line counting)
- ✅ Wired into the parser dispatcher (`src/pipeline/parser/mod.rs`), capture router (`extractor/captures.rs`), enricher (`extractor/enrich.rs`), and graph utils (`db/graph/utils.rs`)
- ✅ 13+ inline unit tests in `markdown.rs` covering AST shape, FQN construction across nesting depths, section body extraction, and special-character headings
- ✅ E2E suite `tests/run_markdown_e2e.sh` (366 lines): body searchability, cross-file disambiguation, deep nesting, special-character headings, document-level intro capture
- ✅ Wired into `tests/run_all_e2e_fast.sh` (line 58) so Markdown runs in the master suite
- ✅ `MarkdownDocument` / `MarkdownSection` handled in `prepare.rs` skip-list and `context.rs` for clean FQN derivation

#### Implementation Files
- `src/pipeline/parser/languages/markdown.rs` — main Markdown parser module (524 lines)
- `src/pipeline/parser/languages/mod.rs` — registers the module (`pub mod markdown;`)
- `src/pipeline/parser/mod.rs` — `md`/`markdown` extension dispatch with `DEFAULT_MD_QUERY` include
- `src/pipeline/parser/extractor/captures.rs` — routes `markdown.*` capture names
- `src/pipeline/parser/extractor/enrich.rs` — FQN + body text enrichment for Markdown
- `src/pipeline/parser/extractor/tests.rs` — unit tests covering MarkdownDocument / MarkdownSection
- `src/pipeline/parser/test_utils.rs` — `parse_markdown_snippet` helper
- `src/pipeline/parser/languages/markdown.rs` (tests module) — inline unit tests for FQN, body extraction, AST walking
- `src/pipeline/prepare.rs` — Markdown entity skip handling
- `src/pipeline/parser/context.rs` — file-scoped FQN derivation
- `src/db/graph/utils.rs` — display labels for `MarkdownDocument` / `MarkdownSection`
- `src/models/entity.rs` — `EntityKind::MarkdownDocument`, `EntityKind::MarkdownSection` enum variants and string mappings
- `queries/markdown.scm` — tree-sitter query file (26 lines, two captures)
- `tests/run_markdown_e2e.sh` — Markdown E2E suite (366 lines)
- `tests/run_all_e2e_fast.sh` — registers the Markdown suite
- `tests/testing_files/markdown/` — fixtures: `README.md`, `GUIDE.md`, `complex.md`, `nested.md`
- `README.md` — documents Markdown language support (line 71)

#### Design Notes
- The `section` node emitted by `tree-sitter-md` already spans from an ATX heading down to the next same-or-higher-level heading, so capturing `section` directly gives both the right `end_line` and the embedded body text without parent-walking or level-tracking in the handler.
- FQNs normalize the same heading text used for the entity `name` field (via `clean_heading_name`) so `## [foo](bar.md)` yields `name = "foo bar.md"` and `fqn = "...::foo bar.md"` consistently — otherwise queries that match by name would miss by FQN.
- `extract_document_intro` returns only the text before the first heading for `MarkdownDocument`, so the document-level embed text captures the file's preamble (frontmatter-style intro) rather than duplicating the first section's body.

#### Limitations (intentional, documented in code)
- Inline link text vs. URL distinction is not preserved in `name` (`## [foo](bar.md)` → `foo bar.md`).
- Reference-style links (`[text][ref]`) are not specially resolved.
- HTML tags inside headings are not stripped.
- Escaped characters (`\*`, `\_`) are not decoded.

---

## Phase 14: Varnish Cache Language Support (v1.5.7 / v1.6.1 — ✅ Completed)

### Objective
Enable `knot` to index Varnish Cache deployments end-to-end: `.vcl` configuration, `.vtc` test cases, and `.vcc` VMOD interface definitions — with a relationship model rich enough to answer operational questions ("which subroutine routes to this backend?", "which file includes this one?", "which VMOD does this config import?").

Varnish is the first language in `knot` supported **without a tree-sitter grammar**: no maintained grammar exists for VCL, so all three formats are decoded by a single hand-rolled lexer plus per-dialect parsers.

### Planned → ✅ Completed
- ✅ Hand-written lexer (`languages/varnish/lexer.rs`) covering VCL's 15 documented gotchas: duration maximal-munch (`10s` vs `10` `s`), adjacent string concatenation, ACL mask literals (`"192.0.2.0"/24`), identifier hyphens, `${...}` macro tokens, version markers, dotted paths, quoted header names, and all comment forms (`#`, `//`, `/* */`)
- ✅ Dialect guard (`languages/varnish/dialect.rs`): the Fastly VCL dialect is detected and skipped (returns empty entities, logs at `debug`) rather than mis-parsed
- ✅ **8 VCL EntityKinds**: `VclVersion`, `VclSubroutine`, `VclBuiltinSub`, `VclBackend`, `VclProbe`, `VclAcl`, `VclImport`, `VclObjectInstance`
- ✅ **6 VTC EntityKinds**: `VtcTestCase`, `VtcServer`, `VtcClient`, `VtcVarnishInstance`, `VtcLogexpect`, `VtcBarrier`
- ✅ **4 VCC EntityKinds**: `VccModule`, `VccFunction`, `VccObject`, `VccMethod` (methods bound to their owning object via `enclosing_class`)
- ✅ **7 new `ReferenceIntent` variants**: `VclSubCall`, `VclBackendRef`, `VclProbeRef`, `VclAclRef`, `VclInclude`, `VclVmodImport`, `VclUnusedRef`
- ✅ **6 new `RelationshipType` variants** with directed `Display` forms: `USES_BACKEND`, `USES_PROBE`, `USES_ACL`, `INCLUDES`, `IMPORTS_VMOD`, `DECLARED_UNUSED`
- ✅ Three-way exhaustive match enforced across `models/entity.rs` (`Display`), `db/graph/utils.rs` (`kind_to_label`), and `pipeline/parser/context.rs` (`compute_fqn_and_context`)
- ✅ Recursive scanning of `if`/`elseif`/`else` bodies, so `set req.backend_hint = X;` inside a conditional still emits a `USES_BACKEND` edge
- ✅ Multi-part built-in subs: `sub vcl_recv { }` declared across several files aggregates into one `vcl_recv_aggregator` entity, emitted globally in `parse_files_stream` via `aggregate_varnish_builtin_subs` (shared `Arc<Mutex<Vec<ParsedEntity>>>` buffer wired through `parser/mod.rs`)
- ✅ VTC embedded VCL: `varnish vX { … }` blocks are delegated to the VCL parser with line offsets so cross-references resolve to the right lines; `-errvcl` blocks are skipped; `-vcl+backend` synthesises `vcl_backend` entities per `server` with `is_test_context = true`
- ✅ `explore_file` fallback `## Other Entities` bucket so all 18 Varnish kinds stay visible to LLMs (they previously fell through `_ => {}` and were silently dropped)
- ✅ **68 unit tests** in `pipeline::parser::languages::varnish`
- ✅ **E2E suite** `tests/run_varnish_e2e.sh` (~24 Cypher assertions) — entity counts, all 6 relationship types, VCL/VTC/VCC extraction, multi-part sub aggregation, Fastly suppression, unique-token semantic search, `explore_file` listing. Registered as the 19th suite in `tests/run_all_e2e_fast.sh`
- ✅ **v1.6.1 follow-up — include resolution**: `include` directives with absolute paths (`include "/etc/varnish/language.vcl";`) failed to map to their target files because the parser pre-formatted the path into an FQN. Fixed by storing the raw path and resolving with a three-strategy fallback (repo-root, relative-to-current-file, filename fuzzy match). Spec: `docs/specs/varnish_include_resolution_plan.md`

#### Implementation Files
- `src/pipeline/parser/languages/varnish/lexer.rs` — shared hand-rolled lexer for all three formats
- `src/pipeline/parser/languages/varnish/dialect.rs` — Fastly dialect detection guard
- `src/pipeline/parser/languages/varnish/vcl.rs` — `.vcl` configuration parser
- `src/pipeline/parser/languages/varnish/vtc.rs` — `.vtc` test-case parser with embedded-VCL delegation
- `src/pipeline/parser/languages/varnish/vcc.rs` — `.vcc` VMOD interface parser
- `src/pipeline/parser/languages/varnish/mod.rs` — module wiring + `aggregate_varnish_builtin_subs`
- `src/pipeline/input.rs` — `vcl`, `vtc`, `vcc` registered in `CORE_EXTENSIONS` / `SUPPORTED_EXTENSIONS`
- `src/pipeline/ingest/resolve/mod.rs` — `VclInclude` multi-strategy resolution (v1.6.1)
- `tests/run_varnish_e2e.sh` — Varnish E2E suite; the reference template for `KNOT_SKIP_BUILD`-aware suites
- `tests/testing_files/varnish/` — 12 fixtures: `default.vcl`, `backends.vcl`, `edge_cases.vcl`, `inline_probe.vcl`, `multi_recv_a.vcl`, `multi_recv_b.vcl`, `fastly_sample.vcl`, `etc/varnish/language.vcl`, `basic_hit.vtc`, `errvcl.vtc`, `external_ref.vtc`, `vmod_cookie.vcc`

#### Design Notes
- No tree-sitter grammar exists for VCL, and the three formats share enough lexical structure (identifiers, durations, strings, comments) that one lexer serving three parsers was cheaper than three independent parsers.
- Built-in sub aggregation had to move *out* of `parse_single_file` and into the streaming orchestrator: a `vcl_recv` split across files cannot be aggregated from inside a single-file parse, so `parse_files_stream` collects them repo-wide.
- `tests/run_varnish_e2e.sh` is the only per-language suite that honours `KNOT_SKIP_BUILD`, which is how CI invokes the orchestrator. It is therefore the correct template for any new language suite — `run_kotlin_e2e.sh` and `run_rust_e2e.sh` ignore the flag and pass in CI only incidentally.

---

## Phase 15: C# Support (v1.7.0 — ✅ Completed)

### Objective
Enable `knot` to index C#/.NET codebases with the same fidelity already provided for Java and Kotlin: namespace-qualified FQNs, full type/member extraction, `CALLS` / `EXTENDS` / `IMPLEMENTS` / `REFERENCES` / `CONTAINS` / `OVERRIDES` edges, XML doc comments, and attributes — validated end-to-end through both the MCP server and the CLI.

**Closes** [issue #5](https://github.com/raultov/knot/issues/5).

**Full plan:** [`docs/specs/csharp_support_plan.md`](csharp_support_plan.md) — grammar evaluation, design decisions, integration map, 7 implementation phases, 31 E2E assertions.

### Grammar
`tree-sitter-c-sharp = "0.23.5"` (official `tree-sitter` org, MIT). Verified compatible: grammar ABI 15 is within the range accepted by `tree-sitter 0.26.8` (`LANGUAGE_VERSION_WITH_RESERVED_WORDS 15`), and `tree-sitter-language 0.1.7` is already in `Cargo.lock` — **zero new transitive dependencies**.

### Delivered
- [x] **16 new `CSharp*` EntityKinds**: `CSharpClass`, `CSharpInterface`, `CSharpStruct`, `CSharpRecord`, `CSharpEnum`, `CSharpMethod`, `CSharpConstructor`, `CSharpProperty`, `CSharpField`, `CSharpConstant`, `CSharpDelegate`, `CSharpEvent`, `CSharpIndexer`, `CSharpOperator`, `CSharpNamespace`, `CSharpLocalFunction`
- [x] `queries/csharp.scm` with `csharp.`-prefixed captures, routed through the delegating arm in `extractor/captures.rs` (Rust/Groovy pattern)
- [x] `src/pipeline/parser/languages/csharp/` as a directory (`mod.rs`, `capture.rs`, `fqn.rs`, `refs.rs`, `tests.rs`), following the `rust/` and `varnish/` layout
- [x] **Hybrid FQN resolution** — file-scoped namespaces (C# 10 `namespace X;`) have no `body` in the grammar, so the types that follow are *siblings* under `compilation_unit`, unreachable by a parent walk. Implemented as a file-level pre-pass (`csharp::extract_file_scoped_namespace`, Java `extract_package_name` model) **plus** an ancestor walk for block-form namespaces and nested types (`csharp::build_csharp_fqn_prefix`, C++ `build_cpp_fqn` model). Neither model alone is sufficient
- [x] **`base_list` disambiguation heuristic** — C# has no syntactic distinction between inheritance and implementation (`class Foo : Bar, IBaz` is one `base_list`). Resolved by declarer kind plus the `I`-prefix convention: interfaces always `EXTENDS`, structs always `IMPLEMENTS`, classes take the first entry as `EXTENDS` unless it matches `^I[A-Z]`
- [x] Reference extraction: `invocation_expression`, `member_access_expression`, `object_creation_expression` (redirected to constructors), `attribute`, type positions, `using_directive`. Field-typed receivers (`_repository.FindByIdAsync()`) are substituted with the field's declared type so calls resolve to the exact implementation method; `base.Method()` maps to the resolver's `super` receiver
- [x] **`OVERRIDES` extended beyond the JVM** — `resolve/overrides.rs` gained `.cs` and the C# kinds; its `JVM_*` vocabulary was renamed to `OVERRIDE_CAPABLE_*` so the module stops misdescribing its own scope
- [x] XML doc comments (`/// <summary>`) — no new code needed; `strip_comment_markers` already handles `///`
- [x] 42 unit tests + `tests/run_csharp_e2e.sh` (registered as the 20th suite) with dual MCP + CLI validation across 7 fixtures
- [x] `CURRENT_STATE_VERSION` deliberately **not** bumped — C# adds new kinds without changing existing FQN shapes, so no one is forced into a full re-index

### Out of Scope (follow-up issues)
- ~~`.csproj` / NuGet cross-repo linking~~ — ✅ delivered in v1.7.2 (extension-based discovery, `Directory.Packages.props` Central Package Management, `build_system: "nuget"`, cross-repo `DEPENDS_ON` edges); `.sln` solution-level identity remains open
- `partial class` / `partial method` unification across files
- Resolution-time EXTENDS/IMPLEMENTS correction (replacing the naming heuristic)
- Generic constraints (`where T : IComparable<T>`) as `GENERIC_BOUND` edges

---

## Phase 16: Repository Scope Selection (v1.8.0 — ✅ Completed)

### Objective
Enable `knot` to target multiple repositories in a single query across `search_hybrid_context`, `find_callers`, and `explore_file` MCP tools and CLI commands (`knot search`, `knot callers`, `knot explore`), allowing agents and users to specify a single repository, a union of repositories, or target all indexed repositories.

**Closes** [issue #19](https://github.com/anomalyco/knot/issues/19).

### Delivered
- [x] **Repository Scope Model**: Added `RepoScope` enum (`All`, `One(String)`, `Many(Vec<String>)`) to parse single repo names, comma-separated lists (`"repo-a,repo-b"`), sentinels (`all`, `*`), and JSON string arrays in MCP `repo_name`.
- [x] **Unified Tool Parsing**: MCP tools (`search_hybrid_context`, `find_callers`, `explore_file`) and CLI flags (`--repo/-r`) support multi-repo union scope filtering across vector (Qdrant) and graph (Neo4j) queries.
- [x] **Sentinel Priority & Sanitization**: Sentinels (`all`/`*`) override individual tokens in the same scope specification; whitespace trimmed, empty tokens dropped, case-insensitive matching for sentinels, duplicates collapsed. Unknown repo names yield silent no-rows without errors.
- [x] **Global Result Limit Handling**: `max_results` applied globally across the scope union.
- [x] **E2E Validation**: Added `tests/run_repo_scope_e2e.sh` registered as the 21st suite in `tests/run_all_e2e_fast.sh`.

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
| Phase 13: Markdown Documentation Indexing | Low | ✅ Completed (v1.4.9) |
| Phase 14: Varnish Cache (VCL/VTC/VCC) | High | ✅ Completed (v1.5.7, include resolution v1.6.1) |
| Phase 15: C# Support | High | ✅ Completed (v1.7.0) |
| Phase 16: Repository Scope Selection | Medium | ✅ Completed (v1.8.0) |

---

## Backward Compatibility

- All new language phases are backward compatible
- No database migration needed: new entity types added dynamically
- MCP tools and CLI work seamlessly with existing indexed data

---

## Changelog

### v1.8.0 - Repository Scope Selection
- ✅ **Feat(scope)**: Multi-repository scope selection for `search_hybrid_context`, `find_callers`, `explore_file` and CLI commands `--repo/-r`.
- ✅ **Feat(scope)**: Supports single repository name, comma-separated list (`"repo-a,repo-b"`), sentinel `all` or `*` (targets every indexed repository), and JSON string array (`["repo-a", "repo-b"]` in MCP).
- ✅ **Feat(scope)**: Sentinel priority (`all`/`*` overrides list tokens), whitespace trimming, case-insensitive sentinel matching, and silent empty results for non-existent repositories.
- ✅ **Test(e2e)**: New `tests/run_repo_scope_e2e.sh` (21st test suite in `tests/run_all_e2e_fast.sh`).
- ✅ **Docs**: Updated `README.md`, `.prompt`, `.knot-agent.md`, `src/mcp_handler.rs`, and roadmap specs.
- ✅ Credit: closes #19.

### v1.7.2 - Deterministic Subgraph Root Resolution & MSBuild/NuGet Build-System Support
- ✅ **Feat(query)**: `resolve_subgraph_root` with `root_kind_rank` precedence — `get_entity_subgraph` anchors on the resolved UUID (eliminating homonym-union results) and discloses `root_resolution` (tier, candidates).
- ✅ **Feat(parser)**: MSBuild `.csproj` + `Directory.Packages.props` as a first-class build system — `build_system: "nuget"`, Central Package Management version resolution, NuGet `DEPENDS_ON` cross-repo edges.

### v1.7.1 - find_callers Target Resolution & Substring Noise Reduction
- ✅ **Feat(query)**: Two-stage resolution ladder (exact FQN → FQN suffix → exact name → signature prefix → fuzzy substring); exact hits now suppress fuzzy noise.
- ✅ **Feat(db)**: Neo4j `entity_name_text` / `entity_fqn_text` TEXT indexes; resolution metadata surfaced in CLI and markdown formatters.

### v1.7.0 - C# Language Support
- ✅ **Feat(parser)**: Full C# extraction (16 `CSharp*` entity kinds), namespace-qualified FQNs, `base_list` disambiguation heuristic, `OVERRIDES` beyond the JVM, XML doc comments, and attributes. 42 unit tests + 20th E2E suite. Closes #5.

### v1.6.2 - Accurate Indexing Progress
- ✅ **Fix(progress)**: Weighted progress bands across the whole pipeline (parse → embed/ingest → resolve); `IndexingProgress.total_entities`; `ParseCallbacks` replaces `FileParsedCallback`.

### v1.6.1 - Varnish VCL Include Resolution
- ✅ **Fix(varnish)**: Absolute-path `include` directives resolve via a three-strategy fallback (repo-root, relative, filename fuzzy match) to build `INCLUDES` relationships.

### v1.6.0 - Unsafe Elimination & Code Quality Enforcement
- ✅ **Refactor(unsafe)**: 17 of 18 `unsafe` blocks eliminated; `unsafe_code = "deny"` at crate level; all bare `#[allow(...)]` converted to `#[expect(..., reason = "...")]`; clippy readability lints (`too_many_lines`, `cognitive_complexity`, ...) wired in `clippy.toml`.

### v1.5.7 - Varnish Cache Language Support
- ✅ **Feat(parser)**: `.vcl` / `.vtc` / `.vcc` support via a hand-written lexer (no tree-sitter grammar exists) — 18 entity kinds, 6 relationship types, built-in sub aggregation; 19th E2E suite.

### v1.4.9 - Markdown Documentation Indexing
- ✅ **Feat(parser)**: Added Markdown support (`.md`/`.markdown`) with `MarkdownDocument` (one per file) and `MarkdownSection` (one per ATX heading H1–H6).
- ✅ **Feat(parser)**: Section bodies — paragraphs, fenced code blocks, lists, tables — captured into `embed_text` for full semantic search over documentation content, not just heading titles.
- ✅ **Feat(parser)**: Hierarchical, file-scoped FQNs (e.g. `README.md::Setup > Installation > Linux`) prevent cross-file and within-file heading collisions.
- ✅ **Feat(parser)**: Section boundaries respect heading depth (`section` spans until the next same-or-higher-level heading).
- ✅ **Feat(parser)**: Headings with inline markdown (backticks, em-dash, links, emoji) parse without losing their bodies.
- ✅ **Test(unit)**: 13+ inline tests in `src/pipeline/parser/languages/markdown.rs` covering AST shape, FQN construction across nesting depths, body extraction, and special-character headings.
- ✅ **Test(e2e)**: New `tests/run_markdown_e2e.sh` (366 lines) — body searchability, cross-file disambiguation, deep nesting, special-character headings, document-level intro capture.
- ✅ **Test(e2e)**: Wired into `tests/run_all_e2e_fast.sh` so Markdown runs in the master suite.
- ✅ **Docs(readme)**: Documented Markdown language support.
- ✅ **cargo fmt** clean | **cargo clippy** clean.
- ✅ Credit: @sdi2200246 (PR #17, closes #8).

### v1.3.8 - Subgraph Connectivity & Edge Extraction Fix
- ✅ **Fixed Subgraph Disconnection**: Injects `CONTAINS` relationships into traversal paths when kind-filtering is active, keeping class-to-class paths discoverable.
- ✅ **UUID List Binding Fix**: Replaced `$uuids` parameter with string literal interpolation to fix missing edges.
- ✅ **Constrained Relationship Result**: Constrained direct edges to requested types.
- ✅ **565 unit tests** passing | 12/12 E2E suites passing.

### v1.3.7 - Kind-Aware Subgraph Traversal
- ✅ **Kind-Aware Traversal**: `get_entity_subgraph` now accepts a `visible_kinds` parameter.
- ✅ **Synthetic Edge Roll-up**: Implemented Cypher logic to automatically create synthetic edges between visible nodes connected through hidden intermediaries (e.g., methods/functions).
- ✅ **Connectivity Preservation**: Focusing on classes or interfaces no longer results in disconnected subgraphs when they are linked via method calls.
- ✅ **Integration Testing**: Added `test_get_entity_subgraph_with_visible_kinds` to verify kind-based filtering.
- ✅ **565 unit tests** passing | 12/12 E2E suites passing.

### v1.3.6 - Java Indexing Enhancement (Package & Inheritance)
- ✅ **Package-aware FQN Resolution**: Java entities now include the full package prefix (e.g., `com.example.app.UserService`).
- ✅ **Inheritance Extraction**: Added support for `EXTENDS` (classes/interfaces) and `IMPLEMENTS` (classes) relationships.
- ✅ **Generic Stripping**: Strips generic parameters from base types for cleaner relationship linking (e.g., `Repository<User>` → `Repository`).
- ✅ **Anonymous Class Support**: Tracks method invocations within anonymous inner classes.
- ✅ **565 unit tests** passing | 12/12 E2E suites passing.

### v1.3.5 - E2E Test Stabilization & Async Resilience
- ✅ **Stabilized Groovy E2E Tests**: Introduced `retry_match` helper to handle Qdrant's eventual consistency without performance-killing `.wait(true)` calls.
- ✅ **Fixed C/C++ E2E Test Flakiness**: Fixed non-deterministic relationship resolution in Test 14 by relaxing file path constraints and using FQN matching.
- ✅ **Improved Docker Resilience**: Switched to static container names (`knot_neo4j_cpp_e2e`) to prevent naming conflicts and syntax errors in `run_all_e2e.sh`.
- ✅ **Deterministic CI**: Added stabilization delays and retry loops for all semantic and graph queries in E2E suites.
- ✅ **12/12 E2E test suites pass** consistently in full suite runs.

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
