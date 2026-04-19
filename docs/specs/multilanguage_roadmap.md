# 🗺️ Multi-Language Roadmap for Knot

This document outlines the phased expansion of `knot` from a Java/TypeScript indexer to a comprehensive codebase graph indexer supporting JavaScript, HTML, CSS/SCSS, and eventually Rust. Each phase builds upon the previous, enabling increasingly sophisticated cross-language code understanding.

---

## Overview

**Current State (v0.8.1):**
- ✅ Java support (full AST extraction)
- ✅ Kotlin support (v0.7.4+) - Complete with classes, interfaces, objects, functions, methods, properties
- ✅ TypeScript/TSX/CTS support (modern JavaScript/TypeScript)
- ✅ JavaScript/Node.js support (`.js`, `.mjs`, `.cjs`, `.jsx`)
- ✅ HTML support (`.html`, `.htm` with custom elements, id/class indexing)
- ✅ CSS support (`.css` with class/ID selector extraction)
- ✅ SCSS support (`.scss`, `.sass` with mixins, functions, variables)
- ✅ Typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
- ✅ Dual-database architecture (Qdrant + Neo4j)
- ✅ Three MCP tools (search_hybrid_context, find_callers, explore_file)
- ✅ Standalone CLI Tool (`knot`) with full MCP parity (v0.8.0+)
- ✅ Cross-language linking (JS/HTML/CSS)

**Goal:** Extend `knot` to become the standard indexer for hybrid web projects (JS/HTML/CSS) with full cross-language dependency resolution and support for Kotlin, CLI tools, Rust, and C/C++.

---

## Phase 5: Kotlin Support (v0.7.0) ✅ COMPLETED

### Objective
Enable `knot` to index Kotlin codebases, providing full AST extraction, semantic understanding of Kotlin-specific constructs (data classes, companion objects, extension functions), and cross-language linking (e.g., with Java).

### Implementation Status

#### ✅ Completed Features
- **File Extensions**: Full support for `.kt` and `.kts` files
- **Entity Types**: All 7 Kotlin entity types fully extracted:
  - KotlinClass
  - KotlinInterface
  - KotlinObject (singleton objects)
  - KotlinCompanionObject
  - KotlinFunction (Top-level and extension functions)
  - KotlinMethod (Class methods)
  - KotlinProperty (val/var declarations)

#### ✅ Dependencies
- tree-sitter-kotlin-ng = "1.1.0" (in Cargo.toml)

#### ✅ Relationship Tracking
- **EXTENDS/IMPLEMENTS**: Track class inheritance and interface implementation
- **CALLS**: Function and method invocations
- **REFERENCES**: Type usage in signatures and generics
- **ANNOTATES**: Capture annotations for Spring Boot/Android frameworks

#### ✅ Special Handling
- **Extension Functions**: Properly link `fun String.myExtension()` as callable methods on receiver type
- **Companion Objects**: Map `companion object` methods as static-like calls to parent class
- **Data Classes**: Auto-infer properties from primary constructor

#### ✅ Validation Completed
- ✅ 10 comprehensive E2E integration tests - ALL PASSING
- ✅ Extract top-level functions and extension functions accurately
- ✅ Correctly map companion object methods to parent class
- ✅ Track dependencies between Kotlin and Java files in mixed projects
- ✅ Full tree-sitter-kotlin-ng v1.1.0 grammar compatibility

---

## Phase 6: CLI Interface & Agent Skill (v0.8.0) ✅ COMPLETED

### Objective
Create a standalone CLI binary named `knot` that exposes the exact same functionality as `knot-mcp`. This will allow both humans and AI agents (via standard terminal execution) to query the index without needing an MCP client.

### Implementation Status

#### ✅ Completed Features
- **CLI Binary** (`src/bin/knot.rs`): Standalone `knot` command with three subcommands
- **Command Parity**: 
  - `knot search "query"` → equivalent to `search_hybrid_context`
  - `knot callers "EntityName"` → equivalent to `find_callers`
  - `knot explore "/path/to/file"` → equivalent to `explore_file`
- **Shared Core Logic** (`src/cli_tools/`): Both CLI and MCP use identical business logic
  - `run_search_hybrid_context()` — semantic + structural search
  - `run_find_callers()` — reverse dependency lookup
  - `run_explore_file()` — file anatomy inspection
- **Agent Skill File** (`.knot-agent.md`): 4000+ line comprehensive guide teaching LLMs:
  - How to use each command with examples
  - Output interpretation guide
  - Workflow patterns (feature discovery, impact analysis, dead code detection)
  - Integration with AI agent systems
- **Full Documentation**: CLI help, examples for Java/TypeScript/Kotlin
- **CLI Argument Parsing**: Using `clap` with:
  - `--repo` filter for multi-repo environments
  - `--max-results` for search result limiting
  - Consistent error messages

#### ✅ Architecture Benefits
- **No Duplication**: CLI and MCP share `src/cli_tools/` core
- **Parallel Evolution**: Both interfaces evolve together as features are added
- **Flexible Deployment**: Use MCP in IDEs, CLI in terminals and CI/CD
- **LLM-Friendly**: `.knot-agent.md` skill enables autonomous code analysis via bash execution

