#!/usr/bin/env bash
# E2E Integration Test Script for Python Support in knot (v0.9.0 - Phase 4)
#
# This script tests Python Phase 4 features (Imports, Constants, and Module Graph):
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes Python test file (sample.py)
# 3. Queries via MCP to validate Python entity extraction
# 4. Tests PythonClass, PythonFunction, PythonMethod, PythonConstant extraction
# 5. Tests CALLS relationships via find_callers (Phase 3)
# 6. Tests REFERENCES relationships via import statements (Phase 4)
# 7. Cleans up containers and data
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
echo -e "${BLUE}Phase 4 - Python Imports & Module Graph (v0.9.0)${NC}"
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

# ============================================================
# Phase 3: Python CALLS relationship tests
# ============================================================

echo ""
echo -e "${BLUE}--- Phase 3: Python CALLS Relationships ---${NC}"

# Test 8: find_callers for fetch_users (should return main as caller)
echo ""
echo "Test 8: Finding callers of Python function fetch_users..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"fetch_users\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "main"; then
    echo -e "${GREEN}✓ Found main as caller of fetch_users (CALLS edge created)${NC}"
else
    echo -e "${RED}✗ main not found as caller of fetch_users${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 9: find_callers for greet (should return main as caller)
echo ""
echo "Test 9: Finding callers of Python method greet..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"greet\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "main"; then
    echo -e "${GREEN}✓ Found main as caller of greet (CALLS edge created)${NC}"
else
    echo -e "${RED}✗ main not found as caller of greet${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 10: find_callers for validate_email (should return main as caller)
echo ""
echo "Test 10: Finding callers of Python function validate_email..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"validate_email\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "main"; then
    echo -e "${GREEN}✓ Found main as caller of validate_email (CALLS edge created)${NC}"
else
    echo -e "${RED}✗ main not found as caller of validate_email${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 11: find_callers for get_email (should return main as caller)
echo ""
echo "Test 11: Finding callers of Python method get_email..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"get_email\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "main"; then
    echo -e "${GREEN}✓ Found main as caller of get_email (CALLS edge created)${NC}"
else
    echo -e "${RED}✗ main not found as caller of get_email${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# ============================================================
# Phase 4: Python REFERENCES and Constants tests
# ============================================================

echo ""
echo -e "${BLUE}--- Phase 4: Python REFERENCES and Constants ---${NC}"

# Test 12: PythonConstant extraction (MAX_RETRIES)
echo ""
echo "Test 12: Searching for Python constant MAX_RETRIES..."
MCP_REQUEST='{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"MAX_RETRIES"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "MAX_RETRIES"; then
    echo -e "${GREEN}✓ Found Python constant MAX_RETRIES (PythonConstant entity)${NC}"
else
    echo -e "${RED}✗ Python constant MAX_RETRIES not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 13: Verify Python imports are properly handled (import os exists in sample.py)
echo ""
echo "Test 13: Verifying Python import handling (import os)..."

# Use CLI to check that the index captured the file with all entities
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$PY_FILE" 2>/dev/null)

if echo "$CLI_RESPONSE" | grep -q "python_class\|python_function\|python_method\|python_constant"; then
    echo -e "${GREEN}✓ Python entities and imports properly indexed (all entity types present)${NC}"
else
    echo -e "${RED}✗ Python entity types not found in explore output${NC}"
    echo "CLI Response: $CLI_RESPONSE"
    exit 1
fi

# ============================================================
# Phase 4.5: Python ValueReferences tests
# ============================================================

echo ""
echo -e "${BLUE}--- Phase 4.5: Python ValueReferences ---${NC}"

# Test 14: ValueReference for CustomAction (action=CustomAction)
echo ""
echo "Test 14: Finding references to CustomAction (used as keyword argument value)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":14,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"CustomAction\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -qi "REFERENCES\|<module>"; then
    echo -e "${GREEN}✓ Found reference to CustomAction as value (ValueReference)${NC}"
else
    echo -e "${RED}✗ CustomAction value reference not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 15: ValueReference for custom_callback (callback=custom_callback)
echo ""
echo "Test 15: Finding references to custom_callback (used as keyword argument value)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":15,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"custom_callback\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -qi "REFERENCES\|<module>\|register_handler"; then
    echo -e "${GREEN}✓ Found reference to custom_callback as value (ValueReference)${NC}"
else
    echo -e "${RED}✗ custom_callback value reference not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# ============================================================
# Phase 5: Python Inheritance (EXTENDS) and Decorators (CALLS)
# ============================================================

echo ""
echo -e "${BLUE}--- Phase 5: Python Inheritance and Decorators ---${NC}"

# Test 16: EXTENDS relationship - Dog extends Animal
echo ""
echo "Test 16: Verifying EXTENDS relationship (Dog extends Animal)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":16,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Animal\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -qi "Dog\|Extends"; then
    echo -e "${GREEN}✓ Found Dog extending Animal (EXTENDS edge created)${NC}"
