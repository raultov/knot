# AGENTS.md — Knot Development Guidance

**knot** is a dual-database codebase indexer (Rust 2024 edition) that powers semantic code search via MCP. This file captures repo-specific guidance that would trip up an unfamiliar agent.

---

## Quick Commands

**Build everything:**
```bash
cargo build --release
```

**Run unit tests only:**
```bash
cargo test
```

**Run all E2E tests** (requires Docker + databases):
```bash
./tests/run_all_e2e_fast.sh
```

**Run a single E2E language suite:**
```bash
./tests/run_typescript_e2e.sh       # TypeScript
./tests/run_java_e2e.sh              # Java
./tests/run_javascript_e2e.sh        # JavaScript
./tests/run_web_e2e.sh               # HTML + JSX + CSS + SCSS + hybrid
./tests/run_kotlin_e2e.sh            # Kotlin
./tests/run_rust_e2e.sh              # Rust
./tests/run_python_e2e.sh            # Python
./tests/run_build_systems_e2e.sh     # Maven/Gradle/Jenkins/Cargo.toml
./tests/run_groovy_e2e.sh            # Groovy
./tests/run_cpp_e2e.sh               # C/C++
./tests/run_cross_lang_ref_e2e.sh    # Cross-language validation
./tests/run_config_e2e.sh            # YAML/JSON/.properties config
./tests/run_k8s_helm_e2e.sh          # Kubernetes + Helm
./tests/run_cross_repo_dep_e2e.sh    # Cross-repo dependency linking
```

**Code quality:**
```bash
cargo clippy --all-targets -- -D warnings  # Must pass
cargo fmt -- --check                        # Must pass before commit
cargo fmt                                   # Auto-fix formatting
```

---

## Architecture Overview

### Three Binaries
| Binary | Purpose | Feature Flag |
|--------|---------|--------------|
| `knot-indexer` | Parses code, builds indexes (Qdrant + Neo4j) | `indexer` (default) |
| `knot` | CLI tool for search/explore/callers | always built |
| `knot-mcp` | MCP server for LLM integration | always built |

### Entrypoints
- `src/bin/knot-indexer.rs` — Main indexing pipeline
- `src/bin/knot.rs` — CLI tool (search/explore/callers/deps commands)
- `src/bin/knot-mcp.rs` — MCP server (exposes tools to LLMs)
- `src/lib.rs` — Shared library (modules: `pipeline`, `db`, `mcp_tools`, `cli_tools`, etc.)

### Language Parsers
Located in `src/pipeline/parser/languages/`:
- `java.rs`, `typescript.rs`, `javascript.rs` (via tree-sitter)
- `kotlin.rs` (tree-sitter-kotlin-ng v1.1.0)
- `rust.rs` (tree-sitter-rust v0.24) — Supports macros, type aliases, constants
- `python.rs` (v0.9.3) — Full async/decorator/type hint support
- `groovy.rs` (v0.10.3) — Hybrid tree-sitter + lexical parser
- `c_cpp.rs` (v1.0.0) — Namespace-aware FQN for C++
- `html.rs`, `css.rs` — Web stack support
- `toml.rs` (v1.2.5) — Cargo.toml parser (package, deps, features, workspace)
- `yaml.rs` (v1.2.5) — Generic YAML config parser (recursive walk, max depth 10)
- `json_config.rs` (v1.2.5) — JSON config + package.json special handling
- `properties.rs` (v1.2.5) — Java .properties line-by-line parser
- `kubernetes.rs` (v1.2.5) — K8s manifest parser (Deployment, Service, ConfigMap, etc.)
- `helm.rs` (v1.2.5) — Helm chart parser (Chart.yaml, values.yaml, templates)
- `xml.rs` — Maven pom.xml parser (extended with ProjectIdentity extraction)
- `groovy.rs` — Gradle build.gradle parser (extended with ProjectIdentity extraction)

Each module extracts `ParsedEntity` (name, kind, signature, docstring, FQN) and references (calls, extends, implements, references).

