# knot

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-2024-brightgreen.svg)](https://www.rust-lang.org)

**knot** is a high-performance codebase indexer that extracts structural and semantic information from source code, enabling AI agents to understand, analyze, and navigate large code repositories. Currently supports Java and TypeScript, with planned support for JavaScript, HTML, CSS/SCSS, and Rust.

The indexer automatically builds:
- **Vector Search Database** (Qdrant) — semantic understanding via embeddings
- **Graph Database** (Neo4j) — architectural relationships via call graphs

This dual-database approach powers an **MCP (Model Context Protocol) server** that exposes three tools to any LLM client (Claude, Gemini, ChatGPT, Cursor, etc.).

---

## ✨ Key Features

**🔍 Code Intelligence Tools**
- **`search_hybrid_context`**: Semantic + structural search. Find code by meaning, class name, method signature, docstrings, or comments. Returns full context including dependencies.
- **`find_callers`**: Reverse dependency lookup. Identify dead code, perform impact analysis, or understand the full call chain of any function/method.
- **`explore_file`**: File anatomy inspection. Quickly see all classes, interfaces, methods, and functions in a file with signatures and documentation.

**🏗️ Multi-Language Support**
- **Java**: Full AST extraction with package awareness
- **TypeScript/TSX/CTS**: Complete support for modern JavaScript/TypeScript codebases, including CommonJS TypeScript files
- **JavaScript** (Planned): Vanilla JS, Node.js, and module systems (`.js`, `.mjs`, `.cjs`, `.jsx`)
- **HTML** (Planned): Document structure, Web Components, embedded scripts
- **CSS/SCSS** (Planned): Stylesheet indexing with variable and mixin tracking
- **Rust** (Planned): Struct, trait, and macro analysis

**📚 Rich Comment Extraction**
- Captures docstrings (JavaDoc, JSDoc) preceding declarations
- Extracts inline comments within method/function bodies
- Respects nesting boundaries (class comments don't capture method comments)
- Intelligently aggregates comment blocks

**📊 Dual-Database Architecture**
- **Qdrant**: Vector search for semantic code understanding
- **Neo4j**: Graph relationships for structural navigation

**🚀 High Performance**
- Parallel AST extraction via Rayon
- Concurrent database writes via Tokio
- Batch processing with configurable chunk sizes
- Scales to thousands of files

---

## 🛠️ Installation

### Prerequisites

| Component    | Version | Notes                              |
|--------------|---------|-----------------------------------|
| Docker       | 20.10+  | For running Qdrant and Neo4j      |
| qdrant       | 1.x     | Vector database (docker)          |
| neo4j        | 5.x     | Graph database (docker)           |

### Option A: Pre-compiled Binaries (macOS & Modern Linux)

Go to the [Releases](https://github.com/raultov/knot/releases) page and download the native executable for your platform.

**Install via Shell Script (macOS & Linux):**
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/raultov/knot/releases/latest/download/knot-installer.sh | sh
```

**Linux Requirements:**
- **Minimum glibc version**: 2.38+
- **Compatible distributions**:
  - Ubuntu 24.04 LTS or later
  - Debian 13 (Trixie) or later
  - Fedora 39+ / RHEL 10+
  - Arch Linux (rolling release)

**For older Linux distributions or Windows**, use Docker (see Option B) or build from source (see Option C).

### Option B: Docker (Universal Compatibility)

Docker images provide universal compatibility for **any Linux distribution** (including older versions with glibc < 2.38) and **Windows**.

**Build the image locally:**
```bash
docker build -t knot:latest . --network=host
```

**Run the indexer:**
```bash
# Use --network host to connect to databases running on your host machine
docker run --rm \
  -v /path/to/your/repo:/workspace \
  -e KNOT_REPO_PATH=/workspace \
  -e KNOT_NEO4J_PASSWORD=your-password \
  --network host \
  knot:latest \
  knot-indexer
```

**Run the MCP server:**
```bash
docker run --rm \
  -e KNOT_REPO_PATH=/workspace \
  -e KNOT_NEO4J_PASSWORD=your-password \
  --network host \
  knot:latest \
  knot-mcp
```

**Note:** The `Dockerfile` uses a multi-stage build (`builder` stage with Rust, `runtime` stage with Debian Trixie) to ensure a minimal, high-performance image. Use `--network host` to allow the container to access Qdrant and Neo4j running on your host machine.

### Option C: Install via Cargo

```bash
cargo install --git https://github.com/raultov/knot
```

### Option D: Build from Source

**1. Start infrastructure with Docker:**
```bash
docker compose up -d
```

**2. Clone and build:**
```bash
git clone https://github.com/raultov/knot
cd knot
cargo build --release
```

**3. Configure:**
```bash
cp .env.example .env
$EDITOR .env  # Set KNOT_REPO_PATH and Neo4j credentials
```

**4. Index a codebase:**
```bash
./target/release/knot-indexer
```

**5. Start the MCP server:**
```bash
./target/release/knot-mcp
```

---

## 📖 Usage

### Indexing a Codebase

```bash
# Using .env file (recommended)
knot-indexer

# Or specify repository directly
knot-indexer --repo-path /path/to/your/repo --neo4j-password secret
```

The indexer will:
1. Discover all supported source files (`.java`, `.ts`, `.tsx`, `.cts`, and future languages)
2. Extract entities via Tree-sitter AST parsing
3. Generate vector embeddings
4. Store in both Qdrant and Neo4j

### Using the MCP Server

The MCP server exposes three tools to any compatible AI client:

#### Tool 1: `search_hybrid_context` 
**Find code by meaning or keywords**

```
Query: "How is user authentication implemented?"
Result: All auth-related code, signatures, docstrings, and dependencies
```

**Capabilities:**
- Semantic search by functionality
- Class/method/function name lookup
- Docstring and inline comment search
- Architectural pattern discovery
- Full dependency context

#### Tool 2: `find_callers`
**Find who calls a specific function**

```
Query: "Find callers of getCurrentTimeInSeconds"
Result: All code that invokes this function + file locations
```

**Use Cases:**
- **Dead Code Detection**: Zero callers = unused code
- **Impact Analysis**: "What breaks if I modify this?"
- **Refactoring Safety**: Find all references before removing

#### Tool 3: `explore_file`
**Understand file structure**

```
Query: "What's in BrowserService.ts?"
Result: All classes, methods, and functions with signatures and docs
```

**Use Cases:**
- Quick file navigation
- Module structure overview
- Finding all methods in a class without reading line-by-line

---

## 🔗 MCP Client Configuration

### Supported Clients

knot works with any MCP-compatible AI client:
- ✅ **Claude Desktop** (Anthropic)
- ✅ **Gemini CLI** (Google)
- ✅ **ChatGPT CLI / GPT** (OpenAI)
- ✅ **Cursor** (AI IDE)
- ✅ **Any standard MCP client**

### Configuration Examples

#### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "knot": {
      "command": "/absolute/path/to/knot/target/release/knot-mcp",
      "env": {
        "KNOT_REPO_PATH": "/path/to/indexed/repo",
        "KNOT_QDRANT_URL": "http://localhost:6334",
        "KNOT_NEO4J_URI": "bolt://localhost:7687",
        "KNOT_NEO4J_USER": "neo4j",
        "KNOT_NEO4J_PASSWORD": "your-password"
      }
    }
  }
}
```

#### Gemini CLI

```bash
gemini mcp add knot /absolute/path/to/knot/target/release/knot-mcp
gemini mcp enable knot  # Inside Gemini CLI session
```

#### ChatGPT / GPT CLI

Similar JSON configuration in your client's MCP configuration file.

---

## ⚙️ Configuration Reference

All options can be set via environment variables (`.env`) or CLI flags. Environment variables take precedence.

| `.env` Variable            | CLI Flag                   | Default                     | Description                                              |
|----------------------------|----------------------------|-----------------------------|----------------------------------------------------------|
| `KNOT_REPO_PATH`           | `--repo-path`              | *(required)*                | Root directory of the repository to index                |
| `KNOT_REPO_NAME`           | `--repo-name`              | *(auto-detected)*           | Repository name for multi-repo isolation (auto-detected from last path component) |
| `KNOT_QDRANT_URL`          | `--qdrant-url`             | `http://localhost:6334`     | Qdrant server URL                                        |
| `KNOT_QDRANT_COLLECTION`   | `--qdrant-collection`      | `knot_entities`             | Qdrant collection name                                   |
| `KNOT_NEO4J_URI`           | `--neo4j-uri`              | `bolt://localhost:7687`     | Neo4j Bolt URI                                           |
| `KNOT_NEO4J_USER`          | `--neo4j-user`             | `neo4j`                     | Neo4j username                                           |
| `KNOT_NEO4J_PASSWORD`      | `--neo4j-password`         | *(required)*                | Neo4j password                                           |
| `KNOT_CUSTOM_QUERIES_PATH` | `--custom-queries-path`    | *(unset)*                   | Directory with custom `java.scm` / `typescript.scm`      |
| `KNOT_EMBED_DIM`           | `--embed-dim`              | `384`                       | Embedding vector dimension                               |
| `KNOT_BATCH_SIZE`          | `--batch-size`             | `64`                        | Entities per batch                                       |
| `RUST_LOG`                 | *(env only)*               | `info`                      | Log level: `trace`, `debug`, `info`, `warn`, `error`     |

---

## 🎨 Custom Tree-sitter Queries

The built-in extraction queries (`queries/java.scm`, `queries/typescript.scm`) can be overridden without recompiling:

```bash
KNOT_CUSTOM_QUERIES_PATH=/path/to/my/queries ./target/release/knot-indexer
```

Place `java.scm` and/or `typescript.scm` in your custom directory. Missing files fall back to built-in defaults.

---

## 🔄 Workflow Example

**Step 1: Index a Java project**
```bash
./target/release/knot-indexer --repo-path /home/user/my-java-app --neo4j-password secret
```

**Step 2: Start MCP server**
```bash
./target/release/knot-mcp
```

**Step 3: Use with Claude Desktop**
- Claude will list the three tools in its Tools menu
- Ask: "Search for all authentication logic"
- Ask: "Find who calls the login method"
- Ask: "Explore the structure of UserService.java"

### 🤖 Auto-Configuring AI Agents

**knot** includes a universal **`.prompt`** file in its root directory that automatically configures modern AI coding agents (Cursor, Cline, opencode, Claude, etc.) to use the `knot-mcp` tools correctly.

The directive explicitly instructs AI agents to prioritize:
- **`search_hybrid_context`** — for semantic code discovery (instead of `grep`)
- **`find_callers`** — for reverse dependency analysis (instead of finding references manually)
- **`explore_file`** — for file structure inspection (instead of reading line-by-line)

This ensures that when you ask an AI agent to analyze, refactor, or understand your code, it leverages the full power of the vector and graph databases rather than falling back to context-blind regex searches. The `.prompt` file is **universal and tool-agnostic**, working with any LLM client that reads codebase directives.

---

## 🤝 Contributing

Contributions are welcome! Please ensure:
- All code passes `cargo clippy`
- Code is formatted with `cargo fmt`
- Changes are compatible with Rust 2024 edition

---

## 📜 License

This project is licensed under the **MIT License**. See [LICENSE](LICENSE) for details.

---

---

## 📝 Changelog

### v0.3.3.1 (Current Release - Patch)
**Released:** 2026-04-06

**Bug Fix:**
- Fixed index misalignment in fallback pass where `covered_ranges` vector was not aligned with `entities` vector, causing orphaned reference intents to be assigned to wrong entities. Now ensures `covered_ranges[i]` corresponds to `entities[i]`.

---

### v0.3.3
**Released:** 2026-04-06

**Orphaned Reference Intent Capture:**
- **Fallback Pass**: Implemented a third parsing pass that captures call expressions, constructor invocations, and JSX component invocations that occur outside of named entities (functions, methods, classes).
- **Callback Handling**: Fixes critical blind spot where anonymous callbacks, top-level statements, and module-level expressions were completely invisible to the indexer.
- **Heuristic Assignment**: Orphaned intents are intelligently assigned to the nearest entity by byte position, or to a synthetic `<module>` entity if the file contains no named entities.
- **Impact**: Dramatically improves tracking for MCP server callbacks, event handlers, middleware chains, and any code that exists in the top-level scope.
- **Example Fix**: Previously, calls like `formatRegistryItems()` inside `server.setRequestHandler()` callbacks were invisible. Now they are properly tracked and discoverable via `find_callers`.

---

### v0.3.2
**Released:** 2026-04-06

**Fix:**
- Correctly include missing JSX rules file (`queries/tsx.scm`) and necessary logic updates to fully resolve Tree-sitter compilation warnings for `.ts` files.

---

### v0.3.1
**Released:** 2026-04-06

**Fix:**
- Separated JSX rules into `tsx.scm` to eliminate Tree-sitter compilation warnings for `.ts` files, maintaining full React support in `.tsx` files.

**React/JSX Component Invocation Tracking:**
- **JSX Support**: Full AST extraction and indexing of React component invocations via JSX syntax (e.g., `<ChartToolbar />`).
- **Component Discovery**: `find_callers` now correctly identifies all locations where a component is rendered, resolving the critical blind spot for React/frontend projects.
- **Namespaced Components**: Support for component hierarchies (e.g., `<Sheet.Content />`, `<Icons.Search />`), properly tracking receiver and method relationships.
- **HTML Tag Filtering**: Automatic filtering of native HTML tags (lowercase: `<div>`, `<span>`) to avoid false positives, using React naming convention (uppercase = component).
- **Dual Syntax Tracking**: Both traditional function calls (`ChartToolbar()`) and JSX invocations (`<ChartToolbar />`) are now unified under the same `CALLS` relationship in Neo4j.
- **Specification**: See [JSX/React Support Specification](docs/specs/jsx_react_support.md) for technical details and implementation notes.

---

### v0.2.6
**Released:** 2026-04-05

**Enum and Static Member Access Tracking:**
- **Static Member Resolution**: Extended the indexer to capture access to static class members and enum values (e.g., `WebWorkerEvent.Console`) within method bodies, function bodies, and constant/field initializers.
- **Improved AST Traversal**: Added recursive search for `member_expression` nodes with capitalized object identifiers, allowing tracking of previously "blind" references.
- **Discoverability**: `find_callers` now accurately identifies all references to Enums, significantly improving reverse dependency lookup accuracy in TypeScript codebases.
- **Resolved Blind Spot**: Enum and static class member accesses are now properly linked as `REFERENCES` in the Neo4j graph.
- **Metrics**: Increased reference tracking coverage by ~45% in test repositories.

---

### v0.2.5
**Released:** 2026-04-05

**Comprehensive Relationship Tracking for Static Typing:**
- **Typed Relationships in Neo4j**: Added four relationship types to replace generic CALLS edges:
  - **CALLS**: Method/function invocations and constructor instantiations
  - **EXTENDS**: Class inheritance relationships
  - **IMPLEMENTS**: Interface implementation relationships
  - **REFERENCES**: Type annotations and signatures (variable types, return types, parameters)
  - This resolves the fundamental blind spot where abstract classes and interfaces were missing inheritance and usage information

**Enhanced MCP Tools:**
- **`find_callers` Redesigned as `find_references`**: Now returns comprehensive results grouped by relationship type
  - Shows all subclasses via EXTENDS relationships
  - Shows all implementers via IMPLEMENTS relationships  
  - Shows all type usages via REFERENCES relationships
  - Shows all method callers via CALLS relationships
  - Dramatically improves impact analysis for abstract classes, interfaces, and type-heavy code
  
- **`search_hybrid_context` Enriched**: Automatically includes related entities in search results
  - Augments results with subclasses, implementers, and usage samples
  - Provides call counts with sample locations
  - Enables "what depends on this?" context exploration

**Parser & Ingest Improvements:**
- **ReferenceIntent Enum**: Refactored call tracking to support all four relationship types uniformly
- **RelationshipType Enum**: Explicit representation of graph relationship semantics in code
- **Two-Phase Resolution**: Separate raw intent extraction from UUID resolution for cleaner architecture

**Fixes:**
- **Complete Inheritance Resolution**: Abstract classes now properly tracked as base classes in inheritance chains
- **Type Reference Tracking**: Variables, parameters, and return types now contribute to usage metrics
- **Impact Analysis**: Interface changes now correctly identify all implementing classes
- **Compiler/Query Fixes**: Resolved type mismatch and Tree-sitter query syntax errors introduced in release process.

---

### v0.2.4
**Released:** 2026-04-04

**Parser Enhancements:**
- **TypeScript Abstract Declarations**: Added support for parsing and extracting `abstract class` and `abstract` method signatures
  - These are now properly indexed as `Class` and `Method` entities
  - Enables accurate `find_callers` impact analysis and call graph tracking for interfaces and abstract implementations
  - Resolved issue where abstract methods inside TypeScript files were skipped by AST extraction

---

### v0.2.3
**Released:** 2026-04-04

**Language Support Enhancement:**
- **CTS (CommonJS TypeScript) Support**: Added support for `.cts` files used in TypeScript projects targeting CommonJS module systems
  - Typical use case: Mocha custom interfaces, test runners, and Node.js utilities in TypeScript monorepos
  - Treated as standard TypeScript files by Tree-sitter AST extraction
  - Full entity extraction and call graph analysis support

---

### v0.2.2
**Released:** 2026-04-04

**Continuous Delivery & Stability Improvements:**
- **Release Targets**: Removed `aarch64-unknown-linux-gnu` target from automated native builds to resolve critical C++ linker errors caused by cross-compilation.
- **Docker**: Reverted accidental Dockerfile command change and ensured multi-stage build uses `debian:trixie-slim` for proper glibc compatibility.
- **Dockerfile**: Optimized `.dockerignore` to prevent permission issues during build.

**LLM Agent Optimization:**
- **`.prompt` Directive**: Added universal system instructions for all AI agents (Cursor, Cline, Claude, opencode, etc.)
  - Mandates use of `search_hybrid_context`, `find_callers`, `explore_file` over traditional grep/rg/find
  - Works with any LLM client that reads codebase directives (tool-agnostic)
  - Dramatically improves code understanding by leveraging Vector + Graph databases instead of regex searches

---

### v0.2.1
**Released:** 2026-04-04

**Continuous Delivery & Compatibility:**
- **Docker Support**: Added official Docker images (`ghcr.io/raultov/knot`) for universal compatibility:
  - Multi-stage build (`rust:1.90` builder + `debian:trixie` runtime)
  - Works on **any Linux distribution** (including older versions with glibc < 2.38)
  - Solves linking issues with glibc requirement (2.38+) found in native binaries.
- **Release Targets**:
  - Removed Windows native build due to unresolved MSVC linker compatibility issues with ONNX Runtime.
  - Linux builds now use `ubuntu-24.04` runner (glibc 2.39).
- **README Updates**: Clarified requirements, added Docker installation steps, and documented usage of `--network host` for database connectivity.

---

### v0.2.0
**Released:** 2026-04-04

**Continuous Delivery:**
- Integrated `cargo-dist` to automatically generate pre-compiled native binaries for macOS and modern Linux distributions on every GitHub release.
- Pre-built binaries for:
  - **macOS**: Apple Silicon (`aarch64-apple-darwin`)
  - **Linux**: x86_64 (`x86_64-unknown-linux-gnu`) and ARM64 (`aarch64-unknown-linux-gnu`)
- Added 1-click shell installer script for quick installation on compatible systems.


---

### v0.1.3
**Released:** 2026-04-04

**Performance Improvements:**
- **Qdrant Payload Index**: Added Keyword index on `repo_name` field for dramatically faster multi-repository filtering
  - Reduces search latency for repository-specific queries from O(n) to effectively O(1)
  - Maintains single collection architecture (`knot_entities`) for optimal RAM usage at scale
  - Enables efficient handling of hundreds of repositories with millions of vectors

**Code Quality:**
- Cleaned up internal references, replacing project-specific examples with generic `com.example` namespace

---

### v0.1.2
**Released:** 2026-04-04

**Major Features:**
- **Multi-Repository Isolation**: New `repo_name` field enables logical separation of multiple repositories in shared Qdrant + Neo4j infrastructure
  - Auto-detection: Extracts repository name from the last path component (e.g., `/path/to/my-java-repo` → `my-java-repo`)
  - Manual override: Use `--repo-name` CLI flag or `KNOT_REPO_NAME` environment variable
  - Database-level filtering: All queries (vector + graph) filter by repository when specified

**MCP Tools Enhancement:**
- All three tools now support optional `repo_name` parameter for filtered searches:
  - `search_hybrid_context`: Search within a specific repository or across all
  - `find_callers`: Find callers in a specific repository context
  - `explore_file`: Explore files in a specific repository
- Backward compatible: Omit `repo_name` to search across all indexed repositories

**Improvements:**
- Qdrant: Uses keyword filter on `repo_name` payload field for efficient vector filtering
- Neo4j: Uses parameterized Cypher queries with optional `repo_name` WHERE conditions
- Zero clippy warnings, production-ready code quality
- Fully backward compatible with v0.1.1 indexing

**Testing:**
- Compilation: Zero warnings with `cargo build --release`
- Linting: Zero clippy warnings with `cargo clippy --all-targets`
- Code quality: All code formatted with `cargo fmt`

### v0.1.1
**Released:** 2026-04-04

**Major Features:**
- **New Entity Types**: Added `Constant` and `Enum` as first-class entity kinds
- **Decorator Extraction**: Automatic detection and indexing of framework decorators/annotations
  - TypeScript: `@decorator` syntax (e.g., `@Post()`, `@OnEvent()`, `@Controller()`)
  - Java: `@Annotation` syntax (e.g., `@Override`, `@GetMapping()`)
- **Enhanced Callback Detection**: Improved TypeScript call graph to detect:
  - `.bind(this)` patterns (e.g., `this.method.bind(this)`)
  - Callback arguments passed to functions (e.g., `app.use(this.handler)`)

**Improvements:**
- Decorators are now indexed in embeddings for framework-aware search
- Better framework integration discovery for NestJS, Spring, etc.
- Zero clippy warnings, production-ready code quality
- Tested on real codebases (171 TypeScript entities, 67 Java entities)

**Testing:**
- TypeScript project: 171 entities indexed with framework decorators
- Java project: 67 entities indexed
- 228 CALLS relationships detected across both projects
- Neo4j persistence verified

### v0.1.0
- Initial release with dual-database architecture (Qdrant + Neo4j)
- Three MCP tools: `search_hybrid_context`, `find_callers`, `explore_file`
- Support for Java and TypeScript/TSX
- Comment extraction (docstrings and inline comments)
- Call graph analysis

---

## 🚀 Roadmap

### Near-Term (v0.3.x — Multi-Language Foundation)

#### Phase 1: JavaScript (Vanilla & Modules) Support
- [ ] Support `.js`, `.mjs`, `.cjs`, `.jsx` files
- [ ] Parallel indexing of hybrid JS/TS projects
- [ ] Call graph analysis for JavaScript functions and classes
- [ ] JSDoc comment extraction

#### Phase 2: HTML Indexing
- [ ] Support `.html` and `.htm` files
- [ ] Extract HTML elements, IDs, and classes
- [ ] Web Components recognition
- [ ] Embedded script/style block extraction

#### Phase 3: CSS & SCSS Support
- [ ] Support `.css`, `.scss`, `.sass` files
- [ ] Index CSS/SCSS selectors, variables, and mixins
- [ ] Track selector usage and definitions
- [ ] SCSS function and mixin extraction

#### Phase 4: Hybrid Web Ecosystem
- [ ] Cross-language dependency resolution (JS ↔ HTML ↔ CSS)
- [ ] Link JavaScript DOM operations to HTML elements
- [ ] Connect CSS class usage in JavaScript to stylesheets
- [ ] Enable full-stack SPA indexing

**See the [Detailed Multi-Language Roadmap](docs/specs/multilanguage_roadmap.md) for technical specifications.**

### Medium-Term (v0.4.x — Performance & Polish)
- [ ] Incremental indexing (skip unchanged files)
- [ ] Parallel processing optimizations for large mono-repos
- [ ] Cross-repository dependency analysis
- [ ] Custom code analysis rules

### Future (v0.5.x+)

#### Phase 5: Rust Support
- [ ] Support `.rs` files
- [ ] Struct, trait, and impl tracking
- [ ] Macro invocation analysis
- [ ] Ownership-aware call graph analysis

#### Long-Term Vision
- [ ] Python support
- [ ] Go support
- [ ] IDE plugins (VS Code, IntelliJ, Vim)
- [ ] Web UI for graph visualization
- [ ] Language Server Protocol (LSP) integration

---

## 💬 Questions?

For issues, feature requests, or discussions, please open a GitHub issue.
