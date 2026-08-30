#!/usr/bin/env bash
# E2E Integration Test Script for C# Language Support in knot
#
# 48 assertions as specified in docs/specs/csharp_reference_extraction_fix_plan.md §5.3 and find_callers_target_resolution_plan.md.
# Tests class/interface/struct/record/enum/method/property/delegate/event/indexer/
# operator/local-function/constructor extraction, FQN construction across
# file-scoped and block-form namespaces, EXTENDS/IMPLEMENTS/CALLS/REFERENCES/
# CONTAINS/OVERRIDES edges, XML doc comments, attributes, semantic search, and
# find_callers impact analysis.
#
# Usage: ./tests/run_csharp_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/csharp"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_csharp_data"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_csharp_e2e_test"
REPO_NAME="csharp_e2e_test_repo"

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"
export KNOT_REPO_PATH="$TEST_FILES_DIR"
export KNOT_REPO_NAME="$REPO_NAME"
export NEO4J_URI
export NEO4J_USER
export NEO4J_PASSWORD

source "$SCRIPT_DIR/lib/assert_neo4j_relationships.sh"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot C# E2E Integration Test${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}C# E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi
    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi
    echo -e "\n${YELLOW}Cleaning up C# E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# ── Start Docker ──────────────────────────────────────────────────────────────
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/5] Starting Docker containers for C# E2E test...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/5] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set)${NC}"
fi

# ── Wait for services ─────────────────────────────────────────────────────────
if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[2/5] Skipping wait (KNOT_E2E_EXTERNAL_DB set)${NC}"
else
    echo -e "${YELLOW}[2/5] Waiting for services to be ready...${NC}"

    wait_for_port() {
        local port=$1 service=$2 container=$3 elapsed=0
        echo -n "Waiting for $service"
        while true; do
            if [ "$service" = "Neo4j" ]; then
                local status
                status=$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null || echo "starting")
                if [ "$status" = "healthy" ]; then
                    echo ""; echo -e "${GREEN}✓ $service is ready (healthy)${NC}"; return 0
                fi
            else
                if nc -z localhost "$port" 2>/dev/null; then
                    echo ""; echo -e "${GREEN}✓ $service is ready on port $port${NC}"; return 0
                fi
            fi
            if [ $elapsed -ge $TIMEOUT_SECONDS ]; then
                echo ""; echo -e "${RED}ERROR: $service did not start within ${TIMEOUT_SECONDS}s${NC}"; return 1
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

# ── Index ─────────────────────────────────────────────────────────────────────
echo -e "${YELLOW}[3/5] Indexing C# fixture files...${NC}"
cd "$PROJECT_ROOT"

if [[ -z "${KNOT_SKIP_BUILD:-}" ]]; then
    echo "Building knot-indexer..."
    cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true
fi

echo "Running indexer for C# files..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
if [ "${KNOT_SKIP_BUILD:-0}" = "1" ]; then
    INDEXER_OUTPUT=$(./target/release/knot-indexer "${INDEXER_FLAGS[@]}" 2>&1)
else
    INDEXER_OUTPUT=$(cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}" 2>&1)
fi
echo "$INDEXER_OUTPUT"

# Verify [Progress] log lines are emitted when files are actually parsed
if echo "$INDEXER_OUTPUT" | grep -q "No files changed"; then
    echo -e "${YELLOW}⚠ No files to parse — skipping progress log checks (incremental run)${NC}"
elif echo "$INDEXER_OUTPUT" | grep -q "No supported source files found"; then
    echo -e "${YELLOW}⚠ Empty repo — skipping progress log checks${NC}"
else
    if ! echo "$INDEXER_OUTPUT" | grep -qE '\[Progress\] \[.*\] [0-9.]+% — files [0-9]+/[0-9]+'; then
        echo -e "${RED}✗ No [Progress] log line found in indexer output${NC}"
        exit 1
    fi
    if ! echo "$INDEXER_OUTPUT" | grep -q '100\.0%'; then
        echo -e "${RED}✗ No 100.0% progress line found in indexer output${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ Progress log lines verified${NC}"
fi

echo -e "${GREEN}✓ C# files indexed${NC}"

# ── Build binaries if needed ─────────────────────────────────────────────────
if [[ -z "${KNOT_SKIP_BUILD:-}" ]]; then
    cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
    cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true