#### ✅ Validation Completed
- All CLI unit tests pass
- CLI parser tests cover all command variants
- Output format matches MCP for consistency
- Integration with existing database connections verified

---

## Phase 7: Rust Support (v0.8.x)

### Objective
Enable `knot` to index Rust codebases with full semantic understanding of ownership, traits, and macro expansion.
*(Implementation details to be defined closer to release).*

---

## Phase 8: C/C++ Support (v0.9.x)

### Objective
Enable `knot` to index C and C++ codebases, focusing on pointer relationships, header inclusion graphs, and macro analysis.
*(Planned for future expansion).*

---

## Implementation Priority & Timeline

| Phase | Complexity | Status |
|-------|-----------|--------|
| Phase 1: JavaScript | Low | ✅ Completed (v0.6.1) |
| Phase 2: HTML | Low | ✅ Completed (v0.6.3) |
| Phase 3: CSS/SCSS | Medium | ✅ Completed (v0.6.4) |
| Phase 4: Web Ecosystem | High | ✅ Completed (v0.6.5) |
| Phase 5: Kotlin | Medium | ✅ Completed (v0.7.4) |
| Phase 6: CLI | Low | ✅ Completed (v0.8.0) |
| Phase 7: Rust | High | 📋 Planned (v0.8.x) |
| Phase 8: C/C++ | High | 📋 Planned (v0.9.x) |

---

## Breaking Changes & Backward Compatibility

### Phases 1-5 (File Format & Schema)
- ✅ **Backward compatible**: New languages integrate into existing Neo4j/Qdrant structure
- ✅ **No database migration needed**: Add new entity types dynamically
- ✅ **MCP tools unchanged**: Work seamlessly with new entity kinds

### Phase 4 (Cross-Language Relationships)
- ⚠️ **New relationship types**: `REFERENCES_DOM`, `USES_CSS_CLASS`, `IMPORTS_SCRIPT`, `IMPORTS_STYLESHEET`
- ✅ **Optional**: Existing queries continue to work
- ✅ **Gradual rollout**: Enable per-project basis

### Phase 7 & 8 (Rust & C/C++)
- ✅ **Backward compatible**: Additional languages, not a replacement

---

## Contributing & Future Enhancements

### Community Contributions
Contributions in any phase are welcome! Each phase is designed to be modular and independently valuable.

---

## References

- [Tree-sitter Language Support](https://tree-sitter.github.io/tree-sitter/#language-bindings)
- [Tree-sitter JavaScript Grammar](https://github.com/tree-sitter/tree-sitter-javascript)
- [Tree-sitter HTML Grammar](https://github.com/tree-sitter/tree-sitter-html)
- [Tree-sitter CSS Grammar](https://github.com/tree-sitter/tree-sitter-css)
- [Tree-sitter SCSS Grammar](https://github.com/tree-sitter/tree-sitter-scss)
- [Tree-sitter Kotlin Grammar (kotlin-ng)](https://github.com/fwcd/tree-sitter-kotlin)
- [Tree-sitter Rust Grammar](https://github.com/tree-sitter/tree-sitter-rust)

## Changelog

### v0.8.1 - CLI UX Improvements and E2E Coverage
- ✅ **Silenced CLI Logs**: Default log level set to `error` for `knot` CLI to eliminate onnxruntime/fastembed noise in stdout.
- ✅ **Stderr Logging**: All CLI logs now redirected to stderr, keeping stdout clean for Markdown results.
- ✅ **100% Dual Testing Coverage**: Updated all 35 E2E tests (25 main + 10 Kotlin) to simultaneously verify both `knot-mcp` and `knot` CLI.
- ✅ **Docker Integration**: Added `knot` CLI binary to the official Docker image.
- ✅ **Improved Agent Guidance**: Updated `.knot-agent.md` with critical warnings about searching ubiquitous method names (accept, process, etc.) by signature.

### v0.8.0 - CLI Interface
- ✅ **Standalone CLI**: Created `knot` binary with `search`, `callers`, and `explore` commands.
- ✅ **Unified Core**: Extracted shared logic into `cli_tools` module used by both CLI and MCP.
- ✅ **Auto Repo Detection**: CLI automatically detects repository name from the current directory.
- ✅ **LLM Skill File**: Introduced `.knot-agent.md` to teach AI agents how to query the index via CLI.

### v0.7.4 - Enhanced Search Precision
- ✅ **Signature-based Search**: Enhanced `find_callers` and `find_references` to support searching by full method signatures.
- ✅ **Improved Search Accuracy**: Fixed limitations when searching with complex receivers or FQNs.
- ✅ **Comprehensive E2E Testing**: Validated the signature-based search pipeline across languages.

### v0.7.0 - Kotlin Support
- ✅ Complete Kotlin language support with 7 entity types
- ✅ tree-sitter-kotlin-ng v1.1.0 grammar compatibility
- ✅ Comprehensive E2E test suite (10 test cases)
- ✅ Full MCP server integration for Kotlin entities
- ✅ Support for classes, interfaces, objects, companion objects, functions, methods, and properties
- ✅ Extension functions and annotation extraction
