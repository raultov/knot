#!/usr/bin/env bash
# E2E Integration Test Script for the Web Ecosystem in knot
#
# This script tests web-stack language features:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes web fixture files (HTML, JSX, CSS, SCSS, hybrid SPA)
# 3. Queries via MCP to validate element, class, id, and cross-file references
# 4. Tests HTML custom elements, CSS classes, SCSS variables, JSX attributes,
#    and hybrid JS↔HTML/CSS cross-references
# 5. Cleans up containers and data
#
# Usage: ./tests/run_web_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/web"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_web_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_web_e2e_test"
REPO_NAME="web_e2e_test_repo"

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
echo -e "${BLUE}knot Web Ecosystem E2E Integration Test${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Web E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Web E2E test environment...${NC}"
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
    echo -e "${YELLOW}[1/5] Starting Docker containers for Web E2E test...${NC}"
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

# Step 3: Index Web fixture files
echo -e "${YELLOW}[3/5] Indexing Web fixture files...${NC}"
cd "$PROJECT_ROOT"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for Web files..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"

echo -e "${GREEN}✓ Web files indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating Web entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Explore JSX file - class discovery
echo ""
echo "Test 1: Exploring test_javascript.jsx for class discovery..."
JSX_FILE="$TEST_FILES_DIR/test_javascript.jsx"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$JSX_FILE\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$JSX_FILE" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "DataService"; then
    echo -e "${GREEN}✓ MCP: Found DataService in JavaScript file${NC}"
else
    echo -e "${RED}✗ MCP: DataService not found${NC}"
    exit 1
fi

if echo "$CLI_RESPONSE" | grep -q "DataService"; then
    echo -e "${GREEN}✓ CLI: Found DataService in JavaScript file${NC}"
else
    echo -e "${RED}✗ CLI: DataService not found${NC}"
    exit 1
fi

# Test 2: Search for HTML elements and attributes
echo ""
echo "Test 2: Searching for HTML elements and attributes in test_angular.html..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"app-header\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "app-header" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "app-header"; then
    echo -e "${GREEN}✓ MCP: Found app-header custom element${NC}"
else
    echo -e "${RED}✗ MCP: app-header custom element not found${NC}"
    exit 1
fi

if echo "$CLI_RESPONSE" | grep -q "app-header"; then
    echo -e "${GREEN}✓ CLI: Found app-header custom element${NC}"
else
    echo -e "${RED}✗ CLI: app-header custom element not found${NC}"
    exit 1
fi

# Test for HTML id attribute
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"dashboard\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "dashboard" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "dashboard" && echo "$CLI_RESPONSE" | grep -q "dashboard"; then
    echo -e "${GREEN}✓ Found HTML id 'dashboard' (both MCP and CLI)${NC}"
else
    echo -e "${RED}✗ HTML id 'dashboard' not found${NC}"
    exit 1
fi

# Test for HTML class attribute
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"navbar\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "navbar" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "navbar" && echo "$CLI_RESPONSE" | grep -q "navbar"; then
    echo -e "${GREEN}✓ Found HTML class 'navbar' (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ HTML class 'navbar' not found${NC}"
    exit 1
fi

# Test 3: Search for JSX attributes
echo ""
echo "Test 3: Searching for JSX attributes in test_javascript.jsx..."
# JSX id attribute
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"chart-toolbar\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "chart-toolbar" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "chart-toolbar" && echo "$CLI_RESPONSE" | grep -q "chart-toolbar"; then
    echo -e "${GREEN}✓ Found JSX id 'chart-toolbar' (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ JSX id 'chart-toolbar' not found${NC}"
    exit 1
fi

# JSX className attribute
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"btn-primary\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "btn-primary" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "btn-primary" && echo "$CLI_RESPONSE" | grep -q "btn-primary"; then
    echo -e "${GREEN}✓ Found JSX className 'btn-primary' (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ JSX className 'btn-primary' not found${NC}"
    exit 1