fi

# ── Cypher helper ─────────────────────────────────────────────────────────────
run_neo4j_cypher() {
    echo "$1" | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
        --format plain 2>/dev/null | tail -n +2
}

# ── Assertions ────────────────────────────────────────────────────────────────
FAILURES=0
echo -e "${YELLOW}[4/5] Running assertions...${NC}"

assert_cypher_count() {
    local label="$1" query="$2" expected="$3"
    local count
    count=$(run_neo4j_cypher "$query" | tr -d ' "')
    count=${count:-0}
    if [ "$count" = "$expected" ]; then
        echo -e "${GREEN}✓ $label: $count (expected $expected)${NC}"
    else
        echo -e "${RED}✗ $label: expected $expected, got $count${NC}"
        FAILURES=$((FAILURES + 1))
    fi
}

assert_cypher_exists() {
    local label="$1" query="$2"
    local count
    count=$(run_neo4j_cypher "$query" | tr -d ' "')
    count=${count:-0}
    if [ "$count" -gt 0 ]; then
        echo -e "${GREEN}✓ $label: found ($count)${NC}"
    else
        echo -e "${RED}✗ $label: expected >0, got 0${NC}"
        FAILURES=$((FAILURES + 1))
    fi
}

# Helper for MCP invocation (returns last line of stdout)
invoke_mcp() {
    local request="$1"
    if [ "${KNOT_SKIP_BUILD:-0}" = "1" ]; then
        echo "$request" | ./target/release/knot-mcp 2>/dev/null | tail -n 1
    else
        echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1
    fi
}

# Helper for CLI invocation
invoke_cli() {
    if [ "${KNOT_SKIP_BUILD:-0}" = "1" ]; then
        ./target/release/knot "$@" 2>/dev/null
    else
        cargo run --release --bin knot -- "$@" 2>/dev/null
    fi
}

# Helper to retry search until Qdrant returns the expected token
retry_match() {
    local expected="$1"; shift
    local max_attempts=10
    local attempt=1
    while [ $attempt -le $max_attempts ]; do
        if "$@" 2>/dev/null | grep -qiE "$expected"; then
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    return 1
}

# ─────────────────────────────────────────────────────────────────────────────
# A · Entity extraction
# ─────────────────────────────────────────────────────────────────────────────

# A1: csharp_class UserService found in Services/UserService.cs
echo ""
echo "A1. csharp_class UserService in Services/UserService.cs..."
A1_FILE="Services/UserService.cs"
A1_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$A1_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"
A1_MCP=$(invoke_mcp "$A1_REQUEST")
A1_CLI=$(invoke_cli explore "$A1_FILE" -r "$REPO_NAME")
if echo "$A1_MCP" | grep -q "UserService" && echo "$A1_CLI" | grep -q "UserService"; then
    echo -e "${GREEN}✓ A1. csharp_class UserService found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ A1. csharp_class UserService not found${NC}"
    FAILURES=$((FAILURES + 1))
fi

# A2: csharp_interface IRepository found in Domain/IRepository.cs
echo ""
echo "A2. csharp_interface IRepository in Domain/IRepository.cs..."
A2_FILE="Domain/IRepository.cs"
A2_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$A2_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"
A2_MCP=$(invoke_mcp "$A2_REQUEST")
A2_CLI=$(invoke_cli explore "$A2_FILE" -r "$REPO_NAME")
if echo "$A2_MCP" | grep -q "IRepository" && echo "$A2_CLI" | grep -q "IRepository"; then
    echo -e "${GREEN}✓ A2. csharp_interface IRepository found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ A2. csharp_interface IRepository not found${NC}"
    FAILURES=$((FAILURES + 1))
fi

# A3: csharp_method GetUserAsync found
echo ""
echo "A3. csharp_method GetUserAsync found..."
assert_cypher_exists "A3. csharp_method GetUserAsync" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_method' AND e.name = 'GetUserAsync' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# A4: csharp_property ServiceLabel found, and not reported as a field
echo ""
echo "A4. csharp_property ServiceLabel (not reported as field)..."
assert_cypher_exists "A4a. csharp_property ServiceLabel exists" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_property' AND e.name = 'ServiceLabel' AND e.repo_name = '$REPO_NAME' RETURN count(e)"
assert_cypher_count "A4b. No csharp_field ServiceLabel" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_field' AND e.name = 'ServiceLabel' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "0"

