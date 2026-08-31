# Organisation scanning — A to Z

End-to-end guide for indexing every repository in a GitHub organisation (or any folder of clones) with Knot, then querying the results via CLI or MCP.

Knot indexes **local directories**. There is no built-in GitHub/GitLab API connector — you clone first, then run `knot-indexer` per repo (or use the batch script below).

---

## Prerequisites

| Requirement | Version | Notes |
|-------------|---------|-------|
| Docker | 20.10+ | Neo4j + Qdrant via `docker compose` |
| Rust | 1.85+ | `cargo build --release` |
| Git | any | Clone repositories |
| `gh` CLI | optional | List/clone a GitHub org |

Allow ~5–10 GB disk for DB volumes and `.knot/fastembed_cache/` on a 10–50 repo workspace.

---

## 1. Start infrastructure

From the Knot repo root:

```bash
docker compose up -d
```

| Service | URL | Default auth |
|---------|-----|--------------|
| Neo4j Browser | http://localhost:7474 | `neo4j` / `knot_secret_password` |
| Qdrant | http://localhost:6333 | none |

Confirm health: `docker compose ps` — both services should be `healthy`.

---

## 2. Build binaries

```bash
cargo build --release
```

| Binary | Purpose |
|--------|---------|
| `target/release/knot-indexer` | Parse repo → Neo4j graph + Qdrant vectors |
| `target/release/knot` | CLI: search, callers, explore, repos, deps |
| `target/release/knot-mcp` | MCP server for AI clients |

---

## 3. Configure Knot

Config lives at **`~/.config/knot/.env`** (never in the repo being indexed).

```bash
mkdir -p ~/.config/knot
cp .env.example ~/.config/knot/.env
```

Minimum settings:

```env
KNOT_NEO4J_PASSWORD=knot_secret_password
KNOT_QDRANT_URL=http://localhost:6334
KNOT_NEO4J_URI=bolt://localhost:7687
KNOT_REPO_PATH=/path/to/default/repo   # optional default for CLI
```

Priority: CLI flags → environment variables → `~/.config/knot/.env`.

---

## 4. Index one repo (smoke test)

```bash
./target/release/knot-indexer \
  --repo-path /path/to/my-app \
  --neo4j-password knot_secret_password
```

First run on a large codebase (~3–4k files) can take **~60 minutes**. Later runs are incremental (SHA-256 diff in `.knot/index_state.json`).

Verify:

```bash
./target/release/knot repos
./target/release/knot search "authentication" --repo my-app
./target/release/knot explore "src/main.rs" --repo my-app
```

Useful flags:

| Flag | When |
|------|------|
| `--clean` | Full rebuild (deletes existing index for that repo) |
| `--watch` | Re-index on file changes |
| `--repo-name custom` | Override auto-detected name (last path segment) |
| `--include-config-files` | Also index YAML/JSON/K8s/Helm |

---

## 5. Clone an entire GitHub organisation

```bash
./scripts/clone-workspace.sh Synthapse
# or: ./scripts/clone-workspace.sh Synthapse ~/code/Synthapse --no-forks
```

Manual alternative with `gh`:

```bash
ORG="Synthapse"
WORKDIR="$HOME/code/$ORG"
mkdir -p "$WORKDIR"

gh auth login   # once
gh repo list "$ORG" --limit 1000 --json name -q '.[].name' | while read -r repo; do
  [[ -d "$WORKDIR/$repo" ]] || gh repo clone "$ORG/$repo" "$WORKDIR/$repo"
done
```

GitLab/Bitbucket: replace with your platform’s list/clone commands — Knot only needs local paths.

### Remove cloned repositories

```bash
# Preview what would be deleted
./scripts/remove-workspace.sh ~/code/Synthapse --dry-run

# Delete clones (prompts for confirmation)
./scripts/remove-workspace.sh ~/code/Synthapse

# Delete without prompt; also purge Knot index entries per repo
KNOT_PURGE_INDEX=1 ./scripts/remove-workspace.sh ~/code/Synthapse --yes
```

Removing clones does **not** wipe Neo4j/Qdrant unless you use `KNOT_PURGE_INDEX=1` or `docker compose down -v`.

---

## 6. Batch-index a workspace

Use the helper script:

