#!/usr/bin/env bash
# E2E Regression Test — Rust `#[cfg(test)] mod tests` indexing.
#
# Guards against the bug where test functions defined inside `#[cfg(test)]
# mod tests { ... }` blocks were silently absent from the indexed graph,
# so `find_callers` only returned production callers and lost the test
# usages (which serve as living documentation of the public API).
#
# The fix introduces:
#   1. Inline `mod` tracking — every entity defined inside `mod foo {...}`
#      receives an FQN suffix `...::foo::Entity` so `crate::lib::tests::test_x`
#      coexists with `crate::lib::tests::test_x` from a different file.
#   2. `is_test_context` flag on entities living under a `#[cfg(test)]` mod
#      so MCP consumers can tell test code apart from production code.
#
# Fixture: tests/testing_files/rust_test_module/
#   - Cargo.toml          — `test_module_repo` crate
#   - src/lib.rs          — `is_supported`, `production_caller`, and
#                            `#[cfg(test)] mod tests { ... }` with three tests
#                            plus a nested `mod nested { #[test] fn ... }`
#   - src/helpers.rs      — `helper_is_supported` plus its own `mod tests`
#
# Set KNOT_E2E_REGRESSIONS=skip to opt out (useful while wiring CI).

set -e
set -u

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.e2e.yml"
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/rust_test_module"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_rust_test_module_data"
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_rust_test_module_repo"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_rust_test_module_e2e"
REPO_NAME="rust_test_module_e2e"

if [ "${KNOT_E2E_REGRESSIONS:-run}" = "skip" ]; then
    echo -e "${YELLOW}Skipping Rust test-module E2E (set KNOT_E2E_REGRESSIONS=run to enable)${NC}"
    exit 0
fi

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Rust #[cfg(test)] mod tests E2E${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Rust test-module E2E failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# ---- Step 1: Start Docker containers ----
echo -e "${YELLOW}[1/5] Starting Docker containers...${NC}"
cd "$SCRIPT_DIR"
docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
if [ -d "$E2E_DATA_DIR" ]; then
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
fi
docker compose -f "$COMPOSE_FILE" up -d

# ---- Step 2: Wait for services ----
echo -e "${YELLOW}[2/5] Waiting for services to be ready...${NC}"

wait_for_port() {
    local port=$1
    local service=$2
    local container=$3
    local elapsed=0

    echo -n "Waiting for $service"
    while true; do
        if [ "$service" = "Neo4j" ]; then
            local status
            status=$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null || echo "starting")
            if [ "$status" = "healthy" ]; then
                echo ""
                echo -e "${GREEN}✓ $service is ready (healthy)${NC}"
                return 0
            fi
        else
            if nc -z localhost "$port" 2>/dev/null; then
                echo ""
                echo -e "${GREEN}✓ $service is ready on port $port${NC}"
                return 0
            fi
        fi

        if [ $elapsed -ge $TIMEOUT_SECONDS ]; then
            echo ""
            echo -e "${RED}ERROR: $service did not start within ${TIMEOUT_SECONDS}s${NC}"
            return 1
        fi
        sleep $HEALTH_CHECK_INTERVAL
        elapsed=$((elapsed + HEALTH_CHECK_INTERVAL))
        echo -n "."
    done
}

wait_for_port 17687 "Neo4j" "knot_neo4j_e2e"
wait_for_port 16334 "Qdrant" "knot_qdrant_e2e"
sleep 5