# A5: csharp_record UserDto found
echo ""
echo "A5. csharp_record UserDto..."
assert_cypher_exists "A5. csharp_record UserDto" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_record' AND e.name = 'UserDto' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# A6: csharp_struct Point found
echo ""
echo "A6. csharp_struct Point..."
assert_cypher_exists "A6. csharp_struct Point" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_struct' AND e.name = 'Point' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# A7: csharp_enum UserStatus found
echo ""
echo "A7. csharp_enum UserStatus..."
assert_cypher_exists "A7. csharp_enum UserStatus" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_enum' AND e.name = 'UserStatus' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# A8: csharp_delegate Notifier and csharp_event OnNotify found in Legacy/OldStyle.cs
echo ""
echo "A8. csharp_delegate Notifier and csharp_event OnNotify in Legacy/OldStyle.cs..."
A8_FILE="Legacy/OldStyle.cs"
A8_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$A8_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"
A8_MCP=$(invoke_mcp "$A8_REQUEST")
A8_CLI=$(invoke_cli explore "$A8_FILE" -r "$REPO_NAME")
if echo "$A8_MCP" | grep -q "Notifier" && echo "$A8_CLI" | grep -q "Notifier"; then
    echo -e "${GREEN}✓ A8a. csharp_delegate Notifier found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ A8a. csharp_delegate Notifier not found${NC}"
    FAILURES=$((FAILURES + 1))
fi
if echo "$A8_MCP" | grep -q "OnNotify" && echo "$A8_CLI" | grep -q "OnNotify"; then
    echo -e "${GREEN}✓ A8b. csharp_event OnNotify found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ A8b. csharp_event OnNotify not found${NC}"
    FAILURES=$((FAILURES + 1))
fi

# A9: csharp_indexer and csharp_operator found in OldStyle
echo ""
echo "A9. csharp_indexer and csharp_operator in OldStyle..."
assert_cypher_exists "A9a. csharp_indexer in OldStyle.cs" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_indexer' AND e.file_path ENDS WITH 'OldStyle.cs' AND e.repo_name = '$REPO_NAME' RETURN count(e)"
assert_cypher_exists "A9b. csharp_operator in OldStyle.cs" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_operator' AND e.file_path ENDS WITH 'OldStyle.cs' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# A10: csharp_class StringExtensions + csharp_method Slugify + csharp_local_function Normalize
echo ""
echo "A10. csharp_class StringExtensions, csharp_method Slugify, csharp_local_function Normalize..."
assert_cypher_exists "A10a. csharp_class StringExtensions" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_class' AND e.name = 'StringExtensions' AND e.repo_name = '$REPO_NAME' RETURN count(e)"
assert_cypher_exists "A10b. csharp_method Slugify" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_method' AND e.name = 'Slugify' AND e.repo_name = '$REPO_NAME' RETURN count(e)"
assert_cypher_exists "A10c. csharp_local_function Normalize" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_local_function' AND e.name = 'Normalize' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# A11: csharp_constructor for UserService
echo ""
echo "A11. csharp_constructor for UserService..."
assert_cypher_exists "A11. csharp_constructor UserService" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_constructor' AND e.name = 'UserService' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# A12: Exact per-kind counts via assert_cypher_count — guards against double extraction
echo ""
echo "A12. Per-kind exact counts..."
assert_cypher_count "A12a. csharp_interface count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_interface' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "3"
assert_cypher_count "A12b. csharp_class count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_class' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "12"
assert_cypher_count "A12c. csharp_record count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_record' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "5"
assert_cypher_count "A12d. csharp_struct count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_struct' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "1"
assert_cypher_count "A12e. csharp_enum count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_enum' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "2"
assert_cypher_count "A12f. csharp_delegate count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_delegate' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "1"
assert_cypher_count "A12g. csharp_event count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_event' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "1"
assert_cypher_count "A12h. csharp_indexer count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_indexer' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "1"
assert_cypher_count "A12i. csharp_operator count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_operator' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "1"
assert_cypher_count "A12j. csharp_local_function count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_local_function' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "1"
assert_cypher_count "A12k. csharp_constructor count" \
    "MATCH (e:Entity) WHERE e.kind = 'csharp_constructor' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
    "3"

