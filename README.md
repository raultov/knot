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
| Rust         | 1.85+   | Edition 2024                       |
| Docker       | 20.10+  | For running Qdrant and Neo4j      |
| qdrant       | 1.x     | Vector database (docker)          |
| neo4j        | 5.x     | Graph database (docker)           |

### Quick Start

**1. Start infrastructure with Docker:**
```bash
docker compose up -d
```

**2. Build the project:**
```bash
git clone https://github.com/your-org/knot
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
./target/release/knot-indexer

# Or specify repository directly
./target/release/knot-indexer --repo-path /path/to/your/repo --neo4j-password secret
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

## 📦 Project Structure

```
knot/
├── Cargo.toml                          # Rust dependencies
├── docker-compose.yml                  # Local infrastructure (Qdrant + Neo4j)
├── .env.example                        # Configuration template
├── README.md                           # This file
├── queries/
│   ├── java.scm                        # Java AST extraction rules
│   └── typescript.scm                  # TypeScript AST extraction rules
├── testing_files/                      # Test scripts and fixtures
├── src/
│   ├── lib.rs                          # Shared library root
│   ├── models.rs                       # Core data structures
│   ├── config.rs                       # Configuration management
│   ├── mcp_handler.rs                  # MCP protocol handler
│   ├── mcp_tools/                      # Three MCP tools
│   │   ├── search_hybrid_context.rs
│   │   ├── find_callers.rs
│   │   └── explore_file.rs
│   ├── db/                             # Database clients
│   │   ├── vector.rs                   # Qdrant wrapper
│   │   └── graph.rs                    # Neo4j wrapper
│   ├── pipeline/                       # 5-stage indexing pipeline
│   │   ├── input.rs                    # Stage 1: file discovery
│   │   ├── parse.rs                    # Stage 2: AST extraction
│   │   ├── prepare.rs                  # Stage 3: UUID + embedding text
│   │   ├── embed.rs                    # Stage 4: vector generation
│   │   └── ingest.rs                   # Stage 5: dual-write
│   ├── utils/                          # Logging and utilities
│   └── bin/
│       ├── knot-indexer.rs             # Batch indexer binary
│       └── knot-mcp.rs                 # MCP server binary
```

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

## 🚀 Roadmap

- [ ] Python support
- [ ] Go support
- [ ] Incremental indexing (skip unchanged files)
- [ ] Custom code analysis rules
- [ ] IDE plugins (VS Code, IntelliJ)

---

## 💬 Questions?

For issues, feature requests, or discussions, please open a GitHub issue.
