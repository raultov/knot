# Knot Deps: Repository Dependency Graph

**Command:** `knot deps <repo_name> [--depth <N>] [--reverse] [--output <format>]`

## Purpose

Traverse the `DEPENDS_ON` graph between indexed repositories. Knot auto-discovers
cross-repository dependencies from build-system files (Maven `pom.xml`, Gradle
`build.gradle`, Cargo `Cargo.toml`, npm `package.json`, etc.) and stores them as
`:DEPENDS_ON` edges between `:Repository` nodes in Neo4j.

`knot deps` answers:
- "Which repositories does this project depend on?"
- "Which projects depend on this library?" (with `--reverse`)
- "How deep does the dependency chain go?" (with `--depth`)

## Parameters

- **`<repo_name>`** (required): The repository to inspect. Must match a
  `:Repository.name` already present in the graph. Use `knot repos` to discover
  the available names.

- **`--depth <N>`**: Maximum depth for transitive traversal (default: 3, max: 10 — the bound is enforced; larger values are clamped to 10, values below 1 to 1).
  - `1` = direct dependencies only
  - `2` = direct + one level deeper
  - `3` = the default and the deepest level usually needed in practice
  - Maximum: 10 (enforced; larger values are clamped)
  - Applies to **both** directions: with `--reverse` it controls how many
    levels of dependents are followed (transitive impact analysis).

- **`--reverse`**: Show **reverse** dependencies — repositories that depend
  ON this one. Useful for "who depends on this shared library?" impact analysis.

- **`--output <format>`**: `table` (default), `json`, or `markdown`.

## Output Format

### Table (default)

```
# Dependencies of `my-app`

+-- auth-lib
+-- common-utils
+-- billing-api
```

### JSON

```json
[
  { "repo_name": "auth-lib" },
  { "repo_name": "common-utils" },
  { "repo_name": "billing-api" }
]
```

### Reverse with `--reverse`

```
# Repositories that depend on `auth-lib`

+-- my-app
+-- admin-portal
+-- mobile-app
```

## When to Use Deps

### 1. Onboarding to a Multi-Repository Workspace

When you join a project that has several repositories, start with `knot repos`
to see what is indexed, then `knot deps <name>` to understand the topology.

```bash
knot repos
# Found 4 repos: backend, frontend, shared-lib, mobile-app

knot deps backend --depth 2
# backend depends on: shared-lib, auth-service
# auth-service depends on: shared-lib
```

### 2. Breaking-Change Impact Analysis (Library Author)

Before making a breaking change to a shared library, find every consumer:

```bash
knot deps auth-lib --reverse
# my-app, admin-portal, mobile-app all depend on this
```

Then for each consumer, trace which specific functions they call:

```bash
knot callers "TokenVerifier" --repo my-app
```

### 3. Verifying Auto-Discovered Dependencies

After indexing a new repository, confirm that knot picked up the build-system
declarations:

```bash
# After running knot-indexer against /path/to/new-app
knot deps new-app
# Should list everything declared in pom.xml / build.gradle / Cargo.toml
```

### 4. Diagnosing Circular Dependencies

If a transitive query shows a cycle (e.g. `A` depends on `B` and `B` depends on
`A`), the tool will still report the names. Investigate with `callers` and
`explore` to understand the coupling.

## Cross-Repository Call Resolution

`DEPENDS_ON` edges are not just informational — they enable **cross-repository
call resolution**. When `knot callers` follows a CALLS edge and the target
entity is not in the current repository, knot automatically looks up matching
entities in any of the directly-depended-on repositories.

This means that if you have:

- `auth-lib` indexed (defines `TokenVerifier.verify()`)
- `my-app` indexed, with a `DEPENDS_ON` edge to `auth-lib`

Then `knot callers "TokenVerifier" --repo my-app` will show calls to
`TokenVerifier.verify()` that come from `my-app` *and* any repository `my-app`
depends on. The result is grouped by target so you can see exactly which
specific entity each caller references.

## Workflow Patterns

### Pattern: Library Author — Pre-Release Safety Check

```bash
# 1. Who consumes the library?
knot deps my-lib --reverse
# → my-app, billing-service, mobile-app

# 2. For each consumer, find the most-called entry points
knot callers "my_lib_init" --repo my-app
knot callers "my_lib_init" --repo billing-service

# 3. If the change is breaking, coordinate with each consumer team
```

### Pattern: New Hire — Map the Workspace

```bash
# 1. List everything
knot repos

# 2. Pick a starting point and see what it depends on
knot deps backend --depth 2

# 3. Drill into a specific dependency
knot explore "src/auth/verifier.ts" --repo auth-lib
```

### Pattern: Build-System Refactor

```bash
# 1. Snapshot current dependency graph
knot deps monolith --depth 3 --output json > /tmp/deps-before.json

# 2. Refactor pom.xml / build.gradle / Cargo.toml

# 3. Re-index
knot-indexer --repo-path /path/to/monolith

# 4. Compare graphs
knot deps monolith --depth 3 --output json > /tmp/deps-after.json
diff /tmp/deps-before.json /tmp/deps-after.json
```

## Limitations