# ─────────────────────────────────────────────────────────────────────────────
# B · FQN and namespaces
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "B13. fqn = 'MyApp.Services.UserService'..."
assert_cypher_exists "B13. file-scoped namespace FQN" \
    "MATCH (e:Entity) WHERE e.fqn = 'MyApp.Services.UserService' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

echo ""
echo "B14. fqn = 'MyApp.Services.UserService.GetUserAsync'..."
assert_cypher_exists "B14. method FQN under file-scoped namespace" \
    "MATCH (e:Entity) WHERE e.fqn = 'MyApp.Services.UserService.GetUserAsync' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

echo ""
echo "B15. fqn = 'MyApp.Legacy.Deep.OldStyle.Inner'..."
assert_cypher_exists "B15. nested block namespace + nested type FQN" \
    "MATCH (e:Entity) WHERE e.fqn = 'MyApp.Legacy.Deep.OldStyle.Inner' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

echo ""
echo "B16. fqn = 'MyApp.Domain.Container.Nested'..."
assert_cypher_exists "B16. type nested in type FQN" \
    "MATCH (e:Entity) WHERE e.fqn = 'MyApp.Domain.Container.Nested' AND e.repo_name = '$REPO_NAME' RETURN count(e)"

# ─────────────────────────────────────────────────────────────────────────────
# C · Relationships
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "C17. UserService -[:EXTENDS]-> BaseService..."
if assert_edge_exists "MyApp.Services.UserService" "MyApp.Services.BaseService" "EXTENDS"; then
    echo -e "${GREEN}✓ C17. UserService EXTENDS BaseService${NC}"
else
    echo -e "${RED}✗ C17. UserService EXTENDS BaseService missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C18. UserService -[:IMPLEMENTS]-> IUserService..."
if assert_edge_exists "MyApp.Services.UserService" "MyApp.Domain.IUserService" "IMPLEMENTS"; then
    echo -e "${GREEN}✓ C18. UserService IMPLEMENTS IUserService${NC}"
else
    echo -e "${RED}✗ C18. UserService IMPLEMENTS IUserService missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C19. UserRepository -[:IMPLEMENTS]-> IRepository (generic argument stripped)..."
if assert_edge_exists "MyApp.Infrastructure.UserRepository" "MyApp.Domain.IRepository" "IMPLEMENTS"; then
    echo -e "${GREEN}✓ C19. UserRepository IMPLEMENTS IRepository${NC}"
else
    echo -e "${RED}✗ C19. UserRepository IMPLEMENTS IRepository missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C20. IAdminRepository -[:EXTENDS]-> IRepository..."
if assert_edge_exists "MyApp.Domain.IAdminRepository" "MyApp.Domain.IRepository" "EXTENDS"; then
    echo -e "${GREEN}✓ C20. IAdminRepository EXTENDS IRepository${NC}"
else
    echo -e "${RED}✗ C20. IAdminRepository EXTENDS IRepository missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C21. Point (struct) never extends — zero outgoing EXTENDS edges..."
# The base entry `IEquatable<Point>` resolves to a .NET BCL type that is not
# an entity in the fixture graph, so no IMPLEMENTS edge can be materialised
# (edges only exist between indexed entities). The §3.3 heuristic's testable
# contract here is the negative half: a struct_declaration maps every base
# entry to IMPLEMENTS and none to EXTENDS. Positive IMPLEMENTS coverage for
# in-repo interfaces is asserted by C19/C20.
POINT_EXTENDS_COUNT=$(run_neo4j_cypher \
    "MATCH (a:Entity)-[:EXTENDS]->(b) WHERE a.fqn = 'MyApp.Domain.Point' AND a.repo_name = '$REPO_NAME' RETURN count(b)" | tr -d ' "')
POINT_EXTENDS_COUNT=${POINT_EXTENDS_COUNT:-0}
if [ "$POINT_EXTENDS_COUNT" = "0" ]; then
    echo -e "${GREEN}✓ C21. Point has zero EXTENDS edges (struct never extends)${NC}"
else
    echo -e "${RED}✗ C21. Point has $POINT_EXTENDS_COUNT EXTENDS edges — struct must never extend${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C22. UserService.GetUserAsync -[:CALLS]-> UserRepository.FindByIdAsync..."
