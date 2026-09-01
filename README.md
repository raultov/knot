# knot

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-2024-brightgreen.svg)](https://www.rust-lang.org)
[![knot MCP server](https://glama.ai/mcp/servers/raultov/knot/badges/score.svg)](https://glama.ai/mcp/servers/raultov/knot)

<div align="center">
  <a href="https://glama.ai/mcp/servers/raultov/knot">
    <img src="https://glama.ai/mcp/servers/raultov/knot/badges/card.svg" alt="knot MCP server" />
  </a>
</div>

**knot** is a high-performance codebase indexer that extracts structural and semantic information from source code, enabling AI agents to understand, analyze, and navigate large code repositories. Currently supports Java, Kotlin, TypeScript, JavaScript/Node.js, Rust, Python, **Groovy**, **C/C++**, **C#**, HTML, and CSS/SCSS, plus **Build Systems** (Maven pom.xml, Gradle build.gradle, Jenkins pipeline, **Cargo.toml**, **MSBuild .csproj + Directory.Packages.props**), **Configuration Files** (YAML, JSON, .properties — optional), **Kubernetes + Helm** (optional), and **Cross-Repo Dependency Linking** with full cross-language linking.

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

## 🧮 Token Efficiency — Measured, Not Claimed

An LLM agent exploring an unfamiliar codebase pays for every byte it reads.
Without an index it greps and then reads whole files; with knot it receives a
targeted answer. The difference was measured on **three real indexed
repositories** across nine realistic exploration tasks:

| Repo | Lang | Task | knot tokens | Read-the-code tokens | Reduction |
|------|------|------|------------:|---------------------:|----------:|
| spring-ai | Java | discovery — *how does the chat client run the advisor chain?* | 1 092 | 10 168 | **89.3%** |
| spring-ai | Java | callers — *who uses `ToolCallingManager`?* | 8 808 | 15 554 | **43.4%** |
| spring-ai | Java | explore — *structure of `DefaultChatClient.java`* | 4 865 | 7 838 | **37.9%** |
| puppeteer | TypeScript | discovery — *how is a CDP session created?* | 609 | 4 149 | **85.3%** |
| puppeteer | TypeScript | callers — *who calls `createCDPSession`?* | 1 004 | 39 878 | **97.5%** |
| puppeteer | TypeScript | explore — *structure of the `Page` API* | 7 287 | 25 300 | **71.2%** |
| knot | Rust | discovery — *how are call intents resolved?* | 594 | 14 824 | **96.0%** |
| knot | Rust | callers — *who calls `format_references_result`?* | 461 | 10 949 | **95.8%** |
| knot | Rust | explore — *structure of the graph query module* | 978 | 12 103 | **91.9%** |
| **TOTAL** | — | **9 tasks** | **25 698** | **140 763** | **81.7%** |

**≈ 5.5× fewer tokens** for the same nine questions — 115 000 tokens saved,
enough to keep a long refactoring session inside a single context window.

<details>
<summary><b>Methodology (and how to reproduce it)</b></summary>

Both sides are measured on the **exact bytes an LLM would receive as tool
output**, counted with OpenAI's `cl100k_base` tokenizer (tiktoken):

| Task | knot side | Read-the-code side |
|------|-----------|--------------------|
| `discovery` | `knot search "<question>" --repo <r> --output markdown` | `rg -l <keyword>` (candidate list) **+ full read of the files that actually answer the question** |
| `callers` | `knot callers "<symbol>" --repo <r> --output markdown` | `rg -n "\b<symbol>\b"` **+ full read of the first 5 distinct files with hits** |
| `explore` | `knot explore "<file>" --repo <r> --output markdown` | full read of the file |

The baseline is deliberately **generous**, so the measured saving is a lower
bound:

- greps are restricted to the source files of the language (`-t java`, `-t ts`,
  `-t rust`) — no changelogs, no generated docs, no `node_modules`;
- for `discovery` the baseline is given *oracle file selection*: it reads only
  the files that answer the question, with zero wasted reads;
- for `callers` it reads at most 5 files, while a rigorous impact analysis would
  need every file with a textual hit.

Honest caveats: knot's cost scales with the **number of results**, not with repo
size. The weakest row (`spring-ai` / `ToolCallingManager`, 43%) is a symbol with
156 references — knot enumerates all of them with exact call sites, while the
capped baseline reads only 5 files and still cannot tell a call from a comment.
The `explore` rows for large classes are also the least favourable, because
signatures plus docstrings are a large fraction of a well-documented file.

Repositories measured (as indexed): `spring-ai` 2 406 files / 25 733 entities,
`puppeteer` 1 832 files / 19 310 entities, `knot` 222 files / 4 000 entities.
Raw measurements are stored in
[`.perf_metrics/token_savings.json`](.perf_metrics/token_savings.json).

```bash
pip install tiktoken            # optional: falls back to a chars/4 estimate
# edit the `root` paths in scripts/token_savings_tasks.json to match your checkouts
python3 scripts/token_savings_benchmark.py \
  --config scripts/token_savings_tasks.json \
  --save-json .perf_metrics/token_savings.json
```

The task definitions live in
[`scripts/token_savings_tasks.json`](scripts/token_savings_tasks.json) and the
harness in
[`scripts/token_savings_benchmark.py`](scripts/token_savings_benchmark.py);
point them at any repository you have indexed to measure your own codebase.

</details>

---

## ✨ Key Features

**🔍 Code Intelligence Tools**
- **`search_hybrid_context`**: Semantic + structural search. Find code by meaning, class name, method signature, docstrings, or comments. Returns full context including dependencies.
- **`find_callers`**: Reverse dependency lookup. Identify dead code, perform impact analysis, or understand the full call chain of any function/method. When multiple entities share the same name (e.g., `find_nearest_entity_by_line` in different files), results are automatically grouped by target showing which specific entity each caller references. Supports cross-repository call resolution via `DEPENDS_ON` graph edges. For JVM languages (Java/Kotlin/Groovy) it also surfaces method-level `OVERRIDES` edges bidirectionally — an **Overridden by** group listing subtype implementations/overrides and an **Overrides** group listing the supertype methods a method implements/overrides.
- **`explore_file`**: File anatomy inspection. Quickly see all classes, interfaces, methods, and functions in a file with signatures and documentation.
- **`list_repo_dependencies`** (MCP) / **`knot deps`** (CLI): Dependency graph visualization. Show which repositories depend on each other, forward and reverse, with transitive resolution.
- **`list_repositories`** / **`knot repos`**: Repository inventory. List every indexed repository along with its entity count, file count, build system, and primary language. Supports optional case-insensitive name filtering via `--filter` (CLI) or `filter` parameter (MCP). Useful for orientation, sanity-checking indexing runs, and discovering which languages and build systems are present in the workspace.

**🏗️ Multi-Language Support**
- **Java**: Full AST extraction with package-aware FQN resolution (e.g., `com.example.app.UserService`), class inheritance (`EXTENDS`), interface implementation (`IMPLEMENTS`), annotation tracking, and field-access method invocation resolution
- **Kotlin**: Complete support for Kotlin codebases with classes, interfaces, objects, companion objects, functions, methods, and properties. Fully compatible with tree-sitter-kotlin-ng grammar.
- **C#**: Full C# support via `tree-sitter-c-sharp`. Extracts classes, interfaces, structs, records (both `record class` and `record struct`), enums, methods, constructors, properties, fields (with `const` detection), delegates, events, indexers, operators, local functions, and namespaces with `CSharp*` entity kinds. Namespace-qualified FQNs (`MyApp.Services.UserService.GetUserAsync`) work across both file-scoped (C# 10+) and block-form namespaces, including nested namespaces and nested types. The `base_list` heuristic splits `: Base, IFace` into `EXTENDS`/`IMPLEMENTS` using the `IPascalCase` convention (structs and interfaces are deterministic), generic arguments are stripped (`IRepository<User>` → `IRepository`), XML doc comments (`///`) become docstrings, and attributes (`[Obsolete]`) are captured as decorators. Calls through field-typed receivers resolve to the exact implementation method, and C# `virtual`/`override` plus interface implementation produce method-level `OVERRIDES` edges. **MSBuild/NuGet**: `.csproj` files are parsed for project identity and dependencies (Central Package Management via `Directory.Packages.props` is supported); C# repos get `build_system: "nuget"` in the Repository node instead of the prior `"none"`.
- **TypeScript/TSX/CTS**: Complete support for modern JavaScript/TypeScript codebases, including CommonJS TypeScript files
- **JavaScript/Node.js**: Vanilla JS, Node.js, and module systems (`.js`, `.mjs`, `.cjs`, `.jsx`)
- **Hybrid Web Ecosystem**: Cross-language linking between JavaScript, HTML, and CSS for full-stack SPA analysis
- **HTML**: Custom elements (Web Components, Angular), `id` and `class` attribute indexing for cross-language CSS search
- **JSX/TSX Attributes**: Extracts `id` and `className` from React components for unified HTML/CSS discovery
- **CSS/SCSS**: Stylesheet indexing with class/ID selector extraction and variable tracking (CSS/SCSS variables, mixins, functions)
- **Rust**: Struct, enum, union, trait, function, method, module extraction with trait implementation tracking (IMPLEMENTS relationships) and macro invocation references. Methods are indexed with the qualified FQN `Type::method` (e.g., `KnotMcpHandler::new`, `WidgetA::new`, `Logger::new`) and qualified calls from top-level functions resolve to the right target by receiver. Braced import/use capture — `use foo::{Bar, Baz}` and `use foo::Bar as Baz` produce explicit REFERENCES edges for all imported names, including traits imported solely to bring methods into scope. All Rust entity FQNs are now anchored at the owning crate and module path (e.g. `knot::config::Config`, `knot::pipeline::parser::languages::rust::qualify_rust_fqns`), so two crates that declare a type with the same bare name no longer collide. Files outside `src/` (tests, benches, examples) receive a `__fixture::<path>::<Entity>` FQN prefix (e.g. `__fixture::tests::testing_files::sample::Config`), and files without a `Cargo.toml` ancestor receive `__loose::<path>::<Entity>`, preventing name collisions with real source entities. CONTAINS relationships use `enclosing_class_fqn` for exact disambiguation when multiple entities share the same class name. The on-disk index state file (`.knot/index_state.json`) carries a `version` field; opening a state file from an older version prints an error with instructions to run `knot-indexer --clean`.
 - **Python**: Full Python extraction with class, function, method support, constants, module-level imports, `ValueReference` tracking for keyword arguments, class inheritance (`EXTENDS`), decorator extraction (`@property`, `@staticmethod`, `@route(...)`, `@dataclass`), generic type hints (`List[str]`, `Optional[Dict]`, `*args`/`**kwargs`), Py2/Py3 exception syntax compatibility, and `self.method()` resolution with inherited method walking. Captures `class_definition`, `function_definition` (including async via optional `async` modifier), lambda assignments, and distinguishes methods from functions via parent context detection. **Class instantiation (`ClassName(...)`) is automatically redirected to `ClassName.__init__`** so `find_callers ClassName.__init__` lists every constructor call site (with fallback to inherited `__init__` via the extends chain); only class/struct kinds trigger the redirect — functions keep the legacy behavior.
- **Groovy**: Full Groovy language support via hybrid tree-sitter + ad-hoc lexical parser. Extracts classes, interfaces, traits, enums, typed/`def`/quoted methods (incl. Spock specs), constructors, closures, script-level variables, fields/properties with visibility modifiers, nested classes, and decorators. Tracks package FQN and enclosing class relationships. Multi-line signatures (closure default params), assignment-vs-declaration disambiguation, innermost assignment for nested closures, UUID collision fix for duplicate method names, `find_callers` accurately tracks private methods including those in anonymous `new AnAction` closures. **Inheritance tracking:** emits `EXTENDS`/`IMPLEMENTS` reference intents for `class`/`interface`/`trait`/`enum` headers (single-line and multi-line) so `find_callers` surfaces real nextflow-style hierarchies — qualified parents (e.g. `extends nextflow.plugin.BasePlugin`) and generic-argument stripping (e.g. `extends AbstractRepo<Order, Long> → extends AbstractRepo`) are supported, and generic bounds (`class Box<T extends Comparable>`) are correctly **not** promoted to inheritance edges. **Property accessors:** bare property declarations (`Path baseDir`, `boolean cacheable`) are now indexed as `GroovyProperty`, and compiler-generated `getX`/`setX`/`isX` accessors are synthesised as first-class method entities so `OVERRIDES` edges link Groovy properties to interface getter declarations. Comment-stripping prevents Javadoc continuation lines (`* The pipeline script name`) from producing phantom entities or corrupting scope tracking.
- **Build Systems**: Maven `pom.xml` (dependencies + plugins via roxmltree), Gradle `build.gradle` (deps + plugins + tasks), `Jenkinsfile` pipeline (stages + steps), Cargo `Cargo.toml` (deps + workspace members + features), and MSBuild `.csproj` / `Directory.Packages.props` extraction. MSBuild resolves project identity (`<PackageId>` → `<AssemblyName>` → file stem), emits a `BuildDependency` per `<PackageReference>` (attribute-form and version-less), and resolves Central Package Management versions from the nearest `Directory.Packages.props` ancestor. UTF-8 BOMs are tolerated defensively. Identity marker `identity: package_id` is carried in the signature when the project has an explicit `<PackageId>` so the cross-repo resolver prefers published packages over depth-tied unmarked candidates.
- **Cargo.toml**: Rust package manager support with package metadata, features, workspace members, and multi-format dependency parsing (simple, table, git, path).
- **Configuration Files**: YAML (.yml/.yaml), JSON (.json), and Java Properties (.properties) with leaf-key granularity. Special handling for package.json (npm dependencies as BuildDependency, scripts as ConfigProperty).
- **Varnish Cache**: Hand-written parsers for `.vcl` (configuration), `.vtc` (test cases), and `.vcc` (VMOD C source). VCL extracts backends, probes, ACLs, subroutines (custom + built-in with `vcl_*` names, including aggregator entities for multi-part built-ins), `import` directives (with `as` aliases and `from` paths), `include` edges, `unused` declarations, VMOD instantiations, and `req.backend_hint` assignments. VTC extracts `varnishtest`/`vtest` cases, servers, clients, varnish instances, logexpect blocks, barriers, and `-vcl+backend` synthesised backends (with `is_test_context`). VCC extracts `$Module`, `$Function`, `$Object`, `$Method`, `$Event`, `$Restrict`, ENUMs, and default parameters. References: `Calls`, `Extends`, `Implements`, `References` (with intents `VclSubCall`, `VclBackendRef`, `VclProbeRef`, `VclAclRef`, `VclInclude`, `VclVmodImport`, `VclUnusedRef`, `ValueReference`); relationships: `UsesBackend`, `UsesProbe`, `UsesAcl`, `Includes`, `ImportsVmod`, `DeclaredUnused`. The Fastly VCL dialect is detected and skipped (returns empty entities).
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
- **Performance Benchmarking**: Multi-level validation framework
  - *Unit benchmarks*: Criterion-based benchmarks for parse, embed, and graph write throughput (`benches/`)
  - *E2E benchmarks*: Full pipeline metrics capture with per-stage timing (`tests/benchmark_e2e.sh`)
  - *CI regression tracking*: Automated baseline comparison against tolerance thresholds (`scripts/compare_perf_metrics.sh`)
  - *Token efficiency*: LLM token cost of knot answers vs reading source files (`scripts/token_savings_benchmark.py`) — see [Token Efficiency](#-token-efficiency--measured-not-claimed)

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
knot search "user authentication" --max-results 20 --repo "app-a,app-b"  # Union across repos
knot search "user authentication" --max-results 20 --repo all              # All indexed repos ('all' or '*')
```
Find code entities by meaning, class names, docstrings, or comments.

#### `knot callers` — Reverse Dependency Lookup
```bash
knot callers "LoginService" --repo my-app
knot callers "LoginService" --repo "auth-service,billing-service"
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
knot repos --filter app             # Case-insensitive name filter (substring match)
knot repos --output json            # Machine-readable list
knot repos --output markdown        # GFM table for chat UIs
```
Show the status of every repository currently indexed in the graph database — useful for orientation, sanity-checking that an indexing run completed, and discovering which languages and build systems are present across the workspace. Use `--filter` to quickly locate a specific repository when working with multiple indexed codebases.

**Repository Scope Selection:**
Both the CLI `--repo/-r` flag and MCP `repo_name` parameter support:
- Single repository name: `--repo my-app`
- Comma-separated list: `--repo "repo-a,repo-b"` (MCP also accepts `["repo-a", "repo-b"]`)
- Sentinel: `--repo all` or `--repo "*"` (searches every indexed repository)

*Note:* Multi-repo scope applies a global `max_results` limit across the union. Increase `--max-results` when searching across multiple repositories.

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

> **Upgrade note (v1.5.1):** File paths are now persisted as **repo-relative**
> paths with POSIX separators (e.g. `src/pipeline/embed.rs`). Upgrading from
> v1.4.x triggers an automatic full re-index on first run — the on-disk
> `.knot/index_state.json` carries a version field that the loader rejects
> when stale, and `knot-indexer` then wipes the repo from both databases
> before rebuilding. No manual steps required. Entity UUIDs become
> machine-independent in the process: the same repo indexed from different
> checkout locations now produces identical UUIDs.

### Indexing Progress

The indexer emits `[Progress]` log lines showing real-time completion across
the whole pipeline (parsing, embedding, ingestion, reference resolution).
The percentage is **monotonically non-decreasing** and reaches `100%` only
once the run genuinely terminates.

> **Upgrade note (v1.6.2):** The percentage now spans the entire pipeline via
> weighted bands. Previously it measured only file reading and saturated at
> `100%` within seconds of starting, then froze for minutes while embedding
> and ingestion were still running. See
> [`docs/specs/indexing_progress_accuracy_plan.md`](docs/specs/indexing_progress_accuracy_plan.md)
> for the full design.

Example with 5000 files where 1000 have been parsed and 5,000 entities are
half-way through ingestion:

```
[Progress] [my-repo] 50.0% — files 5000/5000, entities 41600/83200, batch #325 (128 entities)
```

#### Band table

| Phase | Band | Driver |
|---|---|---|
| `Idle` / `Discovering` / `Classifying` / `CleaningStaleData` | `0%` | — |
| Parsing | `0% → 10%` | `parsed_files / total_files` |
| Embedding + Ingestion | `10% → 90%` | `entities_ingested / total_entities` |
| `ResolvingReferences` | `95%` | fixed (no sub-counters available) |
| `Completed` | `100%` | forced |
| `Failed` | last computed value | frozen |

A final log line confirms completion:

```
[Progress] [my-repo] 100.0% — files 5000/5000, entities 83200/83200 — parsing and ingestion complete, resolving references...
```

#### Library API (knot-server integration)

Callers that need to observe progress programmatically can use the `ProgressTracker`:

```rust
use std::sync::Arc;
use knot::pipeline::{ProgressTracker, run_indexing_pipeline_with_progress};

let progress = Arc::new(ProgressTracker::new());
let progress_clone = Arc::clone(&progress);

// Poll snapshot() from another task while the pipeline runs
tokio::spawn(async move {
    loop {
        let snap = progress_clone.snapshot();
        println!(
            "{:.1}% — files {}/{}, entities {}/{}",
            snap.percent_complete,
            snap.parsed_files,
            snap.total_files,
            snap.entities_ingested,
            snap.total_entities
        );
        if snap.stage == IndexingStage::Completed || snap.stage == IndexingStage::Failed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
});

run_indexing_pipeline_with_progress(&cfg, &vdb, &gdb, &mut state, progress).await?;
```

The `snapshot()` method is thread-safe (read-only locks + atomic loads) and returns a
`IndexingProgress` struct that serializes directly to JSON for REST endpoints.

### Running E2E Integration Tests

To ensure indexer stability, run the E2E integration test suite:

```bash
# Run all language E2E tests (TypeScript, Java, JavaScript, Web, Kotlin, Rust, ...)
./tests/run_all_e2e_fast.sh

# Run only Kotlin E2E tests
./tests/run_kotlin_e2e.sh

# Run only Rust E2E tests
./tests/run_rust_e2e.sh

# Run only C# E2E tests
./tests/run_csharp_e2e.sh

# Run only Varnish E2E tests
./tests/run_varnish_e2e.sh
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

# Find by C# interface method (surfaces implementations + call sites)
echo '{"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"FindByIdAsync"}}}' | knot-mcp
```

**Use Cases:**
- **Dead Code Detection**: Zero callers = unused code
- **Impact Analysis**: "What breaks if I modify this?"
- **Refactoring Safety**: Find all references before removing
- **Override Discovery (JVM + C#)**: For Java/Kotlin/Groovy/C# methods, results include an
  **Overridden by** group (implementations/overrides in subtypes) and an
  **Overrides** group (the supertype methods a method implements/overrides). These
  are backed by real `OVERRIDES` edges built at index time and resolved
  transitively at query time, so querying an interface/superclass method surfaces
  every implementation, and querying an implementation surfaces the declaration it
  overrides.

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
| `KNOT_BATCH_SIZE`          | `--batch-size`             | `128`                       | Entities per batch                                       |
| `KNOT_CLEAN`               | `--clean`                  | `false`                     | Force full re-index (delete all existing data)           |
| `KNOT_CUSTOM_CA_CERTS`     | `--custom-ca-certs`       | *(none)*                    | Path to CA certificate bundle for corporate SSL proxies  |
| `KNOT_INCLUDE_CONFIG_FILES` | `--include-config-files`  | `false`                     | Include YAML/JSON/properties/K8s/Helm files in the index |
| `RUST_LOG`                 | *(env only)*              | `info`                      | Log level: `trace`, `debug`, `info`, `warn`, `error`     |

---

## 🎨 Custom Tree-sitter Queries

The built-in extraction queries (`queries/java.scm`, `queries/typescript.scm`, `queries/csharp.scm`) can be overridden without recompiling:

```bash
KNOT_CUSTOM_QUERIES_PATH=/path/to/my/queries ./target/release/knot-indexer
```

Place `java.scm`, `typescript.scm`, and/or `csharp.scm` in your custom directory. Missing files fall back to built-in defaults.

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
- All code passes `cargo clippy` and `cargo fmt`
- No new `unsafe` code (`unsafe_code = "deny"` at crate level; one audited exception in `src/utils/mod.rs` for corporate proxy CA bundle injection, documented via `#[expect(unsafe_code, reason = "…")]`)
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

**Level 3 — Token Efficiency Benchmark:**
```bash
# Measures knot tool output vs grep + file reads on indexed repositories
python3 scripts/token_savings_benchmark.py \
  --config scripts/token_savings_tasks.json \
  --save-json .perf_metrics/token_savings.json
```

Unlike levels 1 and 2 (which measure indexing throughput), this one measures the
*consumer* side: how many LLM tokens an agent spends to answer a question with
knot versus by reading source files. Requires `rg`, a built `knot` binary, the
repositories in the config already indexed, and optionally `tiktoken` for exact
token counts. See [Token Efficiency](#-token-efficiency--measured-not-claimed)
for the published results.

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
- [ ] Go support
- [ ] IDE plugins (VS Code, IntelliJ, Vim)
- [ ] Language Server Protocol (LSP) integration
- [ ] Automated Code Review tool (MCP-based)
- [ ] Ruby support

---

## 💬 Questions?

For issues, feature requests, or discussions, please open a GitHub issue.
