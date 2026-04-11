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

#### Incremental Indexing (Default, v0.4.3+)

```bash
# First run: indexes all files
knot-indexer --repo-path /path/to/your/repo --neo4j-password secret

# Subsequent runs: only re-indexes changed files (fast!)
knot-indexer --repo-path /path/to/your/repo --neo4j-password secret
```

**How it works:**
- Tracks file content via SHA-256 hashes in `.knot/index_state.json`
- Automatically detects: modified, added, and deleted files
- Only re-parses and re-embeds changed files
- Preserves graph relationships to unchanged files
- Processes entities in memory-efficient 512-entity chunks

**Performance:**
- **Initial index (3800 files)**: ~60 minutes on standard hardware
- **Incremental update (3 files changed)**: ~5-10 seconds
- **Memory usage**: Constant ~2GB regardless of repository size

#### Full Re-Index (Clean Mode)

```bash
# Force complete re-index (deletes all existing data)
knot-indexer --clean --repo-path /path/to/your/repo --neo4j-password secret
```

Use `--clean` when:
- You want to rebuild the entire index from scratch
- You've changed Tree-sitter queries or embedding models
- Troubleshooting indexing issues

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
| `KNOT_EMBED_DIM`           | `--embed-dim`              | `384`                       | Embedding vector dimension                               |
| `KNOT_BATCH_SIZE`          | `--batch-size`             | `64`                        | Entities per batch                                       |
| `KNOT_CLEAN`               | `--clean`                  | `false`                     | Force full re-index (delete all existing data)           |
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

## 🚀 Roadmap

### Current Release (v0.4.3 — Robust MCP & Pure Testability) ✅
- ✅ **MCP Layer Refactoring**: Decoupled tool execution logic in `enrich_with_relationships` and `build_server_details` into pure, testable functions.
- ✅ **Comprehensive Test Coverage**: Added 65+ new tests covering the entire MCP toolset (explore, search, find_callers).
- ✅ **Markdown Formatting Tests**: 20+ tests ensuring consistent and reliable search result formatting for AI clients.
- ✅ **Tool Consistency Suite**: Automated checks to ensure all MCP tools maintain required schemas and descriptions.
- ✅ **Indexer Refactoring**: Reduced `knot-indexer`'s `main()` function to a clean, declarative ~50 lines flow (v0.4.2).
- ✅ **Modular Architecture**: Decoupled monolithic modules into specialized sub-modules (v0.4.1).
- ✅ **Incremental Indexing**: Skip unchanged files by tracking SHA-256 content hashes (v0.4.0).
- ✅ **Memory-Efficient Chunking**: Process entities in 512-entity chunks (v0.4.0).
- ✅ **Deterministic UUIDs**: Migrated to deterministic UUID v5 (v0.4.0).
- ✅ **Selective Database Operations**: New `--clean` flag (v0.4.0).
- ✅ **Global Context Hydration**: Resolves relationships to entities in unchanged files (v0.4.0).

### Next (v0.5.0 — Performance & Scale++)
- [ ] Parallel processing optimizations for large mono-repos.
- [ ] Cross-repository dependency analysis.
- [ ] Watch mode for real-time incremental updates.

### Near-Term (v0.6.x — Multi-Language Foundation)
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

### Future (v0.7.x+)
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