if assert_edge_exists "MyApp.Services.UserService.GetUserAsync" "MyApp.Infrastructure.UserRepository.FindByIdAsync" "CALLS"; then
    echo -e "${GREEN}✓ C22. UserService.GetUserAsync CALLS UserRepository.FindByIdAsync${NC}"
else
    echo -e "${RED}✗ C22. UserService.GetUserAsync CALLS UserRepository.FindByIdAsync missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C23. new UserDto(...) redirects to constructor..."
# The CALLS target should be the UserDto constructor (resolved via
# redirect_class_call_to_constructor). Without the fixture project we expect
# the constructor to exist (A11 covers a different ctor; we assert here that
# at least one CALLS edge from GetUserAsync targets a UserDto constructor or
# record candidate).
USERDTO_CTOR_COUNT=$(run_neo4j_cypher \
    "MATCH (a:Entity)-[:CALLS]->(b:Entity) WHERE a.fqn = 'MyApp.Services.UserService.GetUserAsync' AND (b.kind = 'csharp_constructor' AND b.name = 'UserDto' OR b.kind = 'csharp_record' AND b.name = 'UserDto') AND a.repo_name = '$REPO_NAME' RETURN count(b)" | tr -d ' "')
USERDTO_CTOR_COUNT=${USERDTO_CTOR_COUNT:-0}
if [ "$USERDTO_CTOR_COUNT" -gt 0 ]; then
    echo -e "${GREEN}✓ C23. new UserDto(...) redirected to constructor ($USERDTO_CTOR_COUNT)${NC}"
else
    echo -e "${RED}✗ C23. new UserDto(...) did not resolve to a constructor${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C24. UserService.GetUserAsync -[:REFERENCES]-> UserDto..."
if assert_edge_exists "MyApp.Services.UserService.GetUserAsync" "MyApp.Domain.UserDto" "REFERENCES"; then
    echo -e "${GREEN}✓ C24. UserService.GetUserAsync REFERENCES UserDto${NC}"
else
    echo -e "${RED}✗ C24. UserService.GetUserAsync REFERENCES UserDto missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "C25. UserService -[:CONTAINS]-> UserService.GetUserAsync..."
if assert_edge_exists "MyApp.Services.UserService" "MyApp.Services.UserService.GetUserAsync" "CONTAINS"; then
    echo -e "${GREEN}✓ C25. UserService CONTAINS UserService.GetUserAsync${NC}"
else
    echo -e "${RED}✗ C25. UserService CONTAINS UserService.GetUserAsync missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

# ─────────────────────────────────────────────────────────────────────────────
# D · OVERRIDES
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "D26. UserRepository.FindByIdAsync -[:OVERRIDES]-> IRepository.FindByIdAsync..."
if assert_edge_exists "MyApp.Infrastructure.UserRepository.FindByIdAsync" "MyApp.Domain.IRepository.FindByIdAsync" "OVERRIDES"; then
    echo -e "${GREEN}✓ D26. UserRepository.FindByIdAsync OVERRIDES IRepository.FindByIdAsync${NC}"
else
    echo -e "${RED}✗ D26. UserRepository.FindByIdAsync OVERRIDES IRepository.FindByIdAsync missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "D27. UserService.Process -[:OVERRIDES]-> BaseService.Process..."
if assert_edge_exists "MyApp.Services.UserService.Process" "MyApp.Services.BaseService.Process" "OVERRIDES"; then
    echo -e "${GREEN}✓ D27. UserService.Process OVERRIDES BaseService.Process${NC}"
else
    echo -e "${RED}✗ D27. UserService.Process OVERRIDES BaseService.Process missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

# ─────────────────────────────────────────────────────────────────────────────
# E · Comments and attributes
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "E28. XML doc summary of IRepository in explore_file output..."
E28_FILE="Domain/IRepository.cs"
E28_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$E28_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"
E28_MCP=$(invoke_mcp "$E28_REQUEST")
E28_CLI=$(invoke_cli explore "$E28_FILE" -r "$REPO_NAME")
if echo "$E28_MCP" | grep -qiE "Generic persistence abstraction" && echo "$E28_CLI" | grep -qiE "Generic persistence abstraction"; then
    echo -e "${GREEN}✓ E28. XML doc summary present in IRepository explore_file (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ E28. XML doc summary of IRepository missing from explore_file${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "E29. [Obsolete] present in decorators of UserService..."