# ---- Step 3: Stage fixture & index ----
echo -e "${YELLOW}[3/5] Staging fixture repo and indexing...${NC}"
cd "$PROJECT_ROOT"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp -r "$TEST_FILES_DIR"/* "$TMP_REPO_DIR/"

export KNOT_REPO_PATH="$TMP_REPO_DIR"
export KNOT_REPO_NAME="$REPO_NAME"
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer..."
cargo run --release --bin knot-indexer -- --clean

echo -e "${GREEN}✓ Fixture indexed${NC}"

# ---- Step 4: BDD validation ----
echo -e "${YELLOW}[4/5] Validating test-module indexing...${NC}"

run_neo4j_cypher() {
    local query="$1"
    echo "$query" | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
        --format plain 2>/dev/null \
        | awk 'NF && NR > 1 && $0 !~ /^(Available|neo4j>|Connection|Disconnect|Connected)/ { print; exit }'
}

assert_count_ge() {
    local label="$1"
    local actual="$2"
    local expected="$3"
    actual=${actual:-0}
    if [ "$actual" -ge "$expected" ] 2>/dev/null; then
        echo -e "${GREEN}✓ ${label}: ${actual} (>= ${expected})${NC}"
        return 0
    fi
    echo -e "${RED}✗ ${label}: expected >= ${expected}, got ${actual}${NC}"
    return 1
}

assert_count_eq() {
    local label="$1"
    local actual="$2"
    local expected="$3"
    actual=${actual:-0}
    if [ "$actual" = "$expected" ] 2>/dev/null; then
        echo -e "${GREEN}✓ ${label}: ${actual} (== ${expected})${NC}"
        return 0
    fi
    echo -e "${RED}✗ ${label}: expected ${expected}, got ${actual}${NC}"
    return 1
}

# Test 1: each test function inside `mod tests` is indexed.
echo ""
echo "Test 1: All four test functions are present as entities..."
TEST_FQNS=(
    "test_module_repo::tests::test_is_supported_rs"
    "test_module_repo::tests::test_is_supported_ts"
    "test_module_repo::tests::test_is_supported_rejects_txt"
    "test_module_repo::tests::nested::test_deeply_nested_is_supported"
    "test_module_repo::helpers::tests::test_helper_is_supported_rs"
    "test_module_repo::helpers::tests::test_helper_calls_is_supported"
)
MISSING=0
for fqn in "${TEST_FQNS[@]}"; do
    CNT=$(run_neo4j_cypher "MATCH (n:Entity {fqn:'$fqn'}) RETURN count(n) AS cnt;")
    CNT=${CNT:-0}
    if [ "$CNT" -ge 1 ] 2>/dev/null; then
        echo -e "${GREEN}  ✓ ${fqn}${NC}"
    else
        echo -e "${RED}  ✗ ${fqn} — MISSING${NC}"
        MISSING=$((MISSING + 1))
    fi
done
if [ "$MISSING" -gt 0 ]; then
    echo -e "${RED}✗ ${MISSING} expected test entities are missing${NC}"
    exit 1
fi

# Test 2: production caller resolves correctly.
echo ""
echo "Test 2: production_caller --[CALLS]--> is_supported..."
PROD_CALLS=$(run_neo4j_cypher "MATCH (a:Entity {fqn:'test_module_repo::production_caller'})-[r:CALLS]->(b:Entity {fqn:'test_module_repo::is_supported'}) RETURN count(r) AS cnt;")
assert_count_ge "Production CALLS edge" "$PROD_CALLS" 1 || exit 1

# Test 3: test functions emit CALLS edges to is_supported.
echo ""
echo "Test 3: Each test function emits a CALLS edge to is_supported..."
EXPECTED_TEST_CALLERS=(
    "test_module_repo::tests::test_is_supported_rs"
    "test_module_repo::tests::test_is_supported_ts"
    "test_module_repo::tests::test_is_supported_rejects_txt"
    "test_module_repo::tests::nested::test_deeply_nested_is_supported"
)
TEST_CALL_FAILS=0
for fqn in "${EXPECTED_TEST_CALLERS[@]}"; do
    CNT=$(run_neo4j_cypher "MATCH (a:Entity {fqn:'$fqn'})-[r:CALLS]->(b:Entity {fqn:'test_module_repo::is_supported'}) RETURN count(r) AS cnt;")
    CNT=${CNT:-0}
    if [ "$CNT" -ge 1 ] 2>/dev/null; then
        echo -e "${GREEN}  ✓ ${fqn} → is_supported${NC}"
    else
        echo -e "${RED}  ✗ ${fqn} → is_supported — MISSING${NC}"
        TEST_CALL_FAILS=$((TEST_CALL_FAILS + 1))
    fi
done
if [ "$TEST_CALL_FAILS" -gt 0 ]; then
    echo -e "${RED}✗ ${TEST_CALL_FAILS} test-driven CALLS edges are missing${NC}"
    exit 1
fi

# Test 4: total CALLERS of is_supported include both production and tests.
echo ""
echo "Test 4: is_supported has >=5 callers (1 production + 4 tests + helper crossover)..."
TOTAL_CALLERS=$(run_neo4j_cypher "MATCH (a:Entity)-[r:CALLS]->(b:Entity {fqn:'test_module_repo::is_supported'}) RETURN count(DISTINCT a) AS cnt;")
assert_count_ge "Distinct callers of is_supported" "$TOTAL_CALLERS" 5 || exit 1

# Test 5: entities under #[cfg(test)] mod tests carry is_test_context = true.
echo ""
echo "Test 5: Test entities expose is_test_context = true..."
TEST_FLAG_CNT=$(run_neo4j_cypher "MATCH (n:Entity {fqn:'test_module_repo::tests::test_is_supported_rs'}) RETURN n.is_test_context AS flag;")
if [ "$TEST_FLAG_CNT" = "TRUE" ] || [ "$TEST_FLAG_CNT" = "true" ]; then
    echo -e "${GREEN}✓ test_is_supported_rs.is_test_context = true${NC}"
else
    echo -e "${RED}✗ test_is_supported_rs.is_test_context expected true, got '${TEST_FLAG_CNT}'${NC}"
    exit 1
fi

PROD_FLAG=$(run_neo4j_cypher "MATCH (n:Entity {fqn:'test_module_repo::is_supported'}) RETURN n.is_test_context AS flag;")
if [ "$PROD_FLAG" = "FALSE" ] || [ "$PROD_FLAG" = "false" ] || [ -z "$PROD_FLAG" ] || [ "$PROD_FLAG" = "NULL" ] || [ "$PROD_FLAG" = "null" ]; then
    echo -e "${GREEN}✓ is_supported.is_test_context is falsy (production code)${NC}"
else
    echo -e "${RED}✗ is_supported.is_test_context expected falsy, got '${PROD_FLAG}'${NC}"
    exit 1
fi

# Test 6: helpers::tests namespace is distinct from root::tests.
echo ""
echo "Test 6: helpers::tests and tests are distinct namespaces..."
ROOT_TESTS_CNT=$(run_neo4j_cypher "MATCH (n:Entity {fqn:'test_module_repo::tests'}) RETURN count(n) AS cnt;")
HELP_TESTS_CNT=$(run_neo4j_cypher "MATCH (n:Entity {fqn:'test_module_repo::helpers::tests'}) RETURN count(n) AS cnt;")
assert_count_eq "test_module_repo::tests RustModule entity" "$ROOT_TESTS_CNT" 1 || exit 1
assert_count_eq "test_module_repo::helpers::tests RustModule entity" "$HELP_TESTS_CNT" 1 || exit 1

# ---- Step 5: Summary ----
echo ""
echo -e "${YELLOW}[5/5] All Rust test-module regression tests passed!${NC}"
echo ""
echo "Summary:"
echo "  - 6 test functions across two files & nested module are indexed under"
echo "    qualified FQNs like crate::module::tests::name (or nested::name)."
echo "  - CALLS edges from every test caller reach is_supported."
echo "  - Production-only caller remains intact."
echo "  - is_test_context flag distinguishes test code from production code."
echo "  - tests modules at different file levels are kept as distinct namespaces."
echo ""
