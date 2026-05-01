# Contributing to knot

Contributions are welcome! Please ensure:

- All code passes `cargo clippy --all-targets -- -D warnings`
- Code is formatted with `cargo fmt`
- Changes are compatible with Rust 2024 edition
- Unit tests are added for new functionality
- E2E regression tests are added for bug fixes
- No `unsafe` code is introduced

## Development Workflow

1. **Clone the repository**
   ```bash
   git clone https://github.com/raultov/knot
   cd knot
   ```

2. **Start infrastructure** (Neo4j + Qdrant)
   ```bash
   docker compose up -d
   ```

3. **Build**
   ```bash
   cargo build --release
   ```

4. **Run unit tests**
   ```bash
   cargo test
   ```

5. **Run all E2E tests**
   ```bash
   ./tests/run_all_e2e.sh
   ```

6. **Check formatting and linting**
   ```bash
   cargo fmt -- --check
   cargo clippy --all-targets -- -D warnings
   ```

## Testing Philosophy

- **BDD**: E2E tests are written first and must fail before parser logic is implemented.
- **TDD**: Unit tests are written before implementation in parser modules.
- **E2E Regression**: All bugs must have an E2E regression case before the fix.
- **Unit Coverage**: New entity kinds, reference intents, and query patterns require unit tests.

## Project Structure

| Directory | Purpose |
|-----------|---------|
| `src/pipeline/parser/languages/` | Language-specific entity extractors |
| `queries/` | Tree-sitter query patterns (`.scm` files) |
| `tests/` | E2E integration test scripts |
| `src/bin/` | Entry points (`knot-indexer`, `knot`, `knot-mcp`) |
