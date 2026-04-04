# knot

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-2024-brightgreen.svg)](https://www.rust-lang.org)

**knot** is a high-performance codebase indexer that extracts structural and semantic information from Java and TypeScript codebases, enabling AI agents to understand, analyze, and navigate large code repositories.

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
- **TypeScript/TSX**: Complete support for modern JavaScript/TypeScript codebases

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

### Option A: Pre-compiled Binaries (Recommended)

Go to the [Releases](https://github.com/raultov/knot/releases) page and download the native executable for your platform (macOS, Windows, Linux). We provide `.msi` installers for Windows and shell scripts for UNIX systems.

**Install via Shell Script (Unix):**
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/raultov/knot/releases/latest/download/knot-installer.sh | sh
```

**Install via PowerShell (Windows):**
```powershell
irm https://github.com/raultov/knot/releases/latest/download/knot-installer.ps1 | iex
```

### Option B: Install via Cargo

```bash
cargo install --git https://github.com/raultov/knot
```

### Option C: Build from Source

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
1. Discover all `.java`, `.ts`, `.tsx` files
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

### v0.2.0 (Current Release)
**Released:** 2026-04-04

**Continuous Delivery:**
- Integrated `cargo-dist` to automatically generate pre-compiled native binaries for macOS (Apple Silicon/Intel), Linux, and Windows on every GitHub release.
- Added 1-click installer scripts (`.sh` and `.ps1`) to simplify distribution without requiring a local Rust toolchain.

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

### v0.1.3 (Planned)
- [ ] Incremental indexing (skip unchanged files)
- [ ] Better performance for large mono-repos
- [ ] Cross-repository dependency analysis

### Future Versions
- [ ] Python support
- [ ] Go support
- [ ] Custom code analysis rules
- [ ] IDE plugins (VS Code, IntelliJ)
- [ ] Web UI for graph visualization

---

## 💬 Questions?

For issues, feature requests, or discussions, please open a GitHub issue.