### Data Flow
```
Code Files (100+ languages)
    ↓
Tree-sitter Parsing (src/pipeline/parser/)
    ↓
Entity Extraction + Reference Graph Building
    ↓
Parallel Streams (MPSC channels for CPU/IO overlap)
    ├→ Qdrant (vector embeddings for semantic search)
    └→ Neo4j (graph relationships for call chains)
```

---

## Rust FQN Canonical Format

All Rust entity FQNs are anchored at the owning crate and module path:

```
<crate_name>::<module_path>::<EntityName>
<crate_name>::<module_path>::<Type>::<method>
```

- **Crate name**: `[package].name` from `Cargo.toml` (dashes → underscores).
- **Module path**: Derived from file path relative to `src/`:
  - `src/lib.rs`, `src/main.rs` → root (no suffix)
  - `src/foo.rs` → `foo`
  - `src/foo/mod.rs` → `foo`
  - `src/foo/bar.rs` → `foo::bar`
- **Fixtures** without `Cargo.toml`: `__fixture::<path>::Entity`
- **Loose files** without crate root: `__loose::<filename>::Entity`

Crate discovery runs before parsing and maps each `.rs` file to its nearest `Cargo.toml` ancestor. The `index_state.json` carries a `version` field; opening a v1 state file forces a full re-index automatically.

---

## Testing Strategy

### Philosophy
- **BDD**: E2E tests written **first**, must fail before logic implemented
- **TDD**: Unit tests in parser modules before implementation
- **E2E Regression**: Every bug must have E2E case before fix
- **No `unsafe`**: All code must be safe Rust

### Unit Tests
Located inline in source modules. Example: `src/pipeline/parser/languages/rust.rs#1234`.

```bash
cargo test                          # All unit tests
cargo test --lib pipeline::parser   # Specific module
cargo test -- --nocapture          # With stdout
```

### E2E Tests
Scripts in `tests/`:
- Test fixtures in `tests/testing_files/` (real code samples)
- Each script validates both MCP server and CLI tool identically
- Require `docker compose` with Qdrant + Neo4j running
- Use `docker-compose.e2e.yml` (ephemeral test databases)

**Critical**: E2E tests clean up Docker containers between suites (see `run_all_e2e_fast.sh` lines 36-42).

### Running Specific Test
```bash
# Single language (rebuilds binaries if needed)
./tests/run_rust_e2e.sh

# Subset of unit tests by regex
cargo test parser::languages::rust::tests::test_struct -- --nocapture

# With output
RUST_LOG=debug cargo test --lib parser::languages::rust -- --nocapture
```

---

## Repo-Specific Constraints & Quirks

### Rust Edition: 2024
This project requires Rust 1.90+ with unstable 2024 edition features. Standard 2021 features will not work.

```toml
# Cargo.toml
[package]
edition = "2024"
```

### Build Artifacts to Ignore
- `target/` — Compiled binaries, intermediate objects
- `.knot/` — Index state (`index_state.json` file for incremental indexing) and fastembed model cache (`fastembed_cache/`)
- `.e2e_*` — Ephemeral test databases (cleaned up by `run_all_e2e_fast.sh`)
- `node_modules/` — Only for tree-sitter-groovy npm package

### Database Dependencies
**Indexer (`knot-indexer`) requires:**
- Qdrant (default: `http://localhost:6334`) — vector search
- Neo4j (default: `bolt://localhost:7687`) — graph relationships

**Start via:**
```bash
docker compose up -d
# Reads config from ~/.config/knot/.env (copy from .env.example first)
```

**Environment variables** (CLI arguments take highest priority, then env vars, then `~/.config/knot/.env`):
```bash
KNOT_REPO_PATH=/path/to/repo
KNOT_NEO4J_PASSWORD=<required>
KNOT_QDRANT_URL=http://localhost:6334
KNOT_NEO4J_URI=bolt://localhost:7687
KNOT_CUSTOM_CA_CERTS=/etc/ssl/certs/bundle.pem  # Corporate proxy
RUST_LOG=info  # debug, info, warn, error
```

