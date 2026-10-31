#!/usr/bin/env bash
# E2E Integration Test Script for incremental re-indexing (issue #53)
#
# A copy of testing_files/incremental/v1 is indexed, then replaced by v2 and
# re-indexed incrementally (no --clean). In v2, Ledger.post moved down a few
# lines, Ledger.audit was removed and Obsolete.java was deleted. Every entity
# of a modified file must be replaced, never left next to its new copy, and
# the edges from unchanged files (Invoicer.bill → post) must follow it, while
# those into removed entities (Auditor.check → audit) disappear.
#
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes v1 (full run)
# 3. Replaces the tree with v2 and re-indexes incrementally
# 4. Verifies explore_file, find_callers (CLI + MCP) and search see one copy
#    of each entity, and none of the removed ones
#
# Usage: ./tests/run_incremental_e2e.sh
# Requirements: docker, docker-compose

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
FIXTURES_DIR="$SCRIPT_DIR/testing_files/incremental"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_incremental_data"
WORK_DIR="$(mktemp -d)"
REPO_NAME="incremental_e2e_repo"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_incremental_e2e_test"

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot incremental re-index E2E Test (#53)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    local exit_code=$?
    rm -rf "$WORK_DIR"

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Incremental E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up incremental E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/4] Starting Docker containers...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/4] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set; expecting shared DB)${NC}"
fi

if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[2/4] Skipping wait (KNOT_E2E_EXTERNAL_DB set; orchestrator manages readiness)${NC}"
else
    echo -e "${YELLOW}[2/4] Waiting for services to be ready...${NC}"

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

cd "$PROJECT_ROOT"
echo "Building knot-indexer, knot and knot-mcp..."
if [[ -z "${KNOT_SKIP_BUILD:-}" ]]; then
    cargo build --release --bin knot-indexer --bin knot --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
fi

LEDGER_FQN="com.example.knot.incr.Ledger"
LEDGER_FILE="src/main/java/com/example/knot/incr/Ledger.java"

index_work_tree() {
    KNOT_REPO_PATH="$WORK_DIR" KNOT_REPO_NAME="$REPO_NAME" ./target/release/knot-indexer "$@" > /dev/null 2>&1
}

echo -e "${YELLOW}[3/4] Indexing v1, then v2 incrementally...${NC}"
cp -r "$FIXTURES_DIR/v1/." "$WORK_DIR/"
index_work_tree --clean
echo -e "${GREEN}✓ v1 indexed${NC}"
auditor_audit_refs() {
    ./target/release/knot explore "src/main/java/com/example/knot/incr/Auditor.java" -r "$REPO_NAME" -o json 2>/dev/null \
        | jq -r '[.outgoing_references[]? | tostring | select(test("audit"))] | length'
}
V1_AUDIT_REFS=$(auditor_audit_refs)
if [ "$V1_AUDIT_REFS" -lt 1 ]; then
    echo -e "${RED}✗ v1: Auditor.check must reference Ledger.audit before the change${NC}"
    exit 1
fi
rm -rf "$WORK_DIR/src"
cp -r "$FIXTURES_DIR/v2/." "$WORK_DIR/"
index_work_tree
echo -e "${GREEN}✓ v2 indexed incrementally${NC}"

fail() {
    echo -e "${RED}✗ $1${NC}"
    shift
    for detail in "$@"; do echo "$detail"; done
    exit 1
}

# Distinct start lines of the entities whose `$2` field is `$3` in the JSON
# answer `$1` (explore_file rows carry `name`, search rows `fqn`).
start_lines() {
    echo "$1" | jq -r --arg field "$2" --arg value "$3" \
        '[.. | objects | select(.[$field]? == $value and .start_line? != null) | .start_line] | unique | map(tostring) | join(",")'
}

echo -e "${YELLOW}[4/4] Validating the index after the incremental run...${NC}"

echo ""
echo "Test 1: explore_file lists each entity of the modified file once, and not the removed one..."
EXPLORE=$(./target/release/knot explore "$LEDGER_FILE" -r "$REPO_NAME" -o json 2>/dev/null)
CLASS_LINES=$(start_lines "$EXPLORE" name "Ledger")
POST_LINES=$(start_lines "$EXPLORE" name "post")
AUDIT_LINES=$(start_lines "$EXPLORE" name "audit")
[ "$CLASS_LINES" = "4" ] || fail "explore: class Ledger expected at line [4] only, got [$CLASS_LINES]" "$EXPLORE"
[ "$POST_LINES" = "8" ] || fail "explore: Ledger.post expected at line [8] only, got [$POST_LINES]" "$EXPLORE"
[ -z "$AUDIT_LINES" ] || fail "explore: the removed Ledger.audit is still indexed at [$AUDIT_LINES]" "$EXPLORE"
echo -e "${GREEN}✓ CLI: Ledger once (line 4), Ledger.post once (line 8), Ledger.audit gone${NC}"

