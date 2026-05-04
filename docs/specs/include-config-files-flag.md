# Spec: `--include-config-files` flag for knot-indexer

## Summary

New flag `--include-config-files` (default: `false`) that enables indexing of generic
configuration files (YAML, JSON, .properties) and Kubernetes/Helm manifests. By default,
these files are **skipped** in both discovery and parsing.

**Rationale:**
- Configuration files may contain secrets (passwords, tokens, API keys).
- Repos with heavy config content slow down the indexer significantly.
- Most users are interested in code search, not config property search.

## Scope

**Always active** (build system / project identity):
`package.json`, `tsconfig.json`, `pom.xml`, `Cargo.toml`, `Jenkinsfile`.

**Gated by the flag:**
`.yml`, `.yaml` (generic + K8s + Helm), `.json` (generic — not `package.json`/`tsconfig.json`),
`.properties`, `.tpl` (Helm templates), `Chart.yaml`, `values.yaml`.

### Extensions affected

| Extension | `--include-config-files=false` | `--include-config-files=true` |
|-----------|-------------------------------|-------------------------------|
| `.yml` / `.yaml` | Skipped | Indexed (YAML/K8s/Helm dispatch) |
| `.json` | Only `package.json` and `tsconfig.json` | All `.json` indexed |
| `.properties` | Skipped | Indexed |
| `.tpl` | Skipped | Indexed (Helm templates) |
| `.java`, `.ts`, `.rs`, etc. | Indexed | Indexed |
| `pom.xml`, `Cargo.toml` | Indexed (by filename) | Indexed |

## Flag data flow

```
CLI: --include-config-files  /  Env: KNOT_INCLUDE_CONFIG_FILES
  |
  v
IndexerCli.include_config_files (bool)
  |
  v
Config.include_config_files (bool)
  |
  +-- discover_files(repo_path, include_config_files)   [input.rs]
  |
  +-- ParseConfig.include_config_files                   [parser/mod.rs]
  |     |
  |     v
  |     parse_single_file() -- guard in dispatch
  |
  +-- setup_watch_mode(... include_config_files)         [watch.rs]
        |
        v
        is_supported_file(path, include_config_files)    [files.rs]
```

## Changes by file

### 1. `src/config.rs` — New CLI field + Config field

**IndexerCli** (after `rayon_threads`):
```rust
/// Include configuration files (YAML, JSON, .properties) and Kubernetes/Helm
/// manifests in the index. Disabled by default to avoid indexing secrets and
/// to speed up indexing in repos with heavy config content.
#[arg(long, env = "KNOT_INCLUDE_CONFIG_FILES", default_value_t = false)]
pub include_config_files: bool,
```

**Config struct**: Add field `pub include_config_files: bool`.

**`load_indexer()`**: Map `cli.include_config_files` to the Config struct.

**`load_mcp()`, `load_knot_cli()`**: These don't index. Hardcode to `false`.

### 2. `src/pipeline/input.rs` — Filter extensions during discovery

Split `SUPPORTED_EXTENSIONS` into two lists:
- `CORE_EXTENSIONS` — source code extensions (java, ts, tsx, js, kt, py, rs, groovy, gradle, html, css, c, cpp, etc.)
- `CONFIG_EXTENSIONS` — config extensions (`yml`, `yaml`, `json`, `properties`, `tpl`)

Change `discover_files` signature to accept `include_config_files: bool`.

In the `filter_map` closure:
- Extensions in `CORE_EXTENSIONS` → always include.
- Extensions in `CONFIG_EXTENSIONS` → include **only if** `include_config_files == true`.
- **Exception for `.json`**: `package.json` and `tsconfig.json` are always included
  (they match via `known_names`, but also match by `.json` extension).
  When `include_config_files == false`, `.json` files should only be included
  if their filename is `package.json` or `tsconfig.json`.

Keep `SUPPORTED_EXTENSIONS` as the union of both lists for external use
(e.g. `is_supported_file` in watch mode).

Update existing tests in `input.rs`.

### 3. `src/pipeline/files.rs` — `is_supported_file` for watch mode

Add `include_config_files: bool` parameter to `is_supported_file()`.

When `false`, reject config extensions (same logic as `discover_files`).

### 4. `src/pipeline/watch.rs` — Propagate the flag

`setup_watch_mode` receives `include_config_files` and passes it to `is_supported_file`.

### 5. `src/pipeline/runner.rs` — Propagate the flag

- `discover_files(&cfg.repo_path)` → `discover_files(&cfg.repo_path, cfg.include_config_files)`
- `build_parse_config` → add `include_config_files` to `ParseConfig`

### 6. `src/pipeline/parser/mod.rs` — Guard in dispatch

**ParseConfig**: Add field `pub include_config_files: bool`.

**`parse_single_file()`**: Add guards before config parser dispatch arms.
This is defense-in-depth (files should already be filtered in discovery).

```rust
"yml" | "yaml" => {
    if !parse_cfg.include_config_files {
        vec![]
    } else {
        dispatch_yaml(...)
    }
}

"json" => {
    if !parse_cfg.include_config_files
        && filename != "package.json"
        && filename != "tsconfig.json"
    {
        vec![]
    } else {
        languages::json_config::extract_entities_json_config(...)
    }
}

"properties" => {
    if !parse_cfg.include_config_files {
        vec![]
    } else {
        languages::properties::extract_entities_properties(...)
    }
}

"tpl" => {
    if !parse_cfg.include_config_files {
        vec![]
    } else {
        // existing helm template logic
    }
}
```

### 7. Tests

**`src/pipeline/input.rs`**:
- Update existing `discover_files` tests to pass the new parameter.
- New test: with `include_config_files: false`, `.yaml`/`.json`/`.properties`/`.tpl`
  files are not discovered, but `package.json` and `tsconfig.json` are.

**`src/pipeline/parser/mod.rs`**:
- With `include_config_files: false`: YAML/generic JSON/properties/tpl produce `vec![]`.
- With `include_config_files: false`: `package.json` and `tsconfig.json` are still parsed.
- With `include_config_files: true`: everything works as before.

**`src/config.rs`**:
- CLI parsing test for the new flag.

### 8. Documentation

- **`README.md`**: Add `--include-config-files` / `KNOT_INCLUDE_CONFIG_FILES` to the
  configuration reference table. Mention in the usage section.
- **`AGENTS.md`**: Document the new flag.
- **`.env.example`**: Add `# KNOT_INCLUDE_CONFIG_FILES=false` (commented out).

## Implementation order

1. `config.rs` — Add field to `IndexerCli`, `Config`, mapping
2. `input.rs` — Split extensions, modify `discover_files`
3. `files.rs` — Modify `is_supported_file`
4. `watch.rs` — Propagate flag
5. `runner.rs` — Propagate flag to `discover_files` and `ParseConfig`
6. `parser/mod.rs` — Add field to `ParseConfig`, guards in dispatch
7. Unit tests (config, input, parser dispatch)
8. Documentation (README, AGENTS.md, .env.example)
9. Validator: fmt, clippy, cargo test