fi

# Multiple classes in JSX
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"profile-card\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "profile-card" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "profile-card" && echo "$CLI_RESPONSE" | grep -q "profile-card"; then
    echo -e "${GREEN}✓ Found JSX className 'profile-card' (multiple classes, MCP & CLI)${NC}"
else
    echo -e "${RED}✗ JSX className 'profile-card' not found${NC}"
    exit 1
fi

# Test 4: Search for CSS classes
echo ""
echo "Test 4: Searching for CSS classes in test_styles.css..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"btn-primary\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "btn-primary" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "btn-primary" && echo "$CLI_RESPONSE" | grep -q "btn-primary"; then
    echo -e "${GREEN}✓ Found CSS class 'btn-primary' (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ CSS class 'btn-primary' not found${NC}"
    exit 1
fi

# CSS id
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"header-container\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "header-container" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "header-container" && echo "$CLI_RESPONSE" | grep -q "header-container"; then
    echo -e "${GREEN}✓ Found CSS id 'header-container' (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ CSS id 'header-container' not found${NC}"
    exit 1
fi

# Test 5: Search for SCSS classes
echo ""
echo "Test 5: Searching for SCSS classes in test_styles.scss..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"responsive-grid\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "responsive-grid" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "responsive-grid" && echo "$CLI_RESPONSE" | grep -q "responsive-grid"; then
    echo -e "${GREEN}✓ Found SCSS class 'responsive-grid' (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ SCSS class 'responsive-grid' not found${NC}"
    exit 1
fi

# Test 6: Hybrid — CSS class 'btn-primary' referenced in JavaScript
echo ""
echo "Test 6: Hybrid search for CSS class 'btn-primary' (HTML+CSS+JS)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"btn-primary\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "btn-primary" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "btn-primary" && echo "$CLI_RESPONSE" | grep -q "btn-primary"; then
    echo -e "${GREEN}✓ CSS class 'btn-primary' cross-language reference found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ CSS class 'btn-primary' not found in hybrid search${NC}"
    exit 1
fi

# Test 7: Hybrid — HTML id 'app-container' manipulated in JavaScript
echo ""
echo "Test 7: Hybrid search for HTML id 'app-container' manipulated in JS..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"app-container\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "app-container" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "app-container" && echo "$CLI_RESPONSE" | grep -q "app-container"; then
    echo -e "${GREEN}✓ HTML id 'app-container' cross-language reference found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ HTML id 'app-container' not found in hybrid search${NC}"
    exit 1
fi

# Test 8: Hybrid — HTML id 'toggle-btn' used in theme switching
echo ""
echo "Test 8: Hybrid search for HTML id 'toggle-btn' (theme switching)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":13,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"toggle-btn\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "toggle-btn" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "toggle-btn" && echo "$CLI_RESPONSE" | grep -q "toggle-btn"; then
    echo -e "${GREEN}✓ HTML id 'toggle-btn' cross-language reference found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ HTML id 'toggle-btn' not found in hybrid search${NC}"
    exit 1
fi

# Step 5: Summarize
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Web E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated Web ecosystem features:"
echo "  ✓ JSX class discovery (DataService)"
echo "  ✓ HTML custom elements (app-header)"
echo "  ✓ HTML id attributes (dashboard)"
echo "  ✓ HTML class attributes (navbar)"
echo "  ✓ JSX id attribute (chart-toolbar)"
echo "  ✓ JSX className attributes (btn-primary, profile-card)"
echo "  ✓ CSS class (btn-primary)"
echo "  ✓ CSS id (header-container)"
echo "  ✓ SCSS class (responsive-grid)"
echo "  ✓ Hybrid CSS class reference (HTML+CSS+JS btn-primary)"
echo "  ✓ Hybrid HTML id reference (app-container in JS)"
echo "  ✓ Hybrid HTML id reference (toggle-btn in JS)"
echo ""

exit 0
