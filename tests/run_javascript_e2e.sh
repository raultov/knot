#!/usr/bin/env bash
# E2E Integration Test Script for JavaScript Support in knot
#
# This script tests JavaScript-specific features:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes JavaScript test files (alias + import fixtures)
# 3. Queries via MCP to validate JavaScript entity extraction
# 4. Tests class, import/require, alias resolution, circular dependency handling
# 5. Cleans up containers and data
#
# Usage: ./tests/run_javascript_e2e.sh
# Requirements: docker, docker-compose

set -e  # Exit on error
set -u  # Exit on undefined variable

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.e2e.yml"
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/javascript"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_javascript_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_javascript_e2e_test"
REPO_NAME="javascript_e2e_test_repo"

# Timeout settings
TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

# Export common env for child cargo invocations
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"
export KNOT_REPO_PATH="$TEST_FILES_DIR"
export KNOT_REPO_NAME="$REPO_NAME"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot JavaScript E2E Integration Test${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}JavaScript E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up JavaScript E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

# Register cleanup on script exit
trap cleanup EXIT INT TERM

# Step 1: Start Docker containers (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/5] Starting Docker containers for JavaScript E2E test...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/5] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set; expecting shared DB)${NC}"
fi

# Step 2: Wait for services to be ready (skipped if KNOT_E2E_EXTERNAL_DB is set)
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

# Step 3: Index JavaScript test files
echo -e "${YELLOW}[3/5] Indexing JavaScript test files...${NC}"
cd "$PROJECT_ROOT"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for JavaScript files..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"

echo -e "${GREEN}✓ JavaScript files indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating JavaScript entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: JavaScript cross-file alias resolution (require)
echo ""
echo "Test 1: Verifying JavaScript cross-file alias resolution (MyJsAlias → MyJsTarget)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"MyJsTarget\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "MyJsTarget" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "callerJs" || echo "$CLI_RESPONSE" | grep -q "callerJs"; then
    echo -e "${GREEN}✓ Found cross-file alias resolution callerJs → MyJsTarget${NC}"
else
    echo -e "${RED}✗ Missing alias resolution callerJs → MyJsTarget${NC}"
    exit 1
fi

# Test 2: JS circular require alias resolution — completes without hanging
echo ""
echo "Test 2: Verifying JS circular require alias resolution (CycleB ⇄ CycleA)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"CycleA_target\",\"repo_name\":\"$REPO_NAME\",\"max_results\":5}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "CycleA_target" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "CycleA_target" || echo "$CLI_RESPONSE" | grep -q "CycleA_target"; then
    echo -e "${GREEN}✓ Circular require aliases resolved without deadlock; CycleA_target found${NC}"
else
    echo -e "${RED}✗ Missing CycleA_target after circular alias resolution${NC}"
    exit 1
fi

# Test 3: JS circular require — cross-file caller relationship preserved
echo ""
echo "Test 3: Verifying JS circular require preserves relationships (callerInB → CycleA_target)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"CycleA_target\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "CycleA_target" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "callerInB" || echo "$CLI_RESPONSE" | grep -q "callerInB"; then
    echo -e "${GREEN}✓ callerInB → CycleA_target relationship preserved across circular alias${NC}"
else
    echo -e "${RED}✗ Missing callerInB → CycleA_target relationship${NC}"
    exit 1
fi

# Test 4: JavaScript import capture — find_callers for JsImportFoo
echo ""
echo "Test 4: Verifying JavaScript import capture — find_callers for JsImportFoo..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"JsImportFoo\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "JsImportFoo" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "js_imports_uses.js" || echo "$CLI_RESPONSE" | grep -q "js_imports_uses.js"; then
    echo -e "${GREEN}✓ Found js_imports_uses.js as caller of JsImportFoo via import${NC}"
else
    echo -e "${RED}✗ js_imports_uses.js not found as caller of JsImportFoo${NC}"
    exit 1
fi

# Test 5: JavaScript require destructuring — find_callers for JsImportQux
echo ""
echo "Test 5: Verifying JavaScript require destructuring — find_callers for JsImportQux..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"JsImportQux\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "JsImportQux" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "js_imports_uses.js" || echo "$CLI_RESPONSE" | grep -q "js_imports_uses.js"; then
    echo -e "${GREEN}✓ Found js_imports_uses.js as caller of JsImportQux via require destructuring${NC}"
else
    echo -e "${RED}✗ js_imports_uses.js not found as caller of JsImportQux${NC}"
    exit 1
fi

# Test 6: JavaScript explore_file on js_imports_uses.js — verify Imports section
echo ""
echo "Test 6: Verifying explore_file on js_imports_uses.js shows Imports / Referenced Types..."
JS_IMPORTS_FILE="$TEST_FILES_DIR/js_imports_uses.js"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$JS_IMPORTS_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$JS_IMPORTS_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "## Imports / Referenced Types" && echo "$CLI_RESPONSE" | grep -q "## Imports / Referenced Types"; then
    echo -e "${GREEN}✓ explore_file shows Imports / Referenced Types section (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ explore_file missing Imports / Referenced Types section${NC}"
    exit 1
fi

if (echo "$MCP_RESPONSE" | grep -q "JsImportFoo") && (echo "$MCP_RESPONSE" | grep -q "JsImportQux"); then
    echo -e "${GREEN}✓ Imports section lists JsImportFoo and JsImportQux (MCP)${NC}"
else
    echo -e "${RED}✗ Imports section missing JsImportFoo or JsImportQux${NC}"
    exit 1
fi

# Step 5: Summarize
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All JavaScript E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated JavaScript features:"
echo "  ✓ JS cross-file alias resolution (callerJs → MyJsTarget)"
echo "  ✓ JS circular require alias resolution (CycleA ⇄ CycleB)"
echo "  ✓ JS cross-file caller relationship preservation (callerInB → CycleA_target)"
echo "  ✓ JS import capture (JsImportFoo via import)"
echo "  ✓ JS require destructuring (JsImportQux via require)"
echo "  ✓ JS explore_file shows Imports / Referenced Types section"
echo ""

exit 0
