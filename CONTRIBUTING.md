# Contributing to knot

Contributions are welcome! Please ensure:

- All code passes `cargo clippy --all-targets -- -D warnings`
- Code is formatted with `cargo fmt`
- Changes are compatible with Rust 2024 edition (requires Rust 1.90+)
- Unit tests are added for new functionality
- E2E regression tests are added for bug fixes
- Performance regressions are validated with the benchmark framework before submitting PRs
- No `unsafe` code is introduced (except in unavoidable ONNX Runtime interop)

## Local Environment Setup

To run `knot` locally, you need Docker to spin up the required databases and the Rust toolchain to build the project.

1. **Prerequisites**: Ensure you have Docker (20.10+) and Rust 1.90+ installed.
2. **Clone the repository**:
   ```bash
   git clone https://github.com/raultov/knot
   cd knot
   ```
3. **Start the databases** (Qdrant for vector search and Neo4j for graph relationships):
   ```bash
   docker compose up -d
   ```
   *(Note: For running E2E tests, the scripts will automatically spin up ephemeral databases using `tests/docker-compose.e2e.yml`)*
4. **Configure environment variables**:
   ```bash
   cp .env.example .env
   # Edit .env to set KNOT_REPO_PATH and KNOT_NEO4J_PASSWORD
   ```
5. **Build the project**:
   ```bash
   cargo build --release
   ```

## Running Tests

`knot` employs a rigorous testing strategy including unit, end-to-end (E2E), and performance benchmarks.

### Unit Tests
Unit tests are typically located inline within the source modules and define the contract for parsing logic.
```bash
# Run all unit tests
cargo test

# Run tests for a specific module
cargo test --lib pipeline::parser
```

### End-to-End (E2E) Tests
E2E tests validate the full ingestion and querying pipeline. They are written first (BDD) and must fail before the logic is implemented.
```bash
# Run all E2E language suites
./tests/run_all_e2e_fast.sh

# Run a specific language suite
./tests/run_typescript_e2e.sh
./tests/run_java_e2e.sh
./tests/run_javascript_e2e.sh
./tests/run_web_e2e.sh
./tests/run_rust_e2e.sh
./tests/run_kotlin_e2e.sh
```

### Performance Benchmarks
To validate optimizations and detect regressions:
```bash
# 1. Run Criterion unit benchmarks (throughput per language, etc.)
cargo bench --bench pipeline_bench
cargo bench --bench graph_upsert_bench

# 2. Run E2E integration benchmarks (captures full pipeline metrics)
./tests/benchmark_e2e.sh --focus rust_e2e --output-dir /tmp/perf_results

# 3. Compare against the baseline to ensure no regressions
scripts/compare_perf_metrics.sh /tmp/perf_results .perf_metrics/baseline.json
```

## Architecture Overview

`knot` follows a high-performance, dual-database architecture:
- **Vector Database (Qdrant)**: Stores embeddings for semantic code understanding.
- **Graph Database (Neo4j)**: Stores structural relationships (e.g., call graphs, inheritance).

The project is split into three main binaries:
- `knot-indexer` (`src/bin/knot-indexer.rs`): Parses source code via Tree-sitter, extracts entities, and builds the indexes. Uses parallel streaming (MPSC channels) to overlap CPU-bound embedding with I/O-bound ingestion.
- `knot` (`src/bin/knot.rs`): The standalone CLI tool for semantic search and reverse dependency lookup.
- `knot-mcp` (`src/bin/knot-mcp.rs`): The MCP server that exposes `knot` capabilities to AI clients (like Claude or Cursor).

## Where to Start? (Easy Modules to Touch)

If you are looking to contribute, here are the most accessible areas:

- **Adding or Fixing Language Support**: Look into `src/pipeline/parser/languages/`. Each language (like `java.rs`, `rust.rs`, `python.rs`) has a dedicated file where AST parsing is defined. This is the best place to start if you want to improve how a specific language is parsed or add a new one. You'll also likely need to modify or add Tree-sitter queries in the `queries/` directory.
- **Enhancing AI Tools (MCP)**: Check out `src/mcp_tools/`. If you want to improve the prompts, tool descriptions, or formatting returned to AI agents, this is the place.
- **CLI Improvements**: Modify `src/bin/knot.rs` to add new commands or improve output formatting for terminal users.