### Incremental Indexing
Default behavior: only re-parses changed files. Tracked via `.knot/index_state.json` (SHA-256 hashes).

```bash
./target/release/knot-indexer          # Incremental (fast)
./target/release/knot-indexer --clean  # Full re-index (deletes all data)
./target/release/knot-indexer --watch  # Real-time watch mode
```

### Configuration File Indexing

By default, configuration files (`.yml`, `.yaml`, `.json`, `.properties`, `.tpl`) and
Kubernetes/Helm manifests are **not indexed**. Build-system files (`package.json`,
`tsconfig.json`, `pom.xml`, `Cargo.toml`, `Jenkinsfile`) are always indexed.

Use `--include-config-files` (or `KNOT_INCLUDE_CONFIG_FILES=true`) to enable config indexing:
```bash
./target/release/knot-indexer --include-config-files
```

### Entity Kinds (Type System)
Each language defines entity kinds (enum `EntityKind`). Common across all:
- `Class`, `Interface`, `Trait`, `Enum`, `Struct`
- `Function`, `Method`, `Constructor`
- `Variable`, `Constant`, `Field`, `Property`

Language-specific (e.g., Rust): `Macro`, `TypeAlias`, `Union`; Groovy: `Closure`, `GroovyTrait`; C++: `CppNamespace`.

v1.2.5 additions: `CargoPackage`, `CargoFeature`, `WorkspaceMember`, `ConfigProperty`, `K8sDeployment`, `K8sService`, `K8sConfigMap`, `K8sSecret`, `K8sIngress`, `K8sNamespace`, `K8sResource`, `HelmChart`, `HelmValue`, `HelmTemplateVar`, `ProjectIdentity`.

Reference intents (enum `ReferenceIntent`):
- `Calls` — function/method invocation
- `References` — variable/type lookup
- `Extends` — inheritance (`extends`, `->`)
- `Implements` — trait/interface implementation
- `ValueReference` — variable/class passed as keyword argument or referenced by value
- `DomElementReference` — JavaScript references an HTML element by ID
- `CssClassUsage` — JavaScript uses or manipulates a CSS class

Relationship types (enum `RelationshipType`):
- `Calls`, `Extends`, `Implements`, `References`
- `ReferencesDOM`, `UsesCSSClass`, `ImportsScript`, `ImportsStylesheet`
- `MacroCalls`, `Contains`, `GenericBound` (Rust)
- `DependsOn` — Repository-to-repository dependency edge (v1.2.5)

---

## Code Quality Requirements

**All PRs must pass:**

```bash
# 1. Clippy (lint rules enforced)
cargo clippy --all-targets -- -D warnings

# 2. Format check
cargo fmt -- --check

# 3. Unit tests
cargo test

# 4. E2E tests (optional locally, required in CI)
./tests/run_all_e2e_fast.sh
```

**No `unsafe` blocks** allowed except in unavoidable ONNX Runtime interop.

---

## Common Workflows

### Adding a New Language

1. **Create parser module** → `src/pipeline/parser/languages/mylang.rs`
2. **Implement `parse_entities()` function** → Return `Vec<ParsedEntity>`
3. **Add tree-sitter grammar** → Cargo.toml dependency, `queries/mylang.scm`
4. **Write unit tests** → Test entity extraction with fixtures
5. **Write E2E tests** → `tests/run_mylang_e2e.sh`
6. **Register in pipeline** → `src/pipeline/parser/mod.rs`
7. **Update README.md** → Document language support, coverage, caveats

### Fixing a Bug

1. **Write E2E regression test** (must fail with current code)
2. **Identify root cause** → Likely in parser module or reference builder
3. **Fix in parser** → Add unit tests covering the case
4. **Verify E2E test passes**
5. **Check no regressions** → `cargo test && ./tests/run_all_e2e_fast.sh`