assert_cypher_exists "E29. Obsolete attribute captured" \
    "MATCH (e:Entity) WHERE e.fqn = 'MyApp.Services.UserService' AND e.repo_name = '$REPO_NAME' AND any(d IN e.decorators WHERE d CONTAINS 'Obsolete') RETURN count(e)"

# ─────────────────────────────────────────────────────────────────────────────
# F · Semantic search
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "F30. search_hybrid_context 'user repository data access' returns UserRepository..."
F30_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"user repository data access\",\"repo_name\":\"$REPO_NAME\"}}}"
F30_MCP=$(invoke_mcp "$F30_REQUEST")
F30_CLI=$(invoke_cli search "user repository data access" -r "$REPO_NAME")
if retry_match "UserRepository" invoke_cli search "user repository data access" -r "$REPO_NAME"; then
    echo -e "${GREEN}✓ F30a. CLI semantic search returns UserRepository${NC}"
else
    echo -e "${RED}✗ F30a. CLI semantic search did not return UserRepository${NC}"
    FAILURES=$((FAILURES + 1))
fi
if echo "$F30_MCP" | grep -q "UserRepository"; then
    echo -e "${GREEN}✓ F30b. MCP semantic search returns UserRepository${NC}"
else
    echo -e "${RED}✗ F30b. MCP semantic search did not return UserRepository${NC}"
    FAILURES=$((FAILURES + 1))
fi

# ─────────────────────────────────────────────────────────────────────────────
# G · find_callers
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "G31. find_callers FindByIdAsync lists UserService.GetUserAsync under Calls and UserRepository.FindByIdAsync under Overridden by..."
G31_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"FindByIdAsync\",\"repo_name\":\"$REPO_NAME\"}}}"
G31_MCP=$(invoke_mcp "$G31_REQUEST")
G31_CLI=$(invoke_cli callers "FindByIdAsync" -r "$REPO_NAME")
if echo "$G31_MCP" | grep -q "UserService" && echo "$G31_CLI" | grep -q "UserService"; then
    echo -e "${GREEN}✓ G31a. find_callers lists UserService.GetUserAsync under Calls (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ G31a. find_callers missing UserService.GetUserAsync under Calls${NC}"
    FAILURES=$((FAILURES + 1))
fi
if echo "$G31_MCP" | grep -qiE "Overridden by" && echo "$G31_MCP" | grep -q "UserRepository"; then
    echo -e "${GREEN}✓ G31b. find_callers lists UserRepository.FindByIdAsync under Overridden by (MCP)${NC}"
else
    echo -e "${RED}✗ G31b. find_callers missing UserRepository.FindByIdAsync under Overridden by (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

# ─────────────────────────────────────────────────────────────────────────────
# H · Qualified References & False Positives (F32 - F39)
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "F32. assert_edge_exists MyApp.Gestures.GestureConfig.GesturesEnabled MyApp.Gestures.GestureOwner.Off REFERENCES..."
if assert_edge_exists "MyApp.Gestures.GestureConfig.GesturesEnabled" "MyApp.Gestures.GestureOwner.Off" "REFERENCES"; then
    echo -e "${GREEN}✓ F32. GesturesEnabled REFERENCES GestureOwner.Off${NC}"
else
    echo -e "${RED}✗ F32. GesturesEnabled REFERENCES GestureOwner.Off missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "F33. assert_edge_exists MyApp.Gestures.GestureConfig.OwnerOf MyApp.Gestures.GestureOwner.Off REFERENCES..."
if assert_edge_exists "MyApp.Gestures.GestureConfig.OwnerOf" "MyApp.Gestures.GestureOwner.Off" "REFERENCES"; then
    echo -e "${GREEN}✓ F33. OwnerOf REFERENCES GestureOwner.Off${NC}"
else
    echo -e "${RED}✗ F33. OwnerOf REFERENCES GestureOwner.Off missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "F34. assert_edge_exists MyApp.Gestures.GestureConfig.Disable MyApp.Gestures.GestureOwner.OffValue REFERENCES..."
if assert_edge_exists "MyApp.Gestures.GestureConfig.Disable" "MyApp.Gestures.GestureOwner.OffValue" "REFERENCES"; then
    echo -e "${GREEN}✓ F34. Disable REFERENCES GestureOwner.OffValue${NC}"
