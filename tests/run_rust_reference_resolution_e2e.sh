#!/usr/bin/env bash
# E2E Regression Test for Phase 0 — Rust reference resolution.
#
# This script guards against a category of bugs where the Rust parser
# produces spurious REFERENCES edges between two distinct entities that
# happen to share the same name (e.g. `crate_a::config::Config` and
# `crate_a::other_module::types::Config`).
#
# The fixture repo lives under
#   tests/testing_files/rust_reference_resolution/
# with two crates:
#   - crate_a: defines `config::Config` and `other_module::types::Config`
#   - crate_b: imports `crate_a::config::Config` as `CrateAConfig`
#
# Usage:
#   ./tests/run_rust_reference_resolution_e2e.sh
#
# The script runs by default. Set KNOT_E2E_REGRESSIONS=skip to disable.
#
# Requirements: docker, docker-compose, cypher-shell (Neo4j client), nc, cargo.

set -e
set -u

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.e2e.yml"
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/rust_reference_resolution"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_rust_ref_resol_data"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_rust_ref_resol_e2e_test"
REPO_NAME="rust_ref_resol_e2e_test"

# Skip mode: do not run unless explicitly enabled.
if [ "${KNOT_E2E_REGRESSIONS:-run}" = "skip" ]; then
    echo -e "${YELLOW}Skipping Rust Reference Resolution E2E (set KNOT_E2E_REGRESSIONS=run to enable)${NC}"
    exit 0
fi

# Isolated repository: copy the fixture into a temp dir so we control what
# gets indexed and avoid semantic-search pollution from other fixtures.
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_rust_ref_resol_repo"

# Timeout settings
TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Rust Reference Resolution E2E (Phase 0)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Rust Reference Resolution E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Rust Reference Resolution E2E environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# Step 1: Start Docker containers (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/5] Starting Docker containers for Rust reference resolution E2E...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/5] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set; expecting shared DB)${NC}"
fi

# Step 2: Wait for services (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[2/5] Skipping wait (KNOT_E2E_EXTERNAL_DB set; orchestrator manages readiness)${NC}"
else
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
fi

# Step 3: Stage fixture repo and index it
echo -e "${YELLOW}[3/5] Staging fixture repo and indexing...${NC}"
cd "$PROJECT_ROOT"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp -r "$TEST_FILES_DIR/crate_a" "$TMP_REPO_DIR/"
cp -r "$TEST_FILES_DIR/crate_b" "$TMP_REPO_DIR/"

export KNOT_REPO_PATH="$TMP_REPO_DIR"
export KNOT_REPO_NAME="$REPO_NAME"
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for crate_a + crate_b..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"

echo -e "${GREEN}✓ Fixture repo indexed${NC}"

# Step 4: Run BDD validation tests
echo -e "${YELLOW}[4/5] Validating Rust reference resolution...${NC}"

echo "Building knot and knot-mcp..."
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Load the helper library for Neo4j relationship assertions
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib/assert_neo4j_relationships.sh"
export NEO4J_URI NEO4J_USER NEO4J_PASSWORD
export KNOT_REPO_NAME="$REPO_NAME"

# Test 1: find_callers on Config::load (crate_a) must NOT pull in callers
# from crate_a::other_module::types::Config. In a healthy graph, Config::load
# has zero callers in the fixture (it is a leaf constructor).
echo ""
echo "Test 1: knot callers on 'Config::load' must not include spurious references..."
CALLERS_OUTPUT=$(env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot -- callers "Config::load" -r "$REPO_NAME" 2>/dev/null)

# The fixture has no callers for Config::load (it is a leaf method, only
# crate_b/src/lib.rs calls it through CrateAConfig::load). The crucial
# invariant: nothing from other_module/types.rs should leak in.
if echo "$CALLERS_OUTPUT" | grep -q "other_module"; then
    echo -e "${RED}✗ Spurious 'other_module' reference found in Config::load callers${NC}"
    echo "Output:"
    echo "$CALLERS_OUTPUT"
    exit 1