### Cross-Repo Dependency Linking Workflow (v1.2.5)

1. **Index libraries first** → Best practice for full call resolution
2. **Index client repos** → Auto-discovers DEPENDS_ON edges from build files (pom.xml, build.gradle, Cargo.toml, package.json)
3. **Retroactive linking** → Indexing a library after its clients creates DEPENDS_ON edges retroactively; re-index clients with `--clean` for full cross-repo call resolution
4. **Manual override** → Use `--dependencies` / `KNOT_DEPENDENCIES` to force linking without matching build file identities
5. **Query deps** → `knot deps <repo>` for forward dependencies, `knot deps --reverse <repo>` for reverse lookups

### Refactoring Parser Logic

1. **Understand call graph** → Use `find_callers` on the function
2. **Check impact** → Look at E2E tests that may be affected
3. **Update tests if needed** → BDD means tests define the contract
4. **Run full suite** → Clippy, fmt, unit tests, all E2E suites

### Cutting a Release

The release pipeline fires **one CI run per push to master** and **one Release
run per pushed tag**. To avoid duplicate CI runs during a release, the
`release: bump version to X.Y.Z` commit must include the regenerated
`Cargo.lock` in the same commit (not as a follow-up).

```bash
# 1. Bump version
$EDITOR Cargo.toml            # set version = "X.Y.Z"

# 2. Regenerate lockfile IN THE SAME COMMIT
cargo build --release         # rewrites Cargo.lock with the new version
git add Cargo.toml Cargo.lock
git commit -m "release: bump version to X.Y.Z"

# 3. Quality gate (local, fast feedback)
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test

# 4. Single push to master → exactly 1 CI run
git push origin master

# 5. Tag + push tag → exactly 1 Release run
git tag -a "vX.Y.Z" -m "Release vX.Y.Z"
git push origin "vX.Y.Z"

# 6. Publish to crates.io (requires explicit human confirmation)
cargo publish
```

**Why this matters**: if `Cargo.toml` and `Cargo.lock` are committed
separately, `cargo publish` rejects the dirty working tree and you need a
second commit + push → 2 CI runs instead of 1 during the release. Regenerating
the lockfile locally before the commit keeps the release to exactly 2 workflow
runs (1 CI + 1 Release).

Do **not** edit `.github/workflows/release.yml` directly — it is autogenerated
by `cargo-dist` on `dist init`. The file is hand-edited (v1.4.5+) to add a
`test-unit` gate; `allow-dirty = ["ci"]` in `dist-workspace.toml` keeps
`dist init` from clobbering the gate. If you ever need to regenerate the
file manually, re-add the `test-unit` job and the `needs: [test-unit]`
dependency on `plan` (the `HAND-WRITTEN ADDITION` banner in the file
itself documents the contract).

---

## Troubleshooting

### Build Fails with Rust 2024 Edition
- Check `rustc --version` (needs 1.90+)
- If using rustup: `rustup update nightly` or `rustup default stable`

### E2E Tests Time Out
- Qdrant/Neo4j may not be ready → Check `docker ps` and `docker logs`
- Run cleanup: `cd tests && docker compose -f docker-compose.e2e.yml down -v`
- Restart: `docker compose up -d` (from repo root)

### Incremental Index Becomes Stale
- Delete `.knot/` and rebuild: `rm -rf .knot/ && ./target/release/knot-indexer`
- Or use: `./target/release/knot-indexer --clean`

### Clippy or Fmt Fails Before Commit
- Auto-fix: `cargo fmt && cargo clippy --fix`
- Manual check: `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`

---

## CI/CD

**GitHub Actions workflows** in `.github/workflows/`:
- `ci.yml` — Build + E2E (runs on push to master/main and on PRs)
- `release.yml` — Unit tests + GitHub Release (runs on push tags)