else
    echo -e "${RED}✗ F34. Disable REFERENCES GestureOwner.OffValue missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "F35. assert_no_edge MyApp.ViewModels.LightingEffect MyApp.Gestures.GestureOwner.Off REFERENCES..."
if assert_no_edge "MyApp.ViewModels.LightingEffect" "MyApp.Gestures.GestureOwner.Off" "REFERENCES"; then
    echo -e "${GREEN}✓ F35. LightingEffect does not REFERENCE GestureOwner.Off${NC}"
else
    echo -e "${RED}✗ F35. LightingEffect spurious REFERENCE to GestureOwner.Off exists${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "F36. assert_cypher_count outgoing REFERENCES from LightingEffect == 0..."
assert_cypher_count "F36. LightingEffect has 0 outgoing REFERENCES" \
    "MATCH (a:Entity)-[:REFERENCES]->(b:Entity) WHERE a.fqn = 'MyApp.ViewModels.LightingEffect' AND a.repo_name = '$REPO_NAME' RETURN count(b) AS cnt" \
    "0"

echo ""
echo "F37. assert_edge_exists MyApp.Gestures.GestureOwner.Off MyApp.Gestures.GestureOwner EXTENDS..."
if assert_edge_exists "MyApp.Gestures.GestureOwner.Off" "MyApp.Gestures.GestureOwner" "EXTENDS"; then
    echo -e "${GREEN}✓ F37. Off EXTENDS GestureOwner${NC}"
else
    echo -e "${RED}✗ F37. Off EXTENDS GestureOwner missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "F38. assert_no_edge MyApp.Gestures.GestureOwner MyApp.Gestures.GestureOwner.Off REFERENCES..."
if assert_no_edge "MyApp.Gestures.GestureOwner" "MyApp.Gestures.GestureOwner.Off" "REFERENCES"; then
    echo -e "${GREEN}✓ F38. GestureOwner does not REFERENCE GestureOwner.Off${NC}"
else
    echo -e "${RED}✗ F38. GestureOwner spurious REFERENCE to GestureOwner.Off exists${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "F39. assert_edge_exists MyApp.Gestures.GestureConfig.Select MyApp.Gestures.GestureOwner.Button CALLS..."
if assert_edge_exists "MyApp.Gestures.GestureConfig.Select" "MyApp.Gestures.GestureOwner.Button" "CALLS"; then
    echo -e "${GREEN}✓ F39. Select CALLS GestureOwner.Button${NC}"
else
    echo -e "${RED}✗ F39. Select CALLS GestureOwner.Button missing${NC}"
    FAILURES=$((FAILURES + 1))
fi

# ─────────────────────────────────────────────────────────────────────────────
# H · find_callers Target Resolution (H40 - H48)
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "H40-H43. find_callers(\"Off\") should resolve to exactly GestureOwner.Off without noise (OfflineSlot, IsEligible) and disclose exact name match"
H40_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Off\",\"repo_name\":\"$REPO_NAME\"}}}"
H40_MCP=$(invoke_mcp "$H40_REQUEST")
H40_CLI=$(invoke_cli callers "Off" -r "$REPO_NAME")

if echo "$H40_MCP" | grep -q "GestureOwner.Off"; then
    echo -e "${GREEN}✓ H40. find_callers(\"Off\") contains GestureOwner.Off (MCP)${NC}"
else
    echo -e "${RED}✗ H40. find_callers(\"Off\") missing GestureOwner.Off (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

if ! echo "$H40_MCP" | grep -q "OfflineSlot"; then
    echo -e "${GREEN}✓ H41. find_callers(\"Off\") does not contain OfflineSlot (MCP)${NC}"
else
    echo -e "${RED}✗ H41. find_callers(\"Off\") contains OfflineSlot (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

if ! echo "$H40_MCP" | grep -q "IsEligible"; then
    echo -e "${GREEN}✓ H42. find_callers(\"Off\") does not contain IsEligible (MCP)${NC}"
else
    echo -e "${RED}✗ H42. find_callers(\"Off\") contains IsEligible (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

if echo "$H40_MCP" | grep -q "exact name match" && ! echo "$H40_MCP" | grep -qi "Fuzzy match"; then
    echo -e "${GREEN}✓ H43. find_callers(\"Off\") has correct tier disclosure (MCP)${NC}"