- **Only indexed repositories appear**: A dependency declared in `pom.xml` but
  not yet indexed will not show up in `knot deps`. The corresponding
  `:Repository` node is created only when you run `knot-indexer` against that
  path. Use `knot repos` to see what is currently indexed.
- **Build-system-driven**: Edge discovery comes from `pom.xml`,
  `build.gradle`, `Cargo.toml`, and `package.json` — it does not crawl source
  code for `import` statements.
- **Bidirectional linking**: Indexing **either** side of the relationship
  creates the `DEPENDS_ON` edge. Index the consumer and it links to already
  indexed libraries; index a library *after* its consumer and the reverse
  sweep links every already-indexed consumer without re-indexing them. The
  edge appears on whichever run notices the match — no `--clean` required.
- **One identity per repository**: Only the root build manifest registers the
  repository identity. For npm workspaces, sub-package manifests are *not*
  matchable dependency targets — only the workspace root's `package.json`
  name participates in linking.
- **Helm chart dependencies are not linked**: the Helm parser emits chart
  dependency names without a `helm:` prefix, which the forward matcher cannot
  resolve back to an indexed Helm repository. This is a known limitation
  (candidate for a follow-up fix).

## Troubleshooting

### "No dependencies found" but `pom.xml` lists dependencies

**Cause:** The depended-on repository has not been indexed yet.

**Solutions:**
- Run `knot-indexer --repo-path /path/to/dependency` — the `DEPENDS_ON` edge
  is created by that run itself (the reverse sweep links every already
  indexed consumer)
- Verify with `knot repos` that both repositories are present

### `knot deps` reports declared dependencies but none indexed

When a repository declares build dependencies but none of them resolves to an
indexed repository, the output says so explicitly instead of a bare
"No dependencies found.":

```
`job-watch-ui` declares 35 build dependencies, but none of them
resolves to a repository indexed in knot.
...
Declared build dependencies (35):
  npm:@eslint/js:^9.39.5
  npm:@hookform/resolvers:^5.7.1
  ...
```

**Cause:** None of the declared dependency artifacts is an indexed
repository — entity extraction worked, linking correctly found nothing to
link.

**Solutions:**
- Check `knot repos` for what *is* indexed; the full declared list (as a
  reference to the identity names the consumers match on) is right there
- Index the dependency's own repository with
  `knot-indexer --repo-path <path>` — the edge is created by that run
- The `--output json` shape is unchanged (array of `{ "repo_name": ... }`);
  the explanation only appears in the human-readable output

Other empty-result explanations:

- `Repository \`X\` is not indexed.` — the queried name has no `:Repository`
  node; check the spelling against `knot repos`.
- `No dependencies found: \`X\` declares no build dependencies` — the repo
  has no dependency section in its manifest at all.
- `Cannot determine dependents of \`X\`: no matchable build identity` — the
  reverse lookup cannot answer because the repository has no usable
  build-system identity (e.g. `build_system: "none"`); re-index it with a
  build manifest. This is never conflated with "nobody declares it".

### Declared dependency resolves, but no DEPENDS_ON edge yet

If a declared dependency **resolves** to an indexed repository but the edge
is missing, the output says so explicitly instead of claiming the dependency
is not indexed:

```
`job-watch` declares 24 build dependencies; 1 of them resolve to a
repository indexed in knot but have no DEPENDS_ON edge yet:
  cdp-browser-lite:0.3.4 -> cdp-browser-lite
The graph is stale: re-index either side to create the edge(s)
(`knot-indexer --repo-path <path>` on `job-watch` or on the
dependency's own repository).
```

The same distinction exists in reverse: if indexed repositories **declare**
the queried repository but the edges are missing, they are named:

```
No DEPENDS_ON edges point at `cdp-browser-lite`, but 2 indexed repositories
declare it as a build dependency:
  chrome-control-mcp (declares `cdp-browser-lite:0.3`)
  job-watch (declares `cdp-browser-lite:0.3.4`)
The graph is stale: re-index either side to create the edge(s).
```

**Cause:** a stale graph — the consumer was indexed before the library
existed (or before its identity was matchable) and linking did not run since.
A declared artifact that resolves is never reported as "not indexed"; the
three outcomes (edge exists / resolves without edge / does not resolve) are
always reported separately.

### Repository name in `knot deps` does not match the directory

Knot uses the identity declared in the build manifest: the npm package `name`
field (including `@scope/name`, which is stored split as scope + package),
Maven's `<groupId>:<artifactId>`, Cargo's `[package].name`, or MSBuild's
`<PackageId>`. The dashes-to-underscores conversion applies to Rust crates
picked up without a nearest-crate anchor only — it does not apply to npm,
Maven, or Gradle names. Use `knot repos` to confirm the canonical name.

### Results look stale after a build-file change

Re-index the consumer repository:

```bash
knot-indexer --repo-path /path/to/consumer
```

The next `knot deps` will reflect the new build-system state.

## See Also

- `knot repos` — list every indexed repository and its status
- `knot search` — semantic + structural code search
- `knot callers` — reverse dependency lookup within and across repositories
- `knot explore` — file anatomy inspection