```bash
./scripts/index-workspace.sh "$HOME/code/your-org"
```

Or run manually:

```bash
for dir in "$HOME/code/your-org"/*/; do
  ./target/release/knot-indexer \
    --repo-path "$dir" \
    --neo4j-password knot_secret_password
done
./target/release/knot repos
```

Each repo is stored as a separate `:Repository` node in Neo4j. All repos share one Qdrant collection (`knot_entities` by default), filtered by repo name at query time.

---

## 7. Cross-repo dependencies

Knot discovers repo-to-repo edges from build files (`package.json`, `Cargo.toml`, `pom.xml`, `.csproj`, …):

```bash
./target/release/knot deps api-gateway --depth 2
./target/release/knot deps shared-lib --reverse
```

Forward: what this repo depends on. `--reverse`: who depends on this repo.

---

## 8. Query outputs (what you get)

### `knot repos`

Inventory of every indexed repository:

```
REPO          BUILD SYSTEM  LANGUAGE   FILES  ENTITIES
api-gateway   npm           typescript   412    8,204
auth-service  cargo         rust         186    3,891
```

JSON: `knot repos --output json`

### `knot portfolio`

Org-wide portfolio report: inventory, per-repo dependencies, rule-based recommendations (`register`, `retire`, `invest`, `harden`, `consolidate`), and correlation patterns (platform hubs, isolated repos).

```bash
knot portfolio                    # markdown (default)
knot portfolio --output json      # for CMDB or GenAI handoff
```

See [BUSINESS-OPPORTUNITIES.md](BUSINESS-OPPORTUNITIES.md).

### `knot search "<query>"`

Semantic + structural search. Returns entities with signatures, file paths, line numbers, docstrings, and dependency hints.

### `knot callers "<symbol>"`

Reverse lookup — every call site, grouped by target when names collide. Includes `OVERRIDES` edges for JVM and C#.

### `knot explore "<file>"`

All classes, methods, functions in a file with signatures and documentation.

### `knot deps <repo>`

Repository dependency tree (transitive, default depth 3).

---

## 9. MCP (Cursor, Claude, etc.)

```json
{
  "mcpServers": {
    "knot": {
      "command": "/absolute/path/to/knot/target/release/knot-mcp",
      "env": {
        "KNOT_REPO_PATH": "/path/to/workspace",
        "KNOT_NEO4J_URI": "bolt://localhost:7687",
        "KNOT_NEO4J_USER": "neo4j",
        "KNOT_NEO4J_PASSWORD": "knot_secret_password",
        "KNOT_QDRANT_URL": "http://localhost:6334"
      }
    }
  }
}
```

Tools: `list_repositories`, `search_hybrid_context`, `find_callers`, `explore_file`, `list_repo_dependencies`.

Agent skill guides: [`docs/agent-skills/`](agent-skills/).

---

## 10. Contributing / developing Knot

```bash
# Unit + integration tests
cargo test
cargo clippy -- -D warnings

# E2E (starts its own Docker stack)
./tests/run_all_e2e_fast.sh

# Benchmarks
cargo bench
./tests/benchmark_e2e.sh
```

When changing parsers or Tree-sitter queries, re-index test fixtures with `--clean`.

Key directories:

| Path | Role |
|------|------|
| `src/pipeline/` | Indexing pipeline (parse → embed → ingest) |
| `src/mcp_tools/` | MCP tool implementations |
| `src/cli_tools/` | CLI commands |
| `queries/*.scm` | Tree-sitter extraction queries |
| `tests/testing_files/` | Language fixtures for E2E |

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Cannot connect to Neo4j/Qdrant | `docker compose up -d` |
| Empty `knot repos` | Successful `knot-indexer` run required first |
| Progress stuck then crash | Check disk space; try `--clean` on one repo |
| Corporate SSL proxy | `KNOT_CUSTOM_CA_CERTS` in `~/.config/knot/.env` |
| Stale index after upgrade | Automatic full re-index when `.knot/index_state.json` version mismatches |

---

## Related

- [README](../README.md) — features, configuration reference, token efficiency
- [BUSINESS-OPPORTUNITIES.md](BUSINESS-OPPORTUNITIES.md) — turning scan results into portfolio insights
- [CHANGELOG](../CHANGELOG.md) — release history