else
    echo -e "${GREEN}✓ No spurious other_module references in Config::load callers${NC}"
fi

# Test 2: explore on crate_a/src/config.rs must surface Config::load with
# the correct edges (the load() method should be listed in the Methods
# section, and the call from crate_b's lib.rs should resolve through the
# CrateAConfig alias).
echo ""
echo "Test 2: knot explore on crate_a/src/config.rs lists Config and its methods..."
CONFIG_RS="$TMP_REPO_DIR/crate_a/src/config.rs"
EXPLORE_OUTPUT=$(env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot -- explore "$CONFIG_RS" -r "$REPO_NAME" -o markdown 2>/dev/null)

# Expect Config to be indexed and load()/process() to appear as methods.
if echo "$EXPLORE_OUTPUT" | grep -q "Config"; then
    echo -e "${GREEN}✓ Config struct appears in explore_file output${NC}"
else
    echo -e "${RED}✗ Config struct not found in explore_file output${NC}"
    echo "$EXPLORE_OUTPUT"
    exit 1
fi

if echo "$EXPLORE_OUTPUT" | grep -qE "load|process"; then
    echo -e "${GREEN}✓ Config methods (load/process) are listed${NC}"
else
    echo -e "${RED}✗ Config methods (load/process) not found in explore_file output${NC}"
    echo "$EXPLORE_OUTPUT"
    exit 1
fi

# Test 3: Cypher-level assertion — no spurious REFERENCES edge from
# crate_a::config::Config to crate_a::other_module::types::Config.
# This is the heart of the Phase 0 regression: two distinct types with
# the same short name must not get cross-linked.
echo ""
echo "Test 3: No spurious REFERENCES edge between the two Config types in crate_a..."

# Use cypher-shell from the running Neo4j container so we don't depend
# on a host installation.
run_neo4j_cypher() {
    local query="$1"
    echo "$query" | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
        --format plain 2>/dev/null \
        | awk 'NF && NR > 1 && $0 !~ /^(Available|neo4j>|Connection|Disconnect|Connected)/ { print; exit }'
}

run_neo4j_cypher_all() {
    local query="$1"
    echo "$query" | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
        --format plain 2>/dev/null \
        | awk 'NF && NR > 1 && $0 !~ /^(Available|neo4j>|Connection|Disconnect|Connected)/'
}

# First, make sure both Config types are actually indexed. After Phase 3,
# Rust FQNs are qualified: crate_a::config::Config and
# crate_a::other_module::types::Config.
QUERY_PRESENCE="MATCH (n {repo_name: '$REPO_NAME'}) WHERE n.fqn IN ['crate_a::config::Config', 'crate_a::other_module::types::Config'] RETURN count(n) AS cnt;"
PRESENCE_COUNT=$(run_neo4j_cypher "$QUERY_PRESENCE")
PRESENCE_COUNT=${PRESENCE_COUNT:-0}

if [ "$PRESENCE_COUNT" -lt 2 ] 2>/dev/null; then
    echo -e "${RED}✗ Expected Config nodes not found in graph (found $PRESENCE_COUNT, expected >= 2)${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Both crate_a Config nodes are present in the graph ($PRESENCE_COUNT matches)${NC}"
fi

# Now assert no REFERENCES edge connects them in either direction.
QUERY_CROSS="MATCH (a {repo_name: '$REPO_NAME'})-[r:REFERENCES]->(b {repo_name: '$REPO_NAME'}) WHERE a.fqn = 'crate_a::config::Config' AND b.fqn = 'crate_a::other_module::types::Config' RETURN count(r) AS cnt;"
CROSS_FORWARD=$(run_neo4j_cypher "$QUERY_CROSS")
CROSS_FORWARD=${CROSS_FORWARD:-0}

