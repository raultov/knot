# Knot Files: Read-Only File Listing

**Command:** `knot files [--path <prefix-or-glob>] [--repo <name>]`
**MCP tool:** `list_files`

## Purpose

Enumerate the files an indexed repository carries — ordered, with entity
counts — so you can discover the codebase layout before searching or
exploring. This is the answer to "list every file under `src/hooks`" and
the discovery half of "search only in `src/api/**`".

## Parameters

- **`--path <prefix-or-glob>`** (optional): repo-relative scope
  - **Directory prefix** (preferred): matched on a path boundary;
    `src/api` matches `src/api/users.ts` but never `src/api-notes.md`.
  - **Glob**: `*` wildcards a segment (`src/*.rs`), `?` one character,
    `**` matches any depth (`src/**/*_test.rs`).
  - Omitted: lists every indexed file (capped; the reply notes
    truncation and you should narrow the scope).

- **`--repo <name>`**: repository scope (recommended when several indexed
  repositories share path shapes).

## Ordering & determinism

Rows are ordered by `(repo_name, file_path)` with their entity counts —
same query, same order, every time.

## Complementary tools

- `search_hybrid_context` accepts the same `path` value (`--path` for the
  CLI, `path` over MCP) to scope a semantic search to those files.
- `explore_file` inspects the anatomy of one file from the listing.
