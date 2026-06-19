# knot

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-2024-brightgreen.svg)](https://www.rust-lang.org)
[![knot MCP server](https://glama.ai/mcp/servers/raultov/knot/badges/score.svg)](https://glama.ai/mcp/servers/raultov/knot)

<div align="center">
  <a href="https://glama.ai/mcp/servers/raultov/knot">
    <img src="https://glama.ai/mcp/servers/raultov/knot/badges/card.svg" alt="knot MCP server" />
  </a>
</div>

**knot** is a high-performance codebase indexer that extracts structural and semantic information from source code, enabling AI agents to understand, analyze, and navigate large code repositories. Currently supports Java, Kotlin, TypeScript, JavaScript/Node.js, Rust, Python, **Groovy**, **C/C++**, HTML, and CSS/SCSS, plus **Build Systems** (Maven pom.xml, Gradle build.gradle, Jenkins pipeline, **Cargo.toml**), **Configuration Files** (YAML, JSON, .properties — optional), **Kubernetes + Helm** (optional), and **Cross-Repo Dependency Linking** with full cross-language linking.

For recent release notes see [CHANGELOG.md](CHANGELOG.md).

The indexer automatically builds:
- **Vector Search Database** (Qdrant) — semantic understanding via embeddings
- **Graph Database** (Neo4j) — architectural relationships via call graphs

This dual-database approach powers both:
- **MCP (Model Context Protocol) Server** — Exposes three tools to any LLM client (Claude, Gemini, ChatGPT, Cursor, etc.)
- **CLI Tool** — Standalone `knot` command for terminal and scripting environments

### Knot in action

<div align="center">

<img src="demo-cli.gif" alt="knot CLI demo" width="80%">

**CLI** — instant reverse dependency lookup

</div>

<div align="center">

<img src="demo-mcp.gif" alt="knot MCP demo" width="80%">

**MCP** — JSON-RPC protocol for AI agents

</div>

---

## ✨ Key Features

**🔍 Code Intelligence Tools**
- **`search_hybrid_context`**: Semantic + structural search. Find code by meaning, class name, method signature, docstrings, or comments. Returns full context including dependencies.
- **`find_callers`**: Reverse dependency lookup. Identify dead code, perform impact analysis, or understand the full call chain of any function/method. When multiple entities share the same name (e.g., `find_nearest_entity_by_line` in different files), results are automatically grouped by target showing which specific entity each caller references. Supports cross-repository call resolution via `DEPENDS_ON` graph edges.
- **`explore_file`**: File anatomy inspection. Quickly see all classes, interfaces, methods, and functions in a file with signatures and documentation.
- **`list_repo_dependencies`** (MCP) / **`knot deps`** (CLI): Dependency graph visualization. Show which repositories depend on each other, forward and reverse, with transitive resolution.
- **`list_repositories`** / **`knot repos`**: Repository inventory. List every indexed repository along with its entity count, file count, build system, and primary language. Useful for orientation, sanity-checking indexing runs, and discovering which languages and build systems are present in the workspace.

**🏗️ Multi-Language Support**
- **Java**: Full AST extraction with package-aware FQN resolution (e.g., `com.example.app.UserService`), class inheritance (`EXTENDS`), interface implementation (`IMPLEMENTS`), annotation tracking, and field-access method invocation resolution
- **Kotlin**: Complete support for Kotlin codebases with classes, interfaces, objects, companion objects, functions, methods, and properties. Fully compatible with tree-sitter-kotlin-ng grammar.
- **TypeScript/TSX/CTS**: Complete support for modern JavaScript/TypeScript codebases, including CommonJS TypeScript files
- **JavaScript/Node.js**: Vanilla JS, Node.js, and module systems (`.js`, `.mjs`, `.cjs`, `.jsx`)
- **Hybrid Web Ecosystem**: Cross-language linking between JavaScript, HTML, and CSS for full-stack SPA analysis
- **HTML**: Custom elements (Web Components, Angular), `id` and `class` attribute indexing for cross-language CSS search
- **JSX/TSX Attributes**: Extracts `id` and `className` from React components for unified HTML/CSS discovery
- **CSS/SCSS**: Stylesheet indexing with class/ID selector extraction and variable tracking (CSS/SCSS variables, mixins, functions)
- **Rust**: Struct, enum, union, trait, function, method, module extraction with trait implementation tracking (IMPLEMENTS relationships) and macro invocation references. Methods are indexed with the qualified FQN `Type::method` (e.g., `KnotMcpHandler::new`, `WidgetA::new`, `Logger::new`) and qualified calls from top-level functions resolve to the right target by receiver. Braced import/use capture — `use foo::{Bar, Baz}` and `use foo::Bar as Baz` produce explicit REFERENCES edges for all imported names, including traits imported solely to bring methods into scope. All Rust entity FQNs are now anchored at the owning crate and module path (e.g. `knot::config::Config`, `knot::pipeline::parser::languages::rust::qualify_rust_fqns`), so two crates that declare a type with the same bare name no longer collide. Files outside `src/` (tests, benches, examples) receive a `__fixture::<path>::<Entity>` FQN prefix (e.g. `__fixture::tests::testing_files::sample::Config`), and files without a `Cargo.toml` ancestor receive `__loose::<path>::<Entity>`, preventing name collisions with real source entities. CONTAINS relationships use `enclosing_class_fqn` for exact disambiguation when multiple entities share the same class name. The on-disk index state file (`.knot/index_state.json`) carries a `version` field; opening a state file from an older version prints an error with instructions to run `knot-indexer --clean`.
- **Python**: Full Python extraction with class, function, method support, constants, module-level imports, `ValueReference` tracking for keyword arguments, class inheritance (`EXTENDS`), decorator extraction (`@property`, `@staticmethod`, `@route(...)`, `@dataclass`), generic type hints (`List[str]`, `Optional[Dict]`, `*args`/`**kwargs`), Py2/Py3 exception syntax compatibility, and `self.method()` resolution with inherited method walking. Captures `class_definition`, `function_definition` (including async via optional `async` modifier), lambda assignments, and distinguishes methods from functions via parent context detection.
- **Groovy**: Full Groovy language support via hybrid tree-sitter + ad-hoc lexical parser. Extracts classes, interfaces, traits, enums, typed/`def`/quoted methods (incl. Spock specs), constructors, closures, script-level variables, fields/properties with visibility modifiers, nested classes, and decorators. Tracks package FQN and enclosing class relationships. Multi-line signatures (closure default params), assignment-vs-declaration disambiguation, innermost assignment for nested closures, UUID collision fix for duplicate method names, `find_callers` accurately tracks private methods including those in anonymous `new AnAction` closures.
- **Build Systems**: Maven `pom.xml` (dependencies + plugins via roxmltree), Gradle `build.gradle` (deps + plugins + tasks), and `Jenkinsfile` pipeline (stages + steps) extraction.
- **Cargo.toml**: Rust package manager support with package metadata, features, workspace members, and multi-format dependency parsing (simple, table, git, path).
- **Configuration Files**: YAML (.yml/.yaml), JSON (.json), and Java Properties (.properties) with leaf-key granularity. Special handling for package.json (npm dependencies as BuildDependency, scripts as ConfigProperty).
- **Kubernetes + Helm**: K8s manifest parsing (Deployment, Service, ConfigMap, Secret, Ingress, Namespace) with label/annotation tracking and cross-resource references. Helm chart indexing (Chart.yaml metadata, values.yaml key-value pairs, template variable extraction via {{ .Values.X }}).
- **C/C++**: Complete C/C++ support with namespace-aware FQN resolution (`Engine::MyClass::start`), class/struct extraction, function/method tracking, macro definition and usage detection (uppercase identifier heuristic), type reference tracking (declarations, `new` expressions), and full call graph analysis. Supports `.c`, `.h`, `.cpp`, `.hpp`, `.cc`, `.cxx`, `.hh`, `.hxx` extensions via tree-sitter-c and tree-sitter-cpp parsers. Includes intelligent auto-detection for `.h` headers to parse them correctly as C or C++ based on their contents.
- **Markdown**: Documentation indexing with `MarkdownDocument` (one per `.md`/`.markdown` file) and `MarkdownSection` (one per ATX heading H1–H6). Section bodies — including paragraphs, fenced code blocks, lists, and tables — are captured into `embed_text` for full semantic search over documentation content, not just heading titles. FQNs are hierarchical and file-scoped (e.g. `README.md::Setup > Installation > Linux`), so same-named headings in different files or under different parents disambiguate cleanly. Section boundaries respect heading depth: a section's body extends until the next heading of equal or higher level, ensuring `### Linux` under `## Installation` does not bleed into a sibling `## Configuration`. Headings with inline markdown (backticks, em-dash, links, emoji) parse without losing their bodies, and real `start_line`/`end_line` positions are computed via tree-sitter for each section.

**📚 Rich Comment Extraction**
- Captures docstrings (JavaDoc, JSDoc) preceding declarations
- Extracts inline comments within method/function bodies
- Respects nesting boundaries (class comments don't capture method comments)
- Intelligently aggregates comment blocks

**📊 Dual-Database Architecture**
- **Qdrant**: Vector search for semantic code understanding
- **Neo4j**: Graph relationships for structural navigation

**🚀 High Performance**
- **Parallel Streaming Pipeline**: Overlaps CPU-bound embedding with I/O-bound ingestion via MPSC channels
- **Incremental Indexing**: Uses SHA-256 hashes to skip unchanged files
- **Real-time Watch Mode**: Automatically re-indexes changed files in seconds via `--watch`
- **CPU Parallelism**: AST extraction via Rayon
- **Scalable**: Configurable batch processing and constant memory footprint (~2GB) regardless of repository size
- **Performance Benchmarking**: Three-level validation framework
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

**Install agent-skills for your AI (Optional):**
Paste this into your LLM agent (Claude Code, OpenCode, Cursor, etc.):
> Install the knot agent skills by following the instructions at: https://raw.githubusercontent.com/raultov/knot/master/README.md

The first command installs the `knot` binary to your PATH. The second (optional) allows your AI assistant to automatically download the agent skill index (`.knot-agent.md`) and run the installer to extract comprehensive guides for using knot CLI with AI agents and code analysis tools.

**System Requirements:**
- **Linux**: glibc 2.38+ (Ubuntu 24.04+, Debian 13+, Fedora 39+, Arch)
- **macOS**: Modern versions supported
- **Windows**: Use Docker (Option B)

### Option B: Docker (Universal Compatibility)

Docker images provide universal compatibility for **any Linux distribution** and **Windows**.

#### Docker Installation (All Binaries)

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
mkdir -p ~/.config/knot
cp .env.example ~/.config/knot/.env
$EDITOR ~/.config/knot/.env  # Set KNOT_REPO_PATH and Neo4j credentials
```

**4. Index a codebase:**
```bash
./target/release/knot-indexer
```

**5. Query via CLI:**
```bash
./target/release/knot search "your query"
```

**6. Start the MCP server:**
```bash
./target/release/knot-mcp
```

---

## 📖 Usage

### 🤖 Install Agent Skills (For AI Agents)

**Option A: Let an LLM do it**

Paste this into any LLM agent (Claude Code, OpenCode, Cursor, etc.):

> Install the knot agent skills by following the instructions at: https://raw.githubusercontent.com/raultov/knot/master/README.md

**Option B: Terminal (Manual)**

```bash
curl -sO https://raw.githubusercontent.com/raultov/knot/master/.knot-agent.md && curl -fsSL https://raw.githubusercontent.com/raultov/knot/master/scripts/install-agent-skills.sh | bash
```

### 📥 Quick Downloads (Binaries)

**Download knot binaries (CLI + MCP server):**
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/raultov/knot/releases/latest/download/knot-installer.sh | sh
```

### 📖 Agent-Skills Guides

Comprehensive documentation for using knot tools. The agent skills installer extracts:
- **search.md** — Semantic code discovery guide with examples
- **callers.md** — Reverse dependency lookup with critical usage rules
- **explore.md** — File anatomy inspection guide
- **deps.md** — Repository dependency graph guide
- **repos.md** — Indexed repository inventory
- **workflows.md** — Common patterns and best practices

For quick reference without downloading, see [`.knot-agent.md`](.knot-agent.md).

---

### Using the CLI

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

#### `knot deps` — Repository Dependency Graph
```bash
knot deps my-app --depth 2           # Show forward dependencies (transitive)
knot deps my-app --reverse           # Show who depends on this repo
```
Visualize auto-discovered dependencies between indexed repositories with transitive resolution up to 3 levels deep.

#### `knot repos` — List Indexed Repositories
```bash
knot repos                          # Table with REPO / BUILD SYSTEM / LANGUAGE / FILES / ENTITIES
knot repos --output json            # Machine-readable list
knot repos --output markdown        # GFM table for chat UIs
```
Show the status of every repository currently indexed in the graph database — useful for orientation, sanity-checking that an indexing run completed, and discovering which languages and build systems are present across the workspace.

**For detailed CLI usage guide**, see [`.knot-agent.md`](.knot-agent.md) — a machine-readable skill that teaches LLMs how to use knot CLI for autonomous code analysis.

### Indexing a Codebase

#### Incremental Indexing (Default)

```bash
# First run: indexes all files
knot-indexer --repo-path /path/to/your/repo --neo4j-password secret

# Subsequent runs: only re-indexes changed files (fast!)
knot-indexer --repo-path /path/to/your/repo --neo4j-password secret

# NEW: Real-time Watch mode
knot-indexer --watch --repo-path /path/to/your/repo --neo4j-password secret
```

**How it works:**
- Tracks file content via SHA-256 hashes in `.knot/index_state.json`
- Stores the downloaded `fastembed` model in `.knot/fastembed_cache/` to keep the workspace clean
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
# Run all language E2E tests (TypeScript, Java, JavaScript, Web, Kotlin, Rust, ...)
./tests/run_all_e2e_fast.sh

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

**Advanced: Search by Signature**
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

All options can be set via CLI flags, environment variables, or a `~/.config/knot/.env` file.
Priority (highest to lowest): CLI flags > environment variables > `.env` file.

| Env Variable               | CLI Flag                   | Default                     | Description                                              |
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
| `KNOT_INCLUDE_CONFIG_FILES` | `--include-config-files`  | `false`                     | Include YAML/JSON/properties/K8s/Helm files in the index |
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
echo "KNOT_CUSTOM_CA_CERTS=/etc/ssl/certs/corporate-bundle.pem" >> ~/.config/knot/.env
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

For the full release history see [CHANGELOG.md](CHANGELOG.md).

### Upcoming

#### Long-Term Vision
- [ ] Homogenize all E2E test suites to use the per-suite fixture directory architecture (`E2E_DATA_DIR/docker-compose.yml`) already adopted by `run_cpp_e2e.sh`, for better isolation in standalone mode and parallel-safe execution
- [ ] Run the `test-unit` gate also on push to master (currently only runs on tag push via `release.yml`) so unit-test regressions are caught at merge time, not at release time
- [ ] Varnish VCL support
- [ ] Go support
- [ ] C# support
- [ ] IDE plugins (VS Code, IntelliJ, Vim)
- [ ] Language Server Protocol (LSP) integration
- [ ] Automated Code Review tool (MCP-based)
- [ ] CLI commands (opencode, claude, agy) to index repos
- [ ] Ruby support

---

## 💬 Questions?

For issues, feature requests, or discussions, please open a GitHub issue.