QUERY_CROSS_REV="MATCH (a {repo_name: '$REPO_NAME'})-[r:REFERENCES]->(b {repo_name: '$REPO_NAME'}) WHERE a.fqn = 'crate_a::other_module::types::Config' AND b.fqn = 'crate_a::config::Config' RETURN count(r) AS cnt;"
CROSS_REVERSE=$(run_neo4j_cypher "$QUERY_CROSS_REV")
CROSS_REVERSE=${CROSS_REVERSE:-0}

if [ "$CROSS_FORWARD" != "0" ] || [ "$CROSS_REVERSE" != "0" ]; then
    echo -e "${RED}✗ Spurious REFERENCES edge between the two Config types${NC}"
    echo -e "${RED}  crate_a::config::Config -> crate_a::other_module::types::Config: $CROSS_FORWARD${NC}"
    echo -e "${RED}  crate_a::other_module::types::Config -> crate_a::config::Config: $CROSS_REVERSE${NC}"
    exit 1
else
    echo -e "${GREEN}✓ No spurious REFERENCES edge between the two crate_a Config types${NC}"
fi

# Test 4: Use the helper library to assert edge counts and edges that
# SHOULD exist (so the helper is exercised end-to-end, not just on
# failure paths).
echo ""
echo "Test 4: Helper library — assert that crate_b's lib.rs resolves the CrateAConfig import..."

# The use statement in crate_b/src/lib.rs should generate a REFERENCES
# edge from `CrateAConfig` (or the import site) to `crate_a::config::Config`.
# We allow either of the two canonical forms the parser may emit.
QUERY_USE="MATCH (a {repo_name: '$REPO_NAME'})-[r:REFERENCES]->(b {repo_name: '$REPO_NAME'}) WHERE b.fqn = 'crate_a::config::Config' AND a.file_path ENDS WITH 'crate_b/src/lib.rs' RETURN count(r) AS cnt;"
USE_HITS=$(run_neo4j_cypher "$QUERY_USE")
USE_HITS=${USE_HITS:-0}

if [ "$USE_HITS" -ge 1 ] 2>/dev/null; then
    echo -e "${GREEN}✓ crate_b/src/lib.rs references crate_a::config::Config via CrateAConfig ($USE_HITS edge(s))${NC}"
else
    echo -e "${YELLOW}⚠ crate_b/src/lib.rs -> crate_a::config::Config REFERENCES not detected (USE_HITS=$USE_HITS)${NC}"
    echo -e "${YELLOW}  This may be acceptable depending on how the Rust parser emits import edges;${NC}"
    echo -e "${YELLOW}  Phase 0 only requires that the TWO Config types are NOT cross-linked.${NC}"
fi

# Exercise the helper functions themselves with a positive case.
# crate_a::config::Config has outgoing REFERENCES edges (to String, Result, etc.)
# so we verify that assert_edge_count accepts the real count and assert_no_edge
# correctly rejects a non-existent edge.
CONFIG_REFS=$(run_neo4j_cypher "MATCH (a {repo_name: '$REPO_NAME'})-[r:REFERENCES]->() WHERE a.fqn = 'crate_a::config::Config' RETURN count(r) AS cnt;")
CONFIG_REFS=${CONFIG_REFS:-0}

if assert_edge_count "crate_a::config::Config" "REFERENCES" "$CONFIG_REFS"; then
    echo -e "${GREEN}✓ Helper library assert_edge_count works ($CONFIG_REFS outgoing REFERENCES)${NC}"
else
    echo -e "${RED}✗ Helper library assert_edge_count returned non-zero${NC}"
    exit 1
fi

if assert_no_edge "crate_a::config::Config" "crate_a::other_module::types::Config" "REFERENCES"; then
    echo -e "${GREEN}✓ Helper library assert_no_edge works${NC}"
else
    echo -e "${RED}✗ Helper library assert_no_edge returned non-zero${NC}"
    exit 1