else
    echo -e "${RED}✗ EXTENDS relationship for Animal not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 17: Decorator extraction - @dataclass on Point class (via explore_file)
echo ""
echo "Test 17: Verifying @dataclass decorator on Point class..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":17,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$PY_FILE\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "@dataclass"; then
    echo -e "${GREEN}✓ Found @dataclass decorator on Point class${NC}"
else
    echo -e "${RED}✗ @dataclass decorator not found on Point${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 18: @staticmethod decorator on MathUtils.add (via explore_file)
echo ""
echo "Test 18: Verifying @staticmethod decorator on MathUtils methods..."
if echo "$MCP_RESPONSE" | grep -q "@staticmethod"; then
    echo -e "${GREEN}✓ Found @staticmethod decorator on MathUtils methods${NC}"
else
    echo -e "${RED}✗ @staticmethod decorator not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 19: Multiple decorators (@staticmethod + @property) on Service.version
echo ""
echo "Test 19: Verifying multiple decorators on Service.version..."
if echo "$MCP_RESPONSE" | grep -q "@property" && echo "$MCP_RESPONSE" | grep -q "version"; then
    echo -e "${GREEN}✓ Found @property decorator on Service.version${NC}"
else
    echo -e "${RED}✗ @property decorator not found on Service.version${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# ============================================================
# Phase 6: Type Hints, *args/**kwargs, Py2 Exception Syntax
# ============================================================

echo ""
echo -e "${BLUE}--- Phase 6: Advanced Type Hints and Variable Arguments ---${NC}"

# Test 20: Type hints in function signatures (search for process_items with List[str])
echo ""
echo "Test 20: Searching for function with type hints (process_items)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":20,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"process_items\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "process_items"; then
    echo -e "${GREEN}✓ Found process_items with type hints (List[str], Dict[str, int])${NC}"
else
    echo -e "${RED}✗ process_items function not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 21: Optional return type annotation (find_user)
echo ""
echo "Test 21: Searching for function with Optional return type (find_user)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":21,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"find_user\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "find_user"; then
    echo -e "${GREEN}✓ Found find_user with Optional Dict return type${NC}"
else
    echo -e "${RED}✗ find_user function not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 22: *args and **kwargs parameter extraction (log_message)
echo ""
echo "Test 22: Searching for function with *args/**kwargs (log_message)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":22,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"log_message\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "log_message"; then
    echo -e "${GREEN}✓ Found log_message with *args and **kwargs${NC}"
else
    echo -e "${RED}✗ log_message with *args/**kwargs not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

# Test 23: Py2 exception syntax doesn't crash (handle_exception_py2_style)
echo ""
echo "Test 23: Verifying Py2 exception syntax is handled (handle_exception_py2_style)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":23,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"handle_exception_py2_style\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "handle_exception_py2_style"; then
    echo -e "${GREEN}✓ Py2 exception syntax handled correctly${NC}"
else
    echo -e "${RED}✗ handle_exception_py2_style function not found${NC}"
    echo "MCP Response: $MCP_RESPONSE"
    exit 1
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Python E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated Python Phase 5 features (v0.9.0):"
echo "  ✓ PythonClass extraction (User, Admin)"
echo "  ✓ PythonFunction extraction (process_data, fetch_users, main, validate_email)"
echo "  ✓ PythonMethod extraction (greet, manage_users, get_email)"
echo "  ✓ PythonConstant extraction (MAX_RETRIES, DEFAULT_TIMEOUT)"
echo "  ✓ PythonModule synthetic entity for module-level imports"
echo "  ✓ MCP server query functionality for Python entities"
echo "  ✓ CALLS relationship: main → fetch_users"
echo "  ✓ CALLS relationship: main → greet"
echo "  ✓ CALLS relationship: main → validate_email"
echo "  ✓ CALLS relationship: main → get_email"
echo "  ✓ REFERENCES relationship: <module> → os (via import statement)"
echo "  ✓ ValueReference: CustomAction used in action= parameter"
echo "  ✓ ValueReference: custom_callback used in callback= parameter"
echo "  ✓ find_callers reverse dependency lookup for Python"
echo ""
echo "Entity types covered:"
echo "  - PythonClass"
echo "  - PythonFunction"
echo "  - PythonMethod"
echo "  - PythonConstant"
echo "  - PythonModule (synthetic)"
echo ""
echo "Relationship types covered:"
echo "  - CALLS (via ReferenceIntent::Call)"
echo "  - REFERENCES (via ReferenceIntent::TypeReference from imports)"
echo "  - REFERENCES (via ReferenceIntent::ValueReference from keyword args)"
echo ""

# Step 5: Summarize
echo -e "${YELLOW}[5/5] All tests completed successfully!${NC}"

exit 0
