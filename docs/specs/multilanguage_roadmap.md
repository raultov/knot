# Multi-Language Roadmap for Knot

This document outlines the planned expansion of `knot` to support Python and C/C++ codebases, building on the existing foundation for Java, TypeScript, JavaScript, Kotlin, Rust, HTML, CSS, and SCSS.

---

## Overview

**Current State (v0.10.1):**
- Java, Kotlin, TypeScript/TSX, JavaScript/Node.js, Rust, Python, Groovy, HTML, CSS, SCSS support
- Typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
- Build Systems: Maven (pom.xml), Gradle (build.gradle), Jenkinsfile extraction
- Groovy: standard .groovy files via tree-sitter-groovy (classes, interfaces, enums, methods, fields)
- Dual-database architecture (Qdrant + Neo4j)
- Three MCP tools (search_hybrid_context, find_callers, explore_file)
- Standalone CLI Tool (`knot`) with full MCP parity
- Colorized table output, interactive pager, configurable output formats (table/json/markdown)
- Custom CA certificates support for corporate network downloads
- O(N) nested macro traversal optimization for large Rust codebases
- 426 unit tests | 74+ E2E tests across all languages

**Goal:** Extend `knot` to become the standard indexer for hybrid web projects with full cross-language dependency resolution and support for Python and C/C++.

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

## Phase 11: C/C++ Support (v0.11.x)

### Objective
Enable `knot` to index C and C++ codebases, focusing on pointer relationships, header inclusion graphs, and macro analysis.

#### Planned
- tree-sitter-cpp integration
- Header inclusion graph construction
- Macro call tracking
- Pointer/reference relationship analysis

---

## Implementation Priority & Timeline

| Phase | Complexity | Status |
|-------|-----------|--------|
| Phase 1-6: JS/HTML/CSS/Kotlin/CLI | - | ✅ Completed |
| Phase 7: Rust | High | ✅ Completed (v0.8.11) |
| Phase 8: Python | High | ✅ Completed (v0.9.3) |
| Phase 9: Build Systems (Maven/Gradle/Jenkins) | Medium | ✅ Completed (v0.10.0) |
| Phase 10: Groovy Language Support | Medium | ✅ Completed (v0.10.1) |
| Phase 11: C/C++ | High | 📋 Planned (v0.11.x) |

---

## Backward Compatibility

- All new language phases are backward compatible
- No database migration needed: new entity types added dynamically
- MCP tools and CLI work seamlessly with existing indexed data

---

## Changelog

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