echo ""
echo "Test 2: find_callers resolves the moved method to one target with one caller..."
CALLERS=$(./target/release/knot callers "$LEDGER_FQN.post" -r "$REPO_NAME" -o json 2>/dev/null)
TARGETS=$(echo "$CALLERS" | jq -r '[.resolution.targets[].start_line] | map(tostring) | join(",")')
CALLS=$(echo "$CALLERS" | jq -r '[.calls[].name] | join(",")')
[ "$TARGETS" = "8" ] || fail "callers: Ledger.post expected one target at line [8], got [$TARGETS]" "$CALLERS"
[ "$CALLS" = "bill" ] || fail "callers: Ledger.post expected callers [bill], got [$CALLS]" "$CALLERS"
echo -e "${GREEN}✓ CLI: one target (line 8), callers [bill]${NC}"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$LEDGER_FQN.post\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_TEXT=$(echo "$MCP_REQUEST" | KNOT_REPO_PATH="$WORK_DIR" ./target/release/knot-mcp 2>/dev/null | tail -n 1 | jq -r '.result.content[0].text')
echo "$MCP_TEXT" | grep -q "^Resolved to 1 target" || fail "MCP: Ledger.post must resolve to one target" "$MCP_TEXT"
MCP_ROWS=$(echo "$MCP_TEXT" | grep -c '^- \*\*`bill`' || true)
[ "$MCP_ROWS" = "1" ] || fail "MCP: expected one bill row, got $MCP_ROWS" "$MCP_TEXT"
echo -e "${GREEN}✓ MCP: one target, one caller row${NC}"

echo ""
echo "Test 3: the removed method and the deleted file are gone..."
AUDIT_TARGETS=$(./target/release/knot callers "$LEDGER_FQN.audit" -r "$REPO_NAME" -o json 2>/dev/null \
    | jq -r '[.resolution.targets[]?] | length')
[ "$AUDIT_TARGETS" = "0" ] || fail "callers: the removed Ledger.audit still resolves to $AUDIT_TARGETS target(s)"
CHECK_CALLS=$(auditor_audit_refs)
[ "$CHECK_CALLS" = "0" ] || fail "explore: the unchanged Auditor.check still points at the removed Ledger.audit"
OBSOLETE=$(./target/release/knot explore "src/main/java/com/example/knot/incr/Obsolete.java" -r "$REPO_NAME" -o json 2>/dev/null)
OBSOLETE_LINES=$(start_lines "$OBSOLETE" name "retire")
[ -z "$OBSOLETE_LINES" ] || fail "explore: Obsolete.retire of the deleted file is still indexed" "$OBSOLETE"
echo -e "${GREEN}✓ CLI: Ledger.audit, its edge from the unchanged Auditor and Obsolete.java are gone${NC}"

echo ""
echo "Test 4: the vector index holds one copy of the moved method..."
SEARCH=$(./target/release/knot search "ledger post records one entry" -r "$REPO_NAME" -m 20 -o json 2>/dev/null)
SEARCH_LINES=$(start_lines "$SEARCH" fqn "$LEDGER_FQN.post")
[ "$SEARCH_LINES" = "8" ] || fail "search: Ledger.post expected at line [8] only, got [$SEARCH_LINES]" "$SEARCH"
SEARCH_AUDIT=$(start_lines "$SEARCH" fqn "$LEDGER_FQN.audit")
[ -z "$SEARCH_AUDIT" ] || fail "search: the removed Ledger.audit is still in the vector index at [$SEARCH_AUDIT]" "$SEARCH"
echo -e "${GREEN}✓ CLI: search returns Ledger.post once (line 8), no Ledger.audit${NC}"

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All incremental E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated incremental re-indexing:"
echo "  ✓ Entities of a modified file are replaced, not duplicated (graph and vectors)"
echo "  ✓ Entities removed from a modified file and deleted files disappear"
echo ""

exit 0
