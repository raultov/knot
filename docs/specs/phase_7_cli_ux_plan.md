# Phase 7: CLI UX Improvements & Corporate Network Support

## 1. Overview & Objectives

This document outlines the implementation plan for Phase 7 of the `knot` indexer. The goal is to significantly improve the user experience of the standalone CLI (`knot`) by providing human-readable formatted output, interactive pagination, and adding support for custom CA certificates to allow model downloading behind corporate SSL-inspecting proxies.

### Key Requirements
- **Corporate Network Support:** Allow users to pass custom CA certificates via CLI args or environment variables to enable `fastembed` (via `hf-hub`/`ureq`) to download ONNX models through corporate proxies.
- **Output Formatting:** Decouple the raw Markdown output (intended for MCP clients) from the CLI output. Provide `--output` options (`table`, `json`, `markdown`).
- **Interactive Paging:** Automatically pipe large CLI table outputs to a pager (like `less`) when running in a TTY environment.

---

## 2. Feature 1: Custom CA Certificates

**Objective:** Inject a custom certificate bundle path into the native TLS stack used by `fastembed`'s underlying download manager (`hf-hub` -> `ureq` -> `native-tls`/`rustls`).

### 2.1. Configuration (`src/config.rs`)
Add a new argument to the CLI configuration structs (both full indexer and lightweight clients):

```rust
/// Path to a custom CA certificate bundle for corporate network model downloads
#[arg(long, env = "KNOT_CUSTOM_CA_CERTS")]
pub custom_ca_certs: Option<String>,
```

### 2.2. Global Injection (`src/bin/knot.rs`, `knot-mcp.rs`, `knot-indexer.rs`)
At the very beginning of the `main()` function in all three binaries, check if the configuration provides a custom CA certificate path. If so, set standard environment variables before any network requests occur:

```rust
if let Some(cert_path) = &cfg.custom_ca_certs {
    // Force native-tls/rustls to trust the provided bundle
    std::env::set_var("SSL_CERT_FILE", cert_path);
    std::env::set_var("SSL_CERT_DIR", cert_path);
}
```

---

## 3. Feature 2: Configurable Output Formats

**Objective:** Allow users to choose how `knot search`, `knot callers`, and `knot explore` display data.

### 3.1. Dependencies (`Cargo.toml`)
Add crates for table formatting and terminal colors:
```toml
comfy-table = "7.1"
colored = "2.1"
```

### 3.2. Output Enum (`src/config.rs` or `src/cli_tools/mod.rs`)
Create an enum for the requested output format:
```rust
#[derive(clap::ValueEnum, Clone, Default)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Markdown,
}
```
Add `--output <FORMAT>` to the CLI commands in `src/bin/knot.rs`.

### 3.3. Refactoring CLI Tools (`src/cli_tools/`)
Modify `run_search_hybrid_context`, `run_find_callers`, and `run_explore_file`:
- Instead of returning a `String` formatted as Markdown, return a generic struct or the raw `serde_json::Value` (or specific domain models).
- The `knot-mcp.rs` binary will explicitly request the `Markdown` formatter.
- The `knot.rs` binary will use the user's requested formatter.

#### Formatters:
- **JSON:** `serde_json::to_string_pretty(&data)`
- **Markdown:** Preserve the current formatting logic.
- **Table:** Use `comfy-table` to build beautiful tables.
  - Example for `search`: Columns `[Score, Kind, Name, File, Line]`.
  - Example for `callers`: Columns `[Relationship, Kind, Name, File, Line]`.
  - Add colored headers using the `colored` crate.

---

## 4. Feature 3: Interactive Result Navigation (Pager)

**Objective:** Prevent long tables from flooding the terminal buffer.

### 4.1. Implementation (`src/bin/knot.rs`)
When the CLI finishes generating a `Table` or `Markdown` string, and standard output is a TTY:
1. Attempt to pipe the output to `less -R` (or similar standard pager).
2. If `less` is unavailable, or output is redirected to a file (`!stdout.is_terminal()`), print directly to standard output.

```rust
use std::io::Write;
use std::process::{Command, Stdio};

fn print_with_pager(content: &str) {
    if atty::is(atty::Stream::Stdout) {
        if let Ok(mut child) = Command::new("less")
            .arg("-R") // support ANSI colors
            .arg("-X") // leave content on screen after exit
            .stdin(Stdio::piped())
            .spawn() 
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(content.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
    // Fallback
    println!("{}", content);
}
```

---

## 5. Execution Sequence

1. **Step 1:** Implement `--custom-ca-certs` in config and inject `SSL_CERT_FILE` in the binaries. Test with a dummy cert path.
2. **Step 2:** Add `comfy-table` and `colored` dependencies.
3. **Step 3:** Refactor `cli_tools` to return raw data instead of pre-formatted Markdown.
4. **Step 4:** Implement `Table`, `Json`, and `Markdown` renderers.
5. **Step 5:** Wire the `--output` flag in the CLI (`knot.rs`).
6. **Step 6:** Implement the pager logic for long table outputs.
7. **Step 7:** Run `cargo clippy` and `cargo fmt`. Verify all 308+ unit tests and E2E tests still pass (especially ensuring MCP Markdown output remains identical to before).

---

## 6. Performance Considerations

**Performance Optimization:** Implement token_tree result caching if performance becomes an issue with large codebases containing many macro invocations. The current token_tree extraction parses each macro invocation at query time; a simple `HashMap<(u64, u64), Vec<String>>` cache keyed by `(file_id, node_offset)` could avoid redundant parsing for frequently-accessed macro bodies.

(End of file - total 131 lines)