**Job split (v1.4.5):**
- **CI** runs integration + performance tests on every push.
- **Release** gates the GitHub Release on unit tests (`fmt` + `clippy` +
  `cargo test` + release build). If the unit-test job fails, the tag is
  never turned into a release and no GitHub Release is created.

**CI always runs (on push to master/main and on PRs):**
1. `build-binaries` — `cargo build --release --all-features` once, uploads the three
   binaries (`knot-indexer`, `knot`, `knot-mcp`) as a workflow artifact.
2. `test-unit` (fmt + clippy + `cargo test --lib --all-features`) — runs in parallel
   with `build-binaries`; skipped on `release:` commits (those delegate to
   `release.yml`'s own `test-unit` gate).
3. `test-e2e` — `./tests/run_all_e2e_fast.sh` with `KNOT_SKIP_BUILD=1`, downloading
   the pre-built binaries from `build-binaries` instead of rebuilding.
4. `test-performance` — `./tests/benchmark_e2e.sh` with `KNOT_SKIP_BUILD=1`, reusing
   the same pre-built binaries.

**Build reuse pattern (`KNOT_SKIP_BUILD`):** `run_all_e2e_fast.sh` and
`benchmark_e2e.sh` honour `KNOT_SKIP_BUILD=1` to skip their internal
`cargo build --release` step, expecting pre-built binaries in
`target/release/`. CI sets this env var in `test-e2e` and `test-performance`
to avoid rebuilding knot three times per push. Locally, leave it unset and
the scripts build normally.

**Release always runs (on push of a `vX.Y.Z` tag):**
1. `test-unit` (fmt + clippy -D warnings + `cargo test --lib --all-features` + `cargo build --release --all-features`)
2. `plan` → `build-local-artifacts` → `build-global-artifacts` → `host` → `announce` (cargo-dist)
   - The host job creates the GitHub Release; the announce step finalizes the entry.
   - Any failure in `test-unit` blocks all subsequent jobs.

**Maintenance warning** — `.github/workflows/release.yml` is autogenerated by
`cargo-dist` (note the `dist init` comment in the file header). Since v1.4.5
the file is hand-edited to add a `test-unit` gate. The
`allow-dirty = ["ci"]` setting in `dist-workspace.toml` makes `cargo-dist`
skip the freshness check on the file, so `dist init` and the `plan` job do
**not** clobber the hand-edits. Do not remove that setting — if you do, the
next `dist init` will silently overwrite the `test-unit` job and the
`needs: [test-unit]` dependency on `plan`, breaking the release gate.

Failure in CI = PR not mergeable. Failure in Release = tag never publishes
a release. Check `.github/workflows/ci.yml` and `.github/workflows/release.yml`
for exact steps.

---

## Key References in Codebase

- **Parser module registration**: `src/pipeline/parser/mod.rs` (dispatcher for each language)
- **MCP tools definition**: `src/mcp_tools/` (search_hybrid_context, find_callers, explore_file)
- **CLI command routing**: `src/bin/knot.rs` (search/explore/callers/deps subcommands)
- **E2E test structure**: `tests/run_kotlin_e2e.sh` (template for all per-language suites)
- **Docker setup**: `docker-compose.yml` (production), `tests/docker-compose.e2e.yml` (ephemeral)

---

## When to Ask a Human

- **Team conventions not written down**: e.g., naming patterns for new entity kinds
- **Release strategy questions**: e.g., breaking change policy, crates.io publishing cadence
- **Architecture decisions**: e.g., adding a new database or changing the indexing pipeline
- **External tool integration**: e.g., supporting a new language or build system

---

## AI Agent Emphasis

**CRITICAL**: This repo contains **two systems**: the indexer (builds databases) and the MCP tools (queries them). When contributing:

1. **Indexer changes** → Modify parsers, add unit + E2E tests
2. **Query tools changes** → Modify MCP handlers, update tool descriptions
3. **Both** → Update `.prompt` and `.knot-agent.md` if tool behavior changes

Use the `knot` MCP tools yourself for code exploration—this repo is designed to showcase them.
