#!/usr/bin/env bash
# E2E Integration Test Script for Python Support in knot (v0.8.12 - Phase 8 Phase 1)
#
# This script tests Python Phase 1 features (base configuration only):
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes Python test file (sample.py)
# 3. Queries via MCP to validate Python entity extraction
# 4. Tests PythonClass, PythonFunction, PythonMethod extraction
# 5. Cleans up containers and data
#
# NOTE: Full extraction logic is Phase 2. This test validates the pipeline wiring.
#
# Usage: ./tests/run_python_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_python_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_python_e2e_test"
REPO_NAME="python_e2e_test_repo"

# Timeout settings
TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Python E2E Integration Test${NC}"
echo -e "${BLUE}Phase 8 Phase 1 - Python Base Config (v0.8.12)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Python E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Python E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

# Register cleanup on script exit
trap cleanup EXIT INT TERM

# Step 1: Start Docker containers
echo -e "${YELLOW}[1/5] Starting Docker containers for Python E2E test...${NC}"
cd "$SCRIPT_DIR"
docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
if [ -d "$E2E_DATA_DIR" ]; then
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
fi
docker compose -f "$COMPOSE_FILE" up -d

# Step 2: Wait for services to be ready
echo -e "${YELLOW}[2/5] Waiting for services to be ready...${NC}"

wait_for_port() {
    local port=$1
    local service=$2
    local elapsed=0

    while ! nc -z localhost "$port" 2>/dev/null; do
        if [ $elapsed -ge $TIMEOUT_SECONDS ]; then
            echo -e "${RED}ERROR: $service did not start within ${TIMEOUT_SECONDS}s${NC}"
            return 1
        fi
        sleep $HEALTH_CHECK_INTERVAL
        elapsed=$((elapsed + HEALTH_CHECK_INTERVAL))
        echo -n "."
    done
    echo ""
    echo -e "${GREEN}✓ $service is ready${NC}"
}

wait_for_port 17687 "Neo4j"
wait_for_port 16334 "Qdrant"
sleep 5

# Step 3: Index Python test file
echo -e "${YELLOW}[3/5] Indexing Python sample file (sample.py)...${NC}"
cd "$PROJECT_ROOT"

export KNOT_REPO_PATH="$TEST_FILES_DIR"
export KNOT_REPO_NAME="$REPO_NAME"
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for Python files..."
cargo run --release --bin knot-indexer -- --clean

echo -e "${GREEN}✓ Python file indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating Python entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

PY_FILE="$TEST_FILES_DIR/sample.py"

# Test 1: PythonClass extraction (User)
echo ""
echo "Test 1: Exploring sample.py - Python class extraction (User)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$PY_FILE\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$PY_FILE" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "User" && echo "$CLI_RESPONSE" | grep -q "User"; then
    echo -e "${GREEN}✓ Found Python class User (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Python class User not found${NC}"
    exit 1
fi

# Test 2: PythonClass extraction (Admin)
echo ""
echo "Test 2: Searching for Python class Admin..."
MCP_REQUEST='{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Admin"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Admin" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Admin" && echo "$CLI_RESPONSE" | grep -q "Admin"; then
    echo -e "${GREEN}✓ Found Python class Admin (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Python class Admin not found${NC}"
    exit 1
fi

# Test 3: PythonFunction extraction (process_data)
echo ""
echo "Test 3: Searching for Python function process_data..."
MCP_REQUEST='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"process_data"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "process_data" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "process_data" && echo "$CLI_RESPONSE" | grep -q "process_data"; then
    echo -e "${GREEN}✓ Found Python function process_data (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Python function process_data not found${NC}"
    exit 1
fi

# Test 4: PythonFunction extraction (fetch_users)
echo ""
echo "Test 4: Searching for Python function fetch_users..."
MCP_REQUEST='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"fetch_users"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "fetch_users" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "fetch_users" && echo "$CLI_RESPONSE" | grep -q "fetch_users"; then
    echo -e "${GREEN}✓ Found Python function fetch_users (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Python function fetch_users not found${NC}"
    exit 1
fi

# Test 5: PythonFunction extraction (main)
echo ""
echo "Test 5: Searching for Python function main..."
MCP_REQUEST='{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"main"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "main" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "main" && echo "$CLI_RESPONSE" | grep -q "main"; then
    echo -e "${GREEN}✓ Found Python function main (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Python function main not found${NC}"
    exit 1
fi

# Test 6: PythonMethod extraction (greet)
echo ""
echo "Test 6: Searching for Python method greet..."
MCP_REQUEST='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"greet"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "greet" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "greet" && echo "$CLI_RESPONSE" | grep -q "greet"; then
    echo -e "${GREEN}✓ Found Python method greet (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Python method greet not found${NC}"
    exit 1
fi

# Test 7: PythonMethod extraction (manage_users)
echo ""
echo "Test 7: Searching for Python method manage_users..."
MCP_REQUEST='{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"manage_users"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "manage_users" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "manage_users" && echo "$CLI_RESPONSE" | grep -q "manage_users"; then
    echo -e "${GREEN}✓ Found Python method manage_users (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Python method manage_users not found${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Python E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated Python Phase 1 features (v0.8.12):"
echo "  ✓ PythonClass extraction (User, Admin)"
echo "  ✓ PythonFunction extraction (process_data, fetch_users, main)"
echo "  ✓ PythonMethod extraction (greet, manage_users)"
echo "  ✓ MCP server query functionality for Python entities"
echo ""
echo "Entity types covered:"
echo "  - PythonClass"
echo "  - PythonFunction"
echo "  - PythonMethod"
echo ""

# Step 5: Summarize
echo -e "${YELLOW}[5/5] All tests completed successfully!${NC}"

exit 0