else
    echo -e "${RED}✗ H43. find_callers(\"Off\") incorrect/missing tier disclosure (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "H44. find_callers(\"GestureOwner.Off\") should resolve via FQN suffix"
H44_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"GestureOwner.Off\",\"repo_name\":\"$REPO_NAME\"}}}"
H44_MCP=$(invoke_mcp "$H44_REQUEST")
if echo "$H44_MCP" | grep -q "GestureOwner.Off" && ! echo "$H44_MCP" | grep -q "OfflineSlot"; then
    echo -e "${GREEN}✓ H44. find_callers(\"GestureOwner.Off\") resolved via FQN suffix (MCP)${NC}"
else
    echo -e "${RED}✗ H44. find_callers(\"GestureOwner.Off\") failed (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "H45. find_callers(\"MyApp.Gestures.GestureOwner.Off\") should resolve via exact FQN"
H45_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"MyApp.Gestures.GestureOwner.Off\",\"repo_name\":\"$REPO_NAME\"}}}"
H45_MCP=$(invoke_mcp "$H45_REQUEST")
if echo "$H45_MCP" | grep -q "GestureOwner.Off" && ! echo "$H45_MCP" | grep -q "OfflineSlot"; then
    echo -e "${GREEN}✓ H45. find_callers(\"MyApp.Gestures.GestureOwner.Off\") resolved via exact FQN (MCP)${NC}"
else
    echo -e "${RED}✗ H45. find_callers(\"MyApp.Gestures.GestureOwner.Off\") failed (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "H46. find_callers(\"IsEligible(DateTimeOffset\") should resolve via signature-prefix"
H46_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"IsEligible(DateTimeOffset\",\"repo_name\":\"$REPO_NAME\"}}}"
H46_MCP=$(invoke_mcp "$H46_REQUEST")
if echo "$H46_MCP" | grep -q "IsEligible" && ! echo "$H46_MCP" | grep -q "GestureOwner.Off"; then
    echo -e "${GREEN}✓ H46. find_callers(\"IsEligible(DateTimeOffset\") resolved via signature-prefix (MCP)${NC}"
else
    echo -e "${RED}✗ H46. find_callers(\"IsEligible(DateTimeOffset\") failed (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "H47. find_callers(\"Offlin\") should fall back to fuzzy match"
H47_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Offlin\",\"repo_name\":\"$REPO_NAME\"}}}"
H47_MCP=$(invoke_mcp "$H47_REQUEST")
if echo "$H47_MCP" | grep -qi "Fuzzy match" && echo "$H47_MCP" | grep -q "OfflineSlot"; then
    echo -e "${GREEN}✓ H47. find_callers(\"Offlin\") correct fuzzy match disclosure (MCP)${NC}"
else
    echo -e "${RED}✗ H47. find_callers(\"Offlin\") failed (MCP)${NC}"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "H48. CLI and MCP find_callers(\"Off\") should output parity"
# Compare the set of target entities listed (occurrence counts differ because
# the CLI renders a table and the MCP tool renders Markdown).
H48_CLI_TARGETS=$(echo "$H40_CLI" | grep -oE "[A-Za-z.]*GestureOwner\.Off" | sort -u)
H48_MCP_TARGETS=$(echo "$H40_MCP" | grep -oE "[A-Za-z.]*GestureOwner\.Off" | sort -u)
if [ "$H48_CLI_TARGETS" = "$H48_MCP_TARGETS" ] && [ -n "$H48_CLI_TARGETS" ]; then
    echo -e "${GREEN}✓ H48. CLI/MCP parity on targets for \"Off\"${NC}"
else
    echo -e "${RED}✗ H48. CLI/MCP parity failed for \"Off\"${NC}"
    FAILURES=$((FAILURES + 1))
fi

# ── Results ───────────────────────────────────────────────────────────────────
echo ""
if [ "$FAILURES" -eq 0 ]; then
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}All C# E2E tests passed! ✓${NC}"
    echo -e "${GREEN}========================================${NC}"
else
    echo -e "${RED}========================================${NC}"
    echo -e "${RED}$FAILURES C# E2E test(s) failed! ✗${NC}"
    echo -e "${RED}========================================${NC}"
    exit 1
fi
echo ""

exit 0