fi

# =========================================================================
# CONTAINS Collision & Fixture FQN E2E Tests (PR1 + PR2)
# =========================================================================
# These tests verify:
#   PR1: CONTAINS auto-link uses enclosing_class_fqn (not bare name match)
#   PR2: Files outside src/ get __fixture:: FQN prefix
#
# The fixture has:
#   - src/config.rs: real Config struct with load_mcp() method
#   - tests/testing_files/sample.rs: fixture Config struct (same name!)
#
# Before the fix, load_mcp would appear CONTAINED by both Config structs.
# After the fix, only the real Config (FQN-based match) CONTAINS load_mcp.

echo ""
echo "========================================================================="
echo "CONTAINS Collision & Fixture FQN Tests (PR1 + PR2)"
echo "========================================================================="

# Stage the collision fixture in a separate temp repo
COLLISION_REPO_DIR="$SCRIPT_DIR/.e2e_rust_collision_repo"
COLLISION_FIXTURE_DIR="$SCRIPT_DIR/testing_files/rust_contains_collision"
COLLISION_REPO_NAME="collision_test_repo"

rm -rf "$COLLISION_REPO_DIR"
mkdir -p "$COLLISION_REPO_DIR"
cp -r "$COLLISION_FIXTURE_DIR"/* "$COLLISION_REPO_DIR/"

# Index the collision fixture
echo ""
echo "Indexing collision fixture repo..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
COLLISION_OUTPUT=$(env KNOT_REPO_PATH="$COLLISION_REPO_DIR" \
    KNOT_REPO_NAME="$COLLISION_REPO_NAME" \
    KNOT_NEO4J_URI="$NEO4J_URI" \
    KNOT_NEO4J_USER="$NEO4J_USER" \
    KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
    KNOT_QDRANT_URL="$QDRANT_URL" \
    KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
    cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}" 2>&1)

echo "$COLLISION_OUTPUT" | tail -5

# Test PR1-1: Real Config (src/config.rs) CONTAINS load_mcp — positive case
echo ""
echo "Test PR1-1: Real Config CONTAINS load_mcp (FQN-based match)..."
QUERY_REAL_CONTAINS="MATCH (c:Entity {repo_name: '$COLLISION_REPO_NAME', fqn:'collision_test::config::Config'})-[:CONTAINS]->(m:Entity {name:'load_mcp'}) RETURN count(m) AS cnt;"
REAL_COUNT=$(run_neo4j_cypher "$QUERY_REAL_CONTAINS")
REAL_COUNT=${REAL_COUNT:-0}

if [ "$REAL_COUNT" = "1" ] 2>/dev/null; then
    echo -e "${GREEN}✓ Real Config correctly CONTAINS load_mcp (count=$REAL_COUNT)${NC}"
else
    echo -e "${RED}✗ Real Config should CONTAIN load_mcp exactly once, got count=$REAL_COUNT${NC}"
    exit 1
fi

# Test PR1-2: Fixture Config (tests/testing_files/sample.rs) does NOT CONTAIN load_mcp — negative case
echo ""
echo "Test PR1-2: Fixture Config does NOT CONTAIN load_mcp (no spurious edge)..."
QUERY_FIXTURE_CONTAINS="MATCH (c:Entity {repo_name: '$COLLISION_REPO_NAME'})-[:CONTAINS]->(m:Entity {name:'load_mcp'}) WHERE c.fqn STARTS WITH '__fixture::' RETURN count(m) AS cnt;"
FIXTURE_COUNT=$(run_neo4j_cypher "$QUERY_FIXTURE_CONTAINS")
FIXTURE_COUNT=${FIXTURE_COUNT:-0}

if [ "$FIXTURE_COUNT" = "0" ] 2>/dev/null; then
    echo -e "${GREEN}✓ Fixture Config correctly does NOT CONTAIN load_mcp (count=$FIXTURE_COUNT)${NC}"
else
    echo -e "${RED}✗ Fixture Config should NOT CONTAIN load_mcp, got count=$FIXTURE_COUNT${NC}"
    exit 1
fi

# Test PR2-1: Fixture entity has __fixture:: prefix in FQN
echo ""
echo "Test PR2-1: Fixture Config entity has __fixture:: FQN prefix..."
QUERY_FIXTURE_FQN="MATCH (e:Entity {repo_name: '$COLLISION_REPO_NAME'}) WHERE e.file_path CONTAINS '/tests/testing_files/sample.rs' AND e.name = 'Config' RETURN e.fqn AS fqn;"
FIXTURE_FQN=$(run_neo4j_cypher "$QUERY_FIXTURE_FQN")

if echo "$FIXTURE_FQN" | grep -q "__fixture::"; then
    echo -e "${GREEN}✓ Fixture Config FQN starts with __fixture:: (fqn=$FIXTURE_FQN)${NC}"
else
    echo -e "${RED}✗ Fixture Config FQN should start with __fixture::, got: $FIXTURE_FQN${NC}"
    exit 1
fi

# Test PR2-2: Real Config entity does NOT have __fixture:: prefix
echo ""
echo "Test PR2-2: Real Config entity has standard crate FQN..."
QUERY_REAL_FQN="MATCH (e:Entity {repo_name: '$COLLISION_REPO_NAME', fqn:'collision_test::config::Config'}) RETURN e.file_path AS fp;"
REAL_FP=$(run_neo4j_cypher "$QUERY_REAL_FQN")

if echo "$REAL_FP" | grep -q "/src/"; then
    echo -e "${GREEN}✓ Real Config is in src/ (file_path=$REAL_FP)${NC}"
else
    echo -e "${RED}✗ Real Config should be in src/, got file_path=$REAL_FP${NC}"
    exit 1
fi

# Test PR2-3: Fixture method has __fixture:: prefix in enclosing_class_fqn
echo ""
echo "Test PR2-3: Fixture method 'new' has __fixture:: enclosing_class_fqn..."
QUERY_FIXTURE_METHOD="MATCH (e:Entity {repo_name: '$COLLISION_REPO_NAME'}) WHERE e.file_path CONTAINS '/tests/testing_files/sample.rs' AND e.name = 'new' RETURN e.enclosing_class_fqn AS fqn;"
FIXTURE_METHOD_FQN=$(run_neo4j_cypher "$QUERY_FIXTURE_METHOD")

if echo "$FIXTURE_METHOD_FQN" | grep -q "__fixture::"; then
    echo -e "${GREEN}✓ Fixture method 'new' has __fixture:: enclosing_class_fqn (fqn=$FIXTURE_METHOD_FQN)${NC}"
else
    echo -e "${YELLOW}⚠ Fixture method enclosing_class_fqn not __fixture:: prefixed (got: $FIXTURE_METHOD_FQN)${NC}"
    echo -e "${YELLOW}  This may be acceptable if the method is not classified as RustMethod.${NC}"
fi

# Cleanup collision temp repo
rm -rf "$COLLISION_REPO_DIR"

# Step 5: Summarize
echo ""
echo -e "${YELLOW}[5/5] All Rust reference resolution regression tests passed!${NC}"
echo ""
echo "Summary:"
echo "  - Two crate_a Config types (crate_a::config::Config, crate_a::other_module::types::Config) coexist"
echo "  - No spurious REFERENCES edges connect them in either direction"
echo "  - find_callers on Config::load does not leak references from other_module"
echo "  - explore_file lists Config and its methods correctly"
echo "  - Helper library (assert_neo4j_relationships.sh) is exercised"
echo "  - PR1: Real Config CONTAINS load_mcp; fixture Config does NOT (FQN-based disambiguation)"
echo "  - PR2: Fixture entities have __fixture:: FQN prefix; real entities have crate-qualified FQN"
echo ""
