# knot

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-2024-brightgreen.svg)](https://www.rust-lang.org)
[![knot MCP server](https://glama.ai/mcp/servers/raultov/knot/badges/score.svg)](https://glama.ai/mcp/servers/raultov/knot)

<div align="center">
  <a href="https://glama.ai/mcp/servers/raultov/knot">
    <img src="https://glama.ai/mcp/servers/raultov/knot/badges/card.svg" alt="knot MCP server" />
  </a>
</div>

<div align="center">

### Knot in action

</div>

<table>
<tr>
<td width="50%" align="center">

<img src="demo-cli.gif" alt="knot CLI demo" width="120%">

**CLI** — instant reverse dependency lookup

</td>
<td width="50%" align="center">

<img src="demo-mcp.gif" alt="knot MCP demo" width="120%">

**MCP** — JSON-RPC protocol for AI agents

</td>
</tr>
</table>

**knot** is a high-performance codebase indexer that extracts structural and semantic information from source code, enabling AI agents to understand, analyze, and navigate large code repositories. Currently supports Java, Kotlin (v0.7.4+), TypeScript, JavaScript/Node.js, Rust (v0.8.x), Python (v0.9.3), **Groovy** (v0.10.3), **C/C++** (v1.0.0), HTML, and CSS/SCSS, plus **Build Systems** (Maven pom.xml, Gradle build.gradle, Jenkins pipeline, **Cargo.toml** — v1.2.5), **Configuration Files** (YAML, JSON, .properties — v1.2.5), **Kubernetes + Helm** (v1.2.5), and **Cross-Repo Dependency Linking** (v1.2.5) with full cross-language linking.

The indexer automatically builds:
- **Vector Search Database** (Qdrant) — semantic understanding via embeddings
- **Graph Database** (Neo4j) — architectural relationships via call graphs

This dual-database approach powers both:
- **MCP (Model Context Protocol) Server** — Exposes three tools to any LLM client (Claude, Gemini, ChatGPT, Cursor, etc.)
- **CLI Tool** (v0.10.1) — Standalone `knot` command for terminal and scripting environments

---

## ✨ Key Features

**🔍 Code Intelligence Tools**
- **`search_hybrid_context`**: Semantic + structural search. Find code by meaning, class name, method signature, docstrings, or comments. Returns full context including dependencies.
- **`find_callers`**: Reverse dependency lookup. Identify dead code, perform impact analysis, or understand the full call chain of any function/method. When multiple entities share the same name (e.g., `find_nearest_entity_by_line` in different files), results are automatically grouped by target showing which specific entity each caller references. Supports cross-repository call resolution via `DEPENDS_ON` graph edges.
- **`explore_file`**: File anatomy inspection. Quickly see all classes, interfaces, methods, and functions in a file with signatures and documentation.
- **`list_repo_dependencies`** (MCP) / **`knot deps`** (CLI): Dependency graph visualization. Show which repositories depend on each other, forward and reverse, with transitive resolution.

**🏗️ Multi-Language Support**
- **Java**: Full AST extraction with package awareness
- **Kotlin** (v0.7.4+): Complete support for Kotlin codebases with classes, interfaces, objects, companion objects, functions, methods, and properties. Fully compatible with tree-sitter-kotlin-ng grammar.
- **TypeScript/TSX/CTS**: Complete support for modern JavaScript/TypeScript codebases, including CommonJS TypeScript files
- **JavaScript/Node.js** (v0.7.4+): Vanilla JS, Node.js, and module systems (`.js`, `.mjs`, `.cjs`, `.jsx`)
- **Hybrid Web Ecosystem** (v0.6.5): Cross-language linking between JavaScript, HTML, and CSS for full-stack SPA analysis
- **HTML** (v0.6.3+): Custom elements (Web Components, Angular), `id` and `class` attribute indexing for cross-language CSS search
- **JSX/TSX Attributes** (v0.6.3+): Extracts `id` and `className` from React components for unified HTML/CSS discovery
- **CSS/SCSS** (v0.6.4+): Stylesheet indexing with class/ID selector extraction and variable tracking (CSS/SCSS variables, mixins, functions)
- **Rust** (v0.8.11): Struct, enum, union, trait, function, method, module extraction with trait implementation tracking (IMPLEMENTS relationships) and macro invocation references. **NEW in v0.8.6**: Type alias, constant, static, and macro definition extraction with full docstring and signature support. **NEW in v0.8.7**: Enhanced type reference detection inside macros (`vec![]`, `println!()`, `assert!()`, etc.) with intelligent string literal filtering and comprehensive edge case handling. **NEW in v0.8.11**: O(N) nested macro traversal optimization for large Rust codebases with deeply nested `token_tree` nodes.
- **Python** (v0.9.3): Full Python extraction with class, function, method support, constants, module-level imports, `ValueReference` tracking for keyword arguments, class inheritance (`EXTENDS`), decorator extraction (`@property`, `@staticmethod`, `@route(...)`, `@dataclass`), generic type hints (`List[str]`, `Optional[Dict]`, `*args`/`**kwargs`), Py2/Py3 exception syntax compatibility, and `self.method()` resolution with inherited method walking. Captures `class_definition`, `function_definition` (including async via optional `async` modifier), lambda assignments, and distinguishes methods from functions via parent context detection.
- **Groovy** (v0.10.3): Full Groovy language support via hybrid tree-sitter + ad-hoc lexical parser. Extracts classes, interfaces, traits, enums, typed/`def`/quoted methods (incl. Spock specs), constructors, closures, script-level variables, fields/properties with visibility modifiers, nested classes, and decorators. Tracks package FQN and enclosing class relationships. **NEW in v0.10.3**: Multi-line signatures (closure default params), assignment-vs-declaration disambiguation, innermost assignment for nested closures, UUID collision fix for duplicate method names, `find_callers` accurately tracks private methods including those in anonymous `new AnAction` closures.
- **Build Systems** (v0.10.0): Maven `pom.xml` (dependencies + plugins via roxmltree), Gradle `build.gradle` (deps + plugins + tasks), and `Jenkinsfile` pipeline (stages + steps) extraction.
- **Cargo.toml** (v1.2.0): Rust package manager support with package metadata, features, workspace members, and multi-format dependency parsing (simple, table, git, path).
- **Configuration Files** (v1.2.0): YAML (.yml/.yaml), JSON (.json), and Java Properties (.properties) with leaf-key granularity. Special handling for package.json (npm dependencies as BuildDependency, scripts as ConfigProperty).
- **Kubernetes + Helm** (v1.2.0): K8s manifest parsing (Deployment, Service, ConfigMap, Secret, Ingress, Namespace) with label/annotation tracking and cross-resource references. Helm chart indexing (Chart.yaml metadata, values.yaml key-value pairs, template variable extraction via {{ .Values.X }}).
- **C/C++** (v1.0.0): Complete C/C++ support with namespace-aware FQN resolution (`Engine::MyClass::start`), class/struct extraction, function/method tracking, macro definition and usage detection (uppercase identifier heuristic), type reference tracking (declarations, `new` expressions), and full call graph analysis. Supports `.c`, `.h`, `.cpp`, `.hpp`, `.cc`, `.cxx`, `.hh`, `.hxx` extensions via tree-sitter-c and tree-sitter-cpp parsers. Includes intelligent auto-detection for `.h` headers to parse them correctly as C or C++ based on their contents.

**📚 Rich Comment Extraction**
- Captures docstrings (JavaDoc, JSDoc) preceding declarations
- Extracts inline comments within method/function bodies
- Respects nesting boundaries (class comments don't capture method comments)
- Intelligently aggregates comment blocks

**📊 Dual-Database Architecture**
- **Qdrant**: Vector search for semantic code understanding
- **Neo4j**: Graph relationships for structural navigation

**🚀 High Performance**
- **Parallel Streaming Pipeline**: Overlaps CPU-bound embedding with I/O-bound ingestion via MPSC channels (v0.5.0+)
- **Incremental Indexing**: Uses SHA-256 hashes to skip unchanged files
- **Real-time Watch Mode**: Automatically re-indexes changed files in seconds via `--watch`
- **CPU Parallelism**: AST extraction via Rayon
- **Scalable**: Configurable batch processing and constant memory footprint (~2GB) regardless of repository size
- **Performance Benchmarking** (v1.1.0+): Three-level validation framework
  - *Unit benchmarks*: Criterion-based benchmarks for parse, embed, and graph write throughput (`benches/`)
  - *E2E benchmarks*: Full pipeline metrics capture with per-stage timing (`tests/benchmark_e2e.sh`)
  - *CI regression tracking*: Automated baseline comparison against tolerance thresholds (`scripts/compare_perf_metrics.sh`)

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

**Install knot binaries (CLI, MCP server, and indexer):**
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/raultov/knot/releases/latest/download/knot-installer.sh | sh
```

**Download agent-skills guides separately (optional):**
```bash
curl -sO https://raw.githubusercontent.com/raultov/knot/master/.knot-agent.md && curl -fsSL https://raw.githubusercontent.com/raultov/knot/master/.knot-agent-skills.tar.gz | tar -xz
```

The first command installs the `knot` binary to your PATH. The second (optional) downloads the agent skill index (`.knot-agent.md`) and extracts comprehensive guides for using knot CLI with AI agents and code analysis tools.

**Linux Requirements:**
- **Full install (knot-indexer + CLI + MCP)**: glibc 2.38+
  - Ubuntu 24.04 LTS or later
  - Debian 13 (Trixie) or later
  - Fedora 39+ / RHEL 10+
  - Arch Linux (rolling release)
- **Lightweight clients-only (knot CLI + MCP server, no indexing)**: glibc 2.35+ (even older systems like Debian 12 Bookworm work fine)

**For older Linux distributions or Windows**, see the **Lightweight Clients** section below or use Docker (Option B).

### Option B: Docker (Universal Compatibility)

Docker images provide universal compatibility for **any Linux distribution** and **Windows**.

#### Full Install (All Binaries: knot-indexer, knot CLI, knot-mcp)

**Build the image:**
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

**Run the CLI tool:**
```bash
docker run --rm \
  -v /path/to/your/repo:/workspace \
  -e KNOT_REPO_PATH=/workspace \
  -e KNOT_NEO4J_PASSWORD=your-password \
  --network host \
  knot:latest \
  knot search "user login flow"
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

**Note:** Uses Debian Trixie (glibc 2.38+) and includes ONNX Runtime for full functionality.

---

#### Lightweight Clients (Only knot CLI + knot-mcp, No Indexer)

For older systems (Debian 12 Bookworm, Ubuntu 22.04) or production deployments that only need to **query existing indexes** without indexing new code:

**Build the lightweight image:**
```bash
docker build -t knot:clients -f Dockerfile.clients . --network=host
```

Image size: ~100MB (vs ~160MB for full install)

**Run the CLI tool (query existing index):**
```bash
docker run --rm \
  --network host \
  knot:clients \
  knot callers "MyClass"
```

**Run the MCP server:**
```bash
docker run --rm \
  --network host \
  knot:clients \
  knot-mcp
```

**Available tools in lightweight mode:**
- ✅ `knot search` (structural only, no semantic search)
- ✅ `knot callers` (reverse dependency lookup)
- ✅ `knot explore` (file structure inspection)
- ❌ Semantic search requires the full install

**Note:** Uses Debian Bookworm (glibc 2.35+) and excludes ONNX Runtime, making it compatible with older Linux distributions.

### Option C: Install via Cargo

```bash
cargo install --git https://github.com/raultov/knot
```

### Option D: Build from Source

**Full Install (All Binaries):**

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

**5. Query via CLI:**
```bash
./target/release/knot search "your query"
```

### Option E: Lightweight Clients (No Indexing)

For older Linux distributions (e.g. Debian 12 Bookworm, Ubuntu 22.04) or production deployments where you only need the **CLI and MCP server** (not the indexer), compile without the embedding dependencies:

**Build lightweight clients:**
```bash
cargo build --release --no-default-features --features only-clients
```

This produces only `knot` and `knot-mcp` binaries (~8-10 MB each), excluding the 30+ MB of ONNX Runtime dependencies.

**Available tools in lightweight mode:**
- ✅ **`find_callers`**: Reverse dependency lookup (graph search). Automatically groups results by target when multiple entities share the same name.
- ✅ **`explore_file`**: File structure inspection
- ❌ **`search_hybrid_context`**: Semantic search (requires embeddings, not available in this mode)

**Use case:** Query an existing Qdrant + Neo4j index that was built elsewhere, without needing the indexer on your machine.

**Docker alternative (for lightweight mode):**
```bash
docker build -t knot:clients-only -f Dockerfile -f - . << 'EOF'
FROM rust:1.90-slim-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --no-default-features --features only-clients
FROM debian:bookworm-slim
COPY --from=builder /build/target/release/knot* /usr/local/bin/
CMD ["knot-mcp"]
EOF
```

**6. Start the MCP server:**
```bash
./target/release/knot-mcp
```

---

## 📖 Usage

### 📥 Quick Downloads

**Download knot binaries (CLI + MCP server):**
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/raultov/knot/releases/latest/download/knot-installer.sh | sh
```

**Download agent-skills documentation (index + all guides):**
```bash
curl -sO https://raw.githubusercontent.com/raultov/knot/master/.knot-agent.md && curl -fsSL https://raw.githubusercontent.com/raultov/knot/master/.knot-agent-skills.tar.gz | tar -xz
```

### 📖 Agent-Skills Guides

Comprehensive documentation for using knot tools. The download above extracts:
- **search.md** — Semantic code discovery guide with examples
- **callers.md** — Reverse dependency lookup with critical usage rules
- **explore.md** — File anatomy inspection guide
- **workflows.md** — Common patterns and best practices

For quick reference without downloading, see [`.knot-agent.md`](.knot-agent.md).

---

### Using the CLI (v0.8.0+)

The **knot CLI** provides the same capabilities as the MCP server via command-line commands, making it ideal for:
- Terminal-only environments
- Bash scripting and automation
- CI/CD pipelines
- Direct integration with other tools

**Three main commands:**

#### `knot search` — Semantic Code Search
```bash
knot search "user authentication" --max-results 10 --repo my-app
```
Find code entities by meaning, class names, docstrings, or comments.

#### `knot callers` — Reverse Dependency Lookup
```bash
knot callers "LoginService" --repo my-app
```
Find all code that references a specific entity (dead code detection, impact analysis, call chains). When multiple entities share the same name in different files, results are automatically grouped by target with file locations and signatures.

#### `knot explore` — File Structure Inspection
```bash
knot explore "src/services/auth.ts" --repo my-app
```
List all classes, methods, functions in a file with signatures and documentation.

#### `knot deps` — Repository Dependency Graph (v1.2.5)
```bash
knot deps my-app --depth 2           # Show forward dependencies (transitive)
knot deps my-app --reverse           # Show who depends on this repo
```
Visualize auto-discovered dependencies between indexed repositories with transitive resolution up to 3 levels deep.

**For detailed CLI usage guide**, see [`.knot-agent.md`](.knot-agent.md) — a machine-readable skill that teaches LLMs how to use knot CLI for autonomous code analysis.

### Indexing a Codebase

#### Incremental Indexing (Default, v0.4.3+)

```bash
# First run: indexes all files
knot-indexer --repo-path /path/to/your/repo --neo4j-password secret

# Subsequent runs: only re-indexes changed files (fast!)
knot-indexer --repo-path /path/to/your/repo --neo4j-password secret

# NEW: Real-time Watch mode (v0.5.2+)
knot-indexer --watch --repo-path /path/to/your/repo --neo4j-password secret
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

### Running E2E Integration Tests

To ensure indexer stability, run the E2E integration test suite:

```bash
# Run all language E2E tests (Java, TS, JS, HTML, CSS, Kotlin, Rust)
./tests/run_e2e.sh

# Run only Kotlin E2E tests
./tests/run_kotlin_e2e.sh

# Run only Rust E2E tests
./tests/run_rust_e2e.sh
```

See `tests/KOTLIN_E2E_TESTS.md` for detailed coverage and troubleshooting.

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

**Advanced: Search by Signature (NEW in v0.7.4)**
```bash
# Find by full signature (Java)
echo '{"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"registerUser(String"}}}' | knot-mcp

# Find by parameter type (Kotlin)
echo '{"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"findById(Int"}}}' | knot-mcp

# Find by type annotation (TypeScript)
echo '{"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"(EventData"}}}' | knot-mcp
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
| `KNOT_CUSTOM_CA_CERTS`     | `--custom-ca-certs`       | *(none)*                    | Path to CA certificate bundle for corporate SSL proxies  |
| `RUST_LOG`                 | *(env only)*              | `info`                      | Log level: `trace`, `debug`, `info`, `warn`, `error`     |

---

## 🎨 Custom Tree-sitter Queries

The built-in extraction queries (`queries/java.scm`, `queries/typescript.scm`) can be overridden without recompiling:

```bash
KNOT_CUSTOM_QUERIES_PATH=/path/to/my/queries ./target/release/knot-indexer
```

Place `java.scm` and/or `typescript.scm` in your custom directory. Missing files fall back to built-in defaults.

---

## 🔐 Corporate SSL / CA Certificates

In restricted corporate environments with SSL-inspecting proxies, you may need to provide a custom CA certificate bundle so that `knot` can download the embedding model from HuggingFace.

**Via environment variable:**
```bash
export KNOT_CUSTOM_CA_CERTS=/etc/ssl/certs/corporate-bundle.pem
./target/release/knot-indexer --repo-path /path/to/repo --neo4j-password secret
```

**Via CLI flag:**
```bash
./target/release/knot-indexer \
  --custom-ca-certs /etc/ssl/certs/corporate-bundle.pem \
  --repo-path /path/to/repo \
  --neo4j-password secret
```

**Via `.env` file:**
```bash
echo "KNOT_CUSTOM_CA_CERTS=/etc/ssl/certs/corporate-bundle.pem" >> .env
./target/release/knot-indexer
```

This works for all three binaries: `knot-indexer`, `knot-mcp`, and `knot`.

---

## 🔄 Workflow Example

**Step 1: Index a Java project**
```bash
./target/release/knot-indexer --repo-path /home/user/my-java-app --neo4j-password secret
```

**Step 2: Query via CLI (Instant search)**
```bash
./target/release/knot search "authentication logic"
./target/release/knot callers "UserService.login"
```

**Step 3: Start MCP server (For AI Agents)**
```bash
./target/release/knot-mcp
```

**Step 4: Use with Claude Desktop**
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
- All new functionality includes unit tests
- Performance regressions are validated with the benchmark framework before submitting PRs

### Performance Benchmarking

The project includes a three-level benchmarking framework to validate optimizations and detect regressions:

**Level 1 — Unit Benchmarks (Criterion):**
```bash
cargo bench --bench pipeline_bench          # Parse + prepare throughput per language
cargo bench --bench graph_upsert_bench     # Neo4j UNWIND batching speedup (needs Neo4j)
cargo bench --bench channel_backpressure_bench  # Bounded channel overhead
```

**Level 2 — E2E Integration Benchmarks:**
```bash
# Full pipeline metrics with memory and per-stage timing
./tests/benchmark_e2e.sh --focus rust_e2e --output-dir /tmp/perf_results

# Compare against baseline (fails CI if tolerance exceeded)
scripts/compare_perf_metrics.sh /tmp/perf_results .perf_metrics/baseline.json
```

**Baseline files:** `.perf_metrics/baseline.json` stores the last known good metrics (committed, updated on main/master merges). Tolerance thresholds in `.perf_metrics/threshold_tolerances.json` control regression gates (±5% time, ±10% memory by default).

**CI Integration:** The `test-performance` job in `.github/workflows/ci.yml` runs after all E2E correctness tests pass, comparing results against baseline and fails the build on regression.

---

## 📜 License

This project is licensed under the **MIT License**. See [LICENSE](LICENSE) for details.

---

## 🚀 Roadmap

### Current Release (v1.2.5 — Cargo.toml, Config Files, Kubernetes + Helm, Cross-Repo Linking) ✅
- ✅ **Phase 12A — Cargo.toml Parser**: Package metadata, dependencies (simple/table/git/path), features, workspace members via `toml = "0.8"`
- ✅ **Phase 12B — Configuration Files**: YAML (.yml/.yaml), JSON (.json), Java Properties (.properties) with recursive walk, depth limit 10, leaf-key granularity, lock file exclusions, 500KB file size limit. package.json special handling: npm deps as BuildDependency, scripts as ConfigProperty, ProjectIdentity emission
- ✅ **Phase 12C — Kubernetes + Helm**: 10 new EntityKind variants (K8sDeployment, K8sService, K8sConfigMap, K8sSecret, K8sIngress, K8sNamespace, K8sResource, HelmChart, HelmValue, HelmTemplateVar). K8s manifest parsing with label/annotation/reference extraction, Helm Chart.yaml/values.yaml/templates support with {{ .Values.X }} variable tracking
- ✅ **Phase 12D — Cross-Repo Dependency Linking**: Automatic inter-repository call resolution via `:Repository` graph model with `DEPENDS_ON` edges. `ProjectIdentity` marker entity from build files (Maven GAV, Cargo package, npm name). `knot deps` CLI subcommand + `list_repo_dependencies` MCP tool for dependency graph visualization. Retroactive linking for out-of-order indexing
- ✅ **74+ new unit tests** across 6 parser modules + cross-repo integration tests
- ✅ **11/11 E2E test suites pass**: JS/TS/Java, Kotlin, Rust, Python, Build Systems (extended), Config Files, K8s/Helm, Groovy, Cross-Language Ref, C/C++, Cross-Repo Dependencies
- ✅ **cargo fmt** clean | **cargo clippy** clean | **520+ unit tests** passing

### Previous Release (v1.1.0 — Performance Optimization) ✅
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

### Previous Release (v0.10.3 — Groovy Private Methods, Nested Closures & UUID Collision Fix) ✅
- ✅ **UUID Collision Fix**: `ParsedEntity` identity now includes `start_line`
- ✅ **Multi-line Method Extraction**: `try_extract_typed_method_multiline` handles closure default params
- ✅ **Innermost Assignment**: method calls in nested closures go to the innermost method
- ✅ **10 E2E test cases**: typed/`def`/no-paren callers, multi-line closures, innermost assignment
- ✅ **441 unit tests | clippy clean | fmt applied**

### Previous Release (v0.8.7 — Enhanced Rust Type Reference Detection in Macros) ✅
- ✅ **Macro Type Reference Extraction**: Type references inside macro invocations (`vec![]`, `println!()`, `assert!()`, `format!()`, etc.) are now correctly captured
- ✅ **Intelligent String Filtering**: Filters out false positives from string literals using quote-counting heuristics
- ✅ **Comprehensive Edge Case Handling**: Validates identifiers, handles nested macros, supports `macro_rules!` definitions
- ✅ **Improved Accuracy**: EntityKind references increased by +95.7% (46→90 references), now captures test function usage
- ✅ **Enhanced Test Coverage**: Added 4 new tests for token_tree extraction covering various macro types and edge cases

### Previous Release (v0.8.6 — Rust Type Aliases, Constants, and Docstrings) ✅
- ✅ **Rust Type Alias Extraction**: Extracts type alias declarations with full signature (e.g., `type Callback = fn(u32) -> u32`)
- ✅ **Rust Constant/Static Extraction**: Captures `const` and `static mut` declarations with type signatures
- ✅ **Rust Docstring Support**: Full doc comment extraction for Rust entities (handles nested `doc_comment` nodes in tree-sitter-rust)
- ✅ **Rich Vector Embeddings**: Type signatures and documentation are now included in embeddings for better semantic search
- ✅ **Improved Search Ranking**: Rust entities like `Callback` now rank in top 5 search results when querying by name

### Previous Release (v0.8.5 — Rust Module Refactoring & Clippy Fixes) ✅
- ✅ **Rust Module Refactoring**: Extracted Rust parsing logic into dedicated `src/pipeline/parser/languages/rust.rs` for better maintainability and mirroring existing language module architecture.
- ✅ **Clippy Compliance**: Fixed unused import (`uuid::Uuid`) and unnecessary `mut` warning in Rust module tests.
- ✅ **Rust Support Complete**: Phase 8 implementation fully integrated with 17 unit tests and 22 E2E test cases passing.

### Previous Release (v0.8.4 — Agent-Skills Documentation Installer & Lightweight Clients) ✅
- ✅ **Dry-Run Mode**: MCP server can run in offline mode for quality checks on deployment platforms.
- ✅ **Platform-Agnostic**: Removed all platform-specific references; compatible with any deployment platform.
- ✅ **Enhanced Reliability**: Graceful handling of missing database connections for validation scenarios.

### Previous Release (v0.10.0 — Build Systems & CI/CD Support) ✅
- ✅ **Build Systems Support (Phase 9)**: Maven `pom.xml` (dependencies + plugins via roxmltree), Gradle `build.gradle` (deps + plugins + tasks), and Jenkinsfile pipeline (stages + steps) extraction
- ✅ **22 unit tests + 8 E2E tests** (Maven search, pom.xml explore, Gradle dep/task search, Jenkins stage/step search)
- ✅ **BuildDependency, BuildPlugin, BuildTask, PipelineStage, PipelineStep entity kinds** with explore_file formatting

### Previous Release (v0.9.3 — Python Search Stability & CI Fixes) ✅
- ✅ Fixed CLI `explore` & `search` queries that queried the default collection instead of test collection by appending `-r "$REPO_NAME"`
- ✅ Python CLI search bug handled; resolved `knot search` queries failing in specific collection bounds
- ✅ Replaced unreliable `nc -z` network checks with Neo4j-specific Docker health checks (`docker inspect`)
- ✅ 426 unit tests | 23 Python E2E | 22 Rust E2E | 10 Kotlin E2E

### Earlier Release (v0.8.2 — Quality & Doc Refactor) ✅
- ✅ **MCP Quality**: Enhanced tool descriptions for better agent discovery and usage safety.
- ✅ **Token-Efficient Docs**: Modularized agent skill guide into `docs/agent-skills/` for on-demand loading.
- ✅ **Rust Phase 1**: Infrastructure prepared for Rust 2024 integration.
- ✅ **Rust Phase 2-5**: Complete Rust language support including entity extraction, macro tracking, and comprehensive E2E testing (v0.8.x).

### Earlier Release (v0.8.1 — CLI UX & Docker Integration) ✅
- ✅ **Silenced CLI Logs**: Default log level set to `error` for `knot` CLI (cleaner Markdown output).
- ✅ **100% E2E Dual-Testing**: All 35 integration tests simultaneously verify both MCP and CLI.
- ✅ **Docker CLI Support**: Official Docker image now includes the `knot` binary.
- ✅ **Agent Guidance**: Enhanced `.knot-agent.md` with signature-based search warnings.

### Phase 7 (v0.8.10 — CLI UX & Corporate Network Support) ✅
- ✅ **Human-friendly output formatting**: Colorized table output as default with per-entity-kind ANSI colors
- ✅ **Interactive result navigation**: Pager support via `less -R -e` with auto-exit at end of content
- ✅ **Configurable output formats**: `--output` flag supports `table` (default), `json`, and `markdown`
- ✅ **Custom CA Certificates**: `--custom-ca-certs` / `KNOT_CUSTOM_CA_CERTS` for corporate SSL-inspecting proxies
- ✅ **O(N) Macro Traversal Optimization** (v0.8.11): Substring skipping for deeply nested `token_tree` nodes

### Phase 8 (v0.8.11 — Rust Support) ✅
- ✅ Support `.rs` files with tree-sitter-rust parser
- ✅ Struct, enum, union, trait, and impl block extraction
- ✅ Function, method, macro definition and invocation tracking
- ✅ Type alias, constant, static, and module extraction with signatures
- ✅ Docstring extraction for all Rust entity types
- ✅ O(N) nested macro traversal optimization for large Rust codebases
- ✅ 17 unit tests for Rust entity and reference extraction
- ✅ 22 end-to-end integration tests covering all Rust language constructs

### Phase 11 (v1.0.0 — C/C++ Support) ✅
- ✅ Support `.c`, `.cpp`, `.cc`, `.cxx`, `.h`, `.hpp`, `.hh`, `.hxx` files via tree-sitter-c and tree-sitter-cpp
- ✅ Intelligent auto-detection of `.h` files to parse them as C++ if they contain classes, namespaces, or templates
- ✅ Namespace-aware FQN resolution (`Engine::MyClass::start`)
- ✅ Class, struct, function, and method extraction with full signatures
- ✅ Macro definition and usage tracking (uppercase identifier heuristic)
- ✅ Type reference tracking (declarations, `new` expressions, qualified types)
- ✅ Call graph analysis including method calls, field access (`obj->method()`), and scope resolution (`std::vector::size()`)
- ✅ 3 unit tests for C++ entity and reference extraction
- ✅ 4 end-to-end integration tests covering FQN, call graphs, macro usage, and type references

### Upcoming

#### Long-Term Vision
- [ ] Build Server/Client architecture to facilitate enterprise usage with multiple MCP clients
- [ ] Reestructure E2E test suites to run all together at once sharing databases context avoiding Docker overhead
- [ ] Markdown documentation indexing
- [ ] Go support
- [ ] C# support
- [ ] IDE plugins (VS Code, IntelliJ, Vim)
- [ ] Web UI for graph visualization
- [ ] Language Server Protocol (LSP) integration
- [ ] Automated Code Review tool (MCP-based)

---

## 💬 Questions?

For issues, feature requests, or discussions, please open a GitHub issue.
