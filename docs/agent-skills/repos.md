# Knot Repos: Indexed Repository Inventory

**Command:** `knot repos [--filter <substring>] [--output <format>]`

## Purpose

List every repository currently indexed in the graph database, along with
its entity count, file count, build system, and primary language. Use it as
a quick orientation tool — "what is actually in the index right now?" — or
to sanity-check that an indexing run completed.

`knot repos` does not require the embedder or the vector database; it queries
Neo4j only. This makes it the fastest CLI command in the knot toolkit.

## Parameters

- **`-f, --filter <substring>`**: Case-insensitive substring match on the
  repository name. Only repositories whose `:Repository.name` contains
  `<substring>` (lowercased on both sides) are returned. Useful when the
  index holds many repositories and you want to focus on a subset
  (e.g. `--filter auth` to surface `auth-lib`, `auth-service`, etc.).
  When the filter matches nothing, the command prints
  `No repositories found.` and exits 0.

- **`-o, --output <format>`**: `table` (default), `json`, or `markdown`.

## Output Format

### Table (default)

```
Indexed repositories (2):
+-------------+--------------+----------+-------+----------+
| REPO        | BUILD SYSTEM | LANGUAGE | FILES | ENTITIES |
+==========================================================+
| knot        | cargo        | rust     |   150 |    2 262 |
|-------------+--------------+----------+-------+----------|
| knot-server | cargo        | rust     |    16 |      299 |
+-------------+--------------+----------+-------+----------+
```

Column meanings:

- **REPO** — `:Repository.name` in Neo4j. Matches `[package].name` from the
  build file (dashes converted to underscores).
- **BUILD SYSTEM** — Detected build system: `cargo`, `maven`, `gradle`, `npm`,
  `pip`, etc. Empty when the repository was not yet updated with a
  `ProjectIdentity` (older indexes).
- **LANGUAGE** — The most common `language` property across the repository's
  indexed entities. Empty when the repository has no entities.
- **FILES** — Number of distinct `file_path` values among the repository's
  entities.
- **ENTITIES** — Total number of entity nodes (classes, methods, functions,
  etc.) for the repository.

### JSON

```json
[
  {
    "name": "knot",
    "build_system": "cargo",
    "primary_language": "rust",
    "file_count": 150,
    "entity_count": 2262
  },
  {
    "name": "knot-server",
    "build_system": "cargo",
    "primary_language": "rust",
    "file_count": 16,
    "entity_count": 299
  }
]
```

The JSON shape is stable: `{ name, build_system, primary_language,
file_count, entity_count }`.

### Markdown

```markdown
# Indexed repositories (2)

| REPO | BUILD SYSTEM | LANGUAGE | FILES | ENTITIES |
|------|--------------|----------|------:|---------:|
| knot | cargo | rust | 150 | 2 262 |
| knot-server | cargo | rust | 16 | 299 |
```

Renders as a right-aligned GFM table — useful inside chat UIs and Markdown
notes.

### Empty State

When no repositories are indexed, every format prints `No repositories found.`
(plain text). This usually means you have not yet run `knot-indexer` against
any code.

## When to Use Repos

### 1. Onboarding to a Multi-Repository Workspace

When you join a project that ships knot as a shared tool, the first question
is "what is actually indexed?". `knot repos` is the answer.

```bash
knot repos
# → 4 repos: backend, frontend, shared-lib, mobile-app
```

### 2. Verifying an Indexing Run

After running `knot-indexer --repo-path /path/to/new-app`, check that the
repository appears in the list with a non-zero entity count:

```bash
knot-indexer --repo-path /path/to/new-app
knot repos
# → new-app now appears with its entity_count
```

A repository missing from the list, or showing `0` entities, indicates the
indexing run did not complete successfully.

### 3. Discovering Languages and Build Systems

`knot repos` is the fastest way to see which languages and build systems
are present in your workspace — useful when planning a new feature or
investigating a build infrastructure problem.

```bash
knot repos --output json | jq -r '.[].build_system' | sort -u
# → cargo
# → maven
# → npm
```

### 4. Cross-Repository Call Resolution Setup

Before running `knot callers` across repositories, list what is available
so you know which `--repo` values are valid.

```bash
knot repos
# Confirms that auth-lib is indexed
knot callers "TokenVerifier" --repo my-app
# Cross-repo resolution works because my-app has a DEPENDS_ON edge to auth-lib
```

### 5. Focused Lookup with `--filter`

When the index contains dozens of repositories, listing all of them is
noisy. Use `--filter` to find a specific repository (or a family of
related ones) without piping through `grep`:

```bash
knot repos --filter app
# → my-app, mobile-app, web-app

knot repos --filter AUTH       # case-insensitive
# → auth-lib, auth-service

knot repos --filter zzz
# → No repositories found.
```

This is the fastest way to confirm a repository name before running
`knot search`, `knot callers`, `knot explore`, or `knot deps` with
`--repo <name>`.

## Workflow Patterns

### Pattern: Index-Health Check (Cron-Style)

```bash
# Quick health check — exit non-zero if expected repos are missing.
EXPECTED="backend frontend shared-lib"
ACTUAL=$(knot repos --output json | jq -r '.[].name' | sort)
EXPECTED_SORTED=$(echo "$EXPECTED" | tr ' ' '\n' | sort)
if [ "$ACTUAL" != "$EXPECTED_SORTED" ]; then
  echo "Index drift detected"
  exit 1
fi
```

### Pattern: Repositories-by-Language

```bash
# Group repositories by primary language using the JSON output.
knot repos --output json \
  | jq -r 'group_by(.primary_language)[] | "\(.[0].primary_language): \(map(.name) | join(", "))"'
```

### Pattern: Storage-Usage Triage

```bash
# Identify the heaviest repositories by entity count.
knot repos --output json \
  | jq -r 'sort_by(-.entity_count) | .[] | "\(.entity_count|tostring | . + " entities"))\t\(.name)"' \
  | head -10
```

## Limitations

- **Counts are approximate proxies**: `entity_count` and `file_count` come
  from the `Entity` nodes linked to a repository by `repo_name`. They
  reflect what is in the index, not necessarily what is on disk.
- **`primary_language` is a mode, not a strict metric**: When a repository
  has multiple languages (e.g. a polyglot codebase with Rust, TypeScript and
  Python), the table shows the *most common* language, with ties broken
  arbitrarily by HashMap iteration order.
- **No `build_system` for old indexes**: If a repository was indexed
  before build-system support was added, its `build_system` cell is empty.
  Re-index the repository with the current version of `knot-indexer` to
  populate it.
- **Only name filtering is built-in**: `--filter` matches the repository
  name only. There is no `--language` or `--build-system` filter flag
  yet. For those, pipe `--output json` through `jq` for ad-hoc filtering.

## See Also

- `knot deps` — repository-to-repository dependency graph
- `knot search` — semantic + structural code search
- `knot callers` — reverse dependency lookup
- `knot explore` — file anatomy inspection
