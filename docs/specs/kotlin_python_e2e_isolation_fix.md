# Fix: Kotlin & Python E2E Test Isolation in Shared-DB Mode

## Problem

`run_kotlin_e2e.sh` and `run_python_e2e.sh` fail in CI when run via `run_all_e2e_fast.sh` (shared-DB mode), but pass locally when run standalone.

### Root Cause: `REPO_NAME` mismatch between indexing and MCP queries

| Script | `REPO_NAME` used for indexing | `repo_name` hardcoded in MCP queries |
|---|---|---|
| `run_kotlin_e2e.sh` | `e2e_test_repo` | `kotlin_e2e_test_repo` ❌ |
| `run_python_e2e.sh` | `e2e_test_repo` | `python_e2e_test_repo` ❌ |
| `run_rust_e2e.sh` | `rust_e2e_test_repo` | `rust_e2e_test_repo` ✓ |

In shared-DB mode (`KNOT_E2E_EXTERNAL_DB` set), `--clean` is NOT passed to the indexer. `run_e2e.sh` already indexed all 39 files under `e2e_test_repo`, so Kotlin/Python suites see "39 files unchanged" and skip re-indexing. Their MCP queries then search for `kotlin_e2e_test_repo` / `python_e2e_test_repo` which don't exist → empty results → test failure.

### Secondary Issues (Python)

- **Test 24** uses `docker exec knot_neo4j_e2e cypher-shell ...` directly (bypasses MCP/CLI, assumes a specific container name, has no `repo_name` filter — queries globally across all repos).
- **Cleanup function order** in Python differs from Kotlin/Rust (tears down Docker before reporting exit code in shared-DB mode).
- **Shared Qdrant collection** `knot_e2e_test` — no vector-space isolation from `run_e2e.sh` entities.

## Solution: Align with `run_rust_e2e.sh` Pattern

`run_rust_e2e.sh` is the reference implementation. It solves these problems by:

1. Using a **unique `REPO_NAME`** (`rust_e2e_test_repo`) consistent between indexing and queries.
2. Using a **unique `QDRANT_COLLECTION`** (`knot_rust_e2e_test`) for vector isolation.
3. **Isolating the indexed files** into a temporary directory (`$TMP_REPO_DIR`) so the indexer always has new files to process (no "unchanged" false negatives).
4. Using **shell variable expansion** (`"$REPO_NAME"`) in MCP JSON requests instead of hardcoded strings.

---

## Changes: `run_kotlin_e2e.sh`

### 1. Update configuration variables (lines 36–37)

```bash
# Before
QDRANT_COLLECTION="knot_e2e_test"
REPO_NAME="e2e_test_repo"

# After
QDRANT_COLLECTION="knot_kotlin_e2e_test"
REPO_NAME="kotlin_e2e_test_repo"
```

### 2. Add isolated temp directory variable (after line 29)

```bash
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_kotlin_repo"
```

### 3. Create isolated directory before indexing (Step 3)

```bash
# Before cargo run
rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp "$TEST_FILES_DIR/sample.kt" "$TMP_REPO_DIR/"
export KNOT_REPO_PATH="$TMP_REPO_DIR"
```

### 4. Update `KT_FILE` to use `$TMP_REPO_DIR`

```bash
KT_FILE="$TMP_REPO_DIR/sample.kt"
```

### 5. Replace all hardcoded `"repo_name":"kotlin_e2e_test_repo"` with `"$REPO_NAME"`

All ~12 MCP request JSON strings that contain `"repo_name":"kotlin_e2e_test_repo"` must be changed to use shell variable expansion: `"repo_name\":\"$REPO_NAME\"`.

### 6. Update all `KNOT_REPO_PATH="$TEST_FILES_DIR"` in MCP env invocations to `"$TMP_REPO_DIR"`

### 7. Update `cleanup()` to also remove `$TMP_REPO_DIR`

```bash
rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
```

---

## Changes: `run_python_e2e.sh`

### 1. Update configuration variables (lines 38–39)

```bash
# Before
QDRANT_COLLECTION="knot_e2e_test"
REPO_NAME="e2e_test_repo"

# After
QDRANT_COLLECTION="knot_python_e2e_test"
REPO_NAME="python_e2e_test_repo"
```

### 2. Add isolated temp directory variable

```bash
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_python_repo"
```

### 3. Create isolated directory before indexing (Step 3)

```bash
rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp "$TEST_FILES_DIR/sample.py" "$TMP_REPO_DIR/"
export KNOT_REPO_PATH="$TMP_REPO_DIR"
```

### 4. Update `PY_FILE` to use `$TMP_REPO_DIR`

```bash
PY_FILE="$TMP_REPO_DIR/sample.py"
```

### 5. Replace all hardcoded `"repo_name":"python_e2e_test_repo"` with `"$REPO_NAME"`

### 6. Update all `KNOT_REPO_PATH="$TEST_FILES_DIR"` in MCP env invocations to `"$TMP_REPO_DIR"`

### 7. Replace Test 24 (cypher-shell) with MCP `find_callers`

```bash
# Before (fragile: Docker exec, no repo_name filter)
CLASS_METHOD_CALLERS=$(docker exec knot_neo4j_e2e cypher-shell \
    -u neo4j -p e2e_test_password \
    "MATCH (c:Entity)-[:CALLS]->(t:Entity {fqn: 'MyLoraLoader.lib_load_lora'}) RETURN c.name" 2>/dev/null)
if echo "$CLASS_METHOD_CALLERS" | grep -q "load_lora_model_only"; then ...

# After (consistent with all other tests)
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":24,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"lib_load_lora\",\"repo_name\":\"$REPO_NAME\"}}}"
CLASS_METHOD_CALLERS=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" ... KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
if echo "$CLASS_METHOD_CALLERS" | grep -qi "load_lora_model_only\|MyLoraLoader"; then ...
```

### 8. Align `cleanup()` with Kotlin/Rust pattern

Move Docker teardown to *after* the exit-code check, and add `$TMP_REPO_DIR` to cleanup:

```bash
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Python E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Python E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}
```

---

## Why Isolated Directory is Necessary in Shared-DB Mode

Without isolation, the shared-DB mode hits this sequence:

1. `run_e2e.sh` indexes all 39 files under `e2e_test_repo`, writes `.knot/index_state.json` into `tests/testing_files/`.
2. Kotlin/Python suites run `knot-indexer` against the same `tests/testing_files/` — the indexer reads the state file, sees "39 unchanged", and exits without indexing anything under the new `REPO_NAME`.
3. MCP queries for the new `REPO_NAME` return zero results.

With the isolated directory, each suite:
- Creates a fresh directory with only its own file(s)
- Has no pre-existing `.knot/index_state.json`
- Forces a full index of its file under its own `REPO_NAME`
- Uses its own Qdrant collection for complete vector isolation

## Verification

After implementing, reproduce CI locally:

```bash
# Start shared databases
cd tests && docker compose -f docker-compose.e2e.yml up -d
# Wait for services, then:
cd ..

# Simulate shared-DB orchestration order
KNOT_E2E_EXTERNAL_DB=1 ./tests/run_e2e.sh
KNOT_E2E_EXTERNAL_DB=1 ./tests/run_rust_e2e.sh
KNOT_E2E_EXTERNAL_DB=1 ./tests/run_kotlin_e2e.sh   # Must pass
KNOT_E2E_EXTERNAL_DB=1 ./tests/run_python_e2e.sh   # Must pass

# Or use the fast orchestrator directly
./tests/run_all_e2e_fast.sh
```
