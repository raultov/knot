# Multi-Language Roadmap for Knot

This document outlines the planned expansion of `knot` to support Python and C/C++ codebases, building on the existing foundation for Java, TypeScript, JavaScript, Kotlin, Rust, HTML, CSS, and SCSS.

---

## Overview

**Current State (v0.8.11):**
- Java, Kotlin, TypeScript/TSX, JavaScript/Node.js, Rust, HTML, CSS, SCSS support
- Typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
- Dual-database architecture (Qdrant + Neo4j)
- Three MCP tools (search_hybrid_context, find_callers, explore_file)
- Standalone CLI Tool (`knot`) with full MCP parity
- Colorized table output, interactive pager, configurable output formats (table/json/markdown)
- Custom CA certificates support for corporate network downloads
- O(N) nested macro traversal optimization for large Rust codebases

**Goal:** Extend `knot` to become the standard indexer for hybrid web projects with full cross-language dependency resolution and support for Python and C/C++.

---

## Phase 8: Python Support (v0.9.x)

### Objective
Enable `knot` to index Python codebases with full semantic understanding of AST, classes, decorators, and module dependencies.

#### Planned
- tree-sitter-python integration
- Class, function, method, and decorator extraction
- Import resolution and cross-module dependency graph
- Type annotation support (PEP 484)

---

## Phase 9: C/C++ Support (v0.10.x)

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
| Phase 8: Python | High | 📋 Planned (v0.9.x) |
| Phase 9: C/C++ | High | 📋 Planned (v0.10.x) |

---

## Backward Compatibility

- All new language phases are backward compatible
- No database migration needed: new entity types added dynamically
- MCP tools and CLI work seamlessly with existing indexed data

---

## Changelog

### v0.8.11 - Rust Performance Optimization (O(N) Macro Traversal)
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
