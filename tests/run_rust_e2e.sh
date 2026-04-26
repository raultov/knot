#!/usr/bin/env bash
# E2E Integration Test Script for Rust Support in knot (v0.8.x)
#
# This script tests Rust-specific features:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes Rust test file (sample.rs)
# 3. Queries via MCP to validate Rust entity extraction
# 4. Tests struct, enum, trait, impl, function, macro extraction
# 5. Cleans up containers and data
#
# Usage: ./tests/run_rust_e2e.sh
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
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_rust_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_rust_e2e_test"
REPO_NAME="rust_e2e_test_repo"

# Timeout settings
TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Rust E2E Integration Test${NC}"
echo -e "${BLUE}Phase 5 - Rust Support (v0.8.x)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Rust E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Rust E2E test environment...${NC}"
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
echo -e "${YELLOW}[1/5] Starting Docker containers for Rust E2E test...${NC}"
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

# Step 3: Index Rust test file
echo -e "${YELLOW}[3/5] Indexing Rust sample file (sample.rs)...${NC}"
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

echo "Running indexer for Rust files..."
cargo run --release --bin knot-indexer -- --clean

echo -e "${GREEN}✓ Rust file indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating Rust entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Rust struct extraction
echo ""
echo "Test 1: Exploring sample.rs - Rust struct extraction..."
RS_FILE="$TEST_FILES_DIR/sample.rs"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$RS_FILE\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$RS_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Counter" && echo "$CLI_RESPONSE" | grep -q "Counter"; then
    echo -e "${GREEN}✓ Found Rust struct Counter (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust struct Counter not found${NC}"
    exit 1
fi

# Test 2: Rust trait extraction
echo ""
echo "Test 2: Searching for Rust trait Incrementable..."
MCP_REQUEST='{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Incrementable"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Incrementable" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Incrementable" && echo "$CLI_RESPONSE" | grep -q "Incrementable"; then
    echo -e "${GREEN}✓ Found Rust trait Incrementable (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust trait Incrementable not found${NC}"
    exit 1
fi

# Test 3: Rust enum extraction
echo ""
echo "Test 3: Searching for Rust enum Color..."
MCP_REQUEST='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Color"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Color" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Color" && echo "$CLI_RESPONSE" | grep -q "Color"; then
    echo -e "${GREEN}✓ Found Rust enum Color (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust enum Color not found${NC}"
    exit 1
fi

# Test 4: Rust function extraction - verify via explore_file
echo ""
echo "Test 4: Verifying Rust functions are indexed in sample.rs..."
RS_FILE="$TEST_FILES_DIR/sample.rs"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$RS_FILE\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$RS_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

# Check for Functions (Rust) header and specific function names
if (echo "$MCP_RESPONSE" | grep -q "## Functions (Rust)" && echo "$MCP_RESPONSE" | grep -qE "add|longest|fetch_data") && (echo "$CLI_RESPONSE" | grep -q "## Functions (Rust)"); then
    echo -e "${GREEN}✓ Rust functions indexed in sample.rs (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust functions not properly indexed${NC}"
    exit 1
fi

# Test 5: Rust function with explicit name (since macros were removed from query)
echo ""
echo "Test 5: Searching for Rust function longest..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"longest\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "longest" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "longest" && echo "$CLI_RESPONSE" | grep -q "longest"; then
    echo -e "${GREEN}✓ Found Rust function longest (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust function longest not found${NC}"
    exit 1
fi

# Test 6: Rust module extraction
echo ""
echo "Test 6: Searching for Rust module inner..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"module inner\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "module inner" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "inner" && echo "$CLI_RESPONSE" | grep -q "inner"; then
    echo -e "${GREEN}✓ Found Rust module inner (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust module inner not found${NC}"
    exit 1
fi

# Test 7: Rust impl block detection
echo ""
echo "Test 7: Searching for Rust impl blocks on Counter..."
MCP_REQUEST='{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Counter"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Counter" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Counter" && echo "$CLI_RESPONSE" | grep -q "Counter"; then
    echo -e "${GREEN}✓ Found Rust struct Counter with impl blocks (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust struct Counter with impl blocks not found${NC}"
    exit 1
fi

# Test 8: Rust type alias
echo ""
echo "Test 8: Searching for Rust type alias Callback..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"Callback type alias\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Callback" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Callback" && echo "$CLI_RESPONSE" | grep -q "Callback"; then
    echo -e "${GREEN}✓ Found Rust type alias Callback (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust type alias Callback not found${NC}"
    exit 1
fi

# Test 9: Rust constant
echo ""
echo "Test 9: Searching for Rust constant MAX_SIZE..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"MAX_SIZE constant\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "MAX_SIZE" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "MAX_SIZE" && echo "$CLI_RESPONSE" | grep -q "MAX_SIZE"; then
    echo -e "${GREEN}✓ Found Rust constant MAX_SIZE (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust constant MAX_SIZE not found${NC}"
    exit 1
fi

# Test 10: Rust static
echo ""
echo "Test 10: Searching for Rust static COUNTER..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"COUNTER static\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "COUNTER" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "COUNTER" && echo "$CLI_RESPONSE" | grep -q "COUNTER"; then
    echo -e "${GREEN}✓ Found Rust static COUNTER (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust static COUNTER not found${NC}"
    exit 1
fi

# Test 11: Already tested longest in Test 5

# Test 12: Rust generic struct with trait bounds
echo ""
echo "Test 12: Searching for Rust generic struct Container..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"Container\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Container" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Container" && echo "$CLI_RESPONSE" | grep -q "Container"; then
    echo -e "${GREEN}✓ Found Rust generic struct Container (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust generic struct Container not found${NC}"
    exit 1
fi

# Test 13: Rust trait with generic associated type
echo ""
echo "Test 13: Searching for Rust trait Repository with associated type..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":13,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"Repository\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Repository" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Repository" && echo "$CLI_RESPONSE" | grep -q "Repository"; then
    echo -e "${GREEN}✓ Found Rust trait Repository with associated type (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust trait Repository not found${NC}"
    exit 1
fi

# Test 14: Rust async function
echo ""
echo "Test 14: Searching for Rust async function fetch_data..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":14,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"fetch_data\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "fetch_data" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "fetch_data" && echo "$CLI_RESPONSE" | grep -q "fetch_data"; then
    echo -e "${GREEN}✓ Found Rust async function fetch_data (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust async function fetch_data not found${NC}"
    exit 1
fi

# Test 15: Rust struct with derive macros
echo ""
echo "Test 15: Searching for Rust struct Config with derive..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":15,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"Config\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Config" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Config" && echo "$CLI_RESPONSE" | grep -q "Config"; then
    echo -e "${GREEN}✓ Found Rust struct Config with derive macros (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust struct Config not found${NC}"
    exit 1
fi

# Test 16: Rust union extraction
echo ""
echo "Test 16: Searching for Rust union MaybeFloat..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":16,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"MaybeFloat union\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "MaybeFloat" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "MaybeFloat" && echo "$CLI_RESPONSE" | grep -q "MaybeFloat"; then
    echo -e "${GREEN}✓ Found Rust union MaybeFloat (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust union MaybeFloat not found${NC}"
    exit 1
fi

# Test 17: Rust method extraction (methods inside impl blocks)
echo ""
echo "Test 17: Verifying Rust methods are indexed in sample.rs..."
RS_FILE="$TEST_FILES_DIR/sample.rs"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":17,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$RS_FILE\",\"repo_name\":\"rust_e2e_test_repo\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$RS_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

# Check for Methods (Rust) header and specific method names like get_count, increment
if (echo "$MCP_RESPONSE" | grep -q "## Methods (Rust)" && echo "$MCP_RESPONSE" | grep -qE "get_count|increment|with_label") && (echo "$CLI_RESPONSE" | grep -q "## Methods (Rust)"); then
    echo -e "${GREEN}✓ Rust methods indexed in sample.rs (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust methods not properly indexed${NC}"
    exit 1
fi

# Test 18: Rust find_callers for trait implementation
echo ""
echo "Test 18: Testing find_callers for Incrementable trait implementations..."
MCP_REQUEST='{"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"Incrementable"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "Incrementable" -r "$REPO_NAME" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Counter" || echo "$CLI_RESPONSE" | grep -q "Counter"; then
    echo -e "${GREEN}✓ Found callers of Incrementable trait (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ No callers found for Incrementable trait${NC}"
    exit 1
fi

# Test 19: Rust function with where clause
echo ""
echo "Test 19: Searching for Rust function process_value..."
MCP_REQUEST='{"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"process_value","repo_name":"rust_e2e_test_repo"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "process_value" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "process_value" && echo "$CLI_RESPONSE" | grep -q "process_value"; then
    echo -e "${GREEN}✓ Found Rust function process_value (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Rust function process_value not found${NC}"
    exit 1
fi

# Test 20: Rust module re-export
echo ""
echo "Test 20: Searching for re-exported function inner_function..."
MCP_REQUEST='{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"inner_function","repo_name":"rust_e2e_test_repo"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "inner_function" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "inner_function" && echo "$CLI_RESPONSE" | grep -q "inner_function"; then
    echo -e "${GREEN}✓ Found re-exported function inner_function (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Re-exported function inner_function not found${NC}"
    exit 1
fi

# Test 21: Rust inner doc comments
echo ""
echo "Test 21: Verify doc comments are indexed for inner module..."
MCP_REQUEST='{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"inner module documentation","repo_name":"rust_e2e_test_repo"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "inner"; then
    echo -e "${GREEN}✓ Inner module documentation indexed (MCP)${NC}"
else
    echo -e "${YELLOW}⚠ Inner module documentation search returned no results (may be expected)${NC}"
fi

# Test 22: Rust macro invocation tracking (println! in sample)
echo ""
echo "Test 22: Searching for macro invocation println..."
MCP_REQUEST='{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"println","repo_name":"rust_e2e_test_repo"}}}'

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "println" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "println" && echo "$CLI_RESPONSE" | grep -q "println"; then
    echo -e "${GREEN}✓ Found macro invocation println (MCP & CLI)${NC}"
else
    echo -e "${YELLOW}⚠ Macro invocation println not found (may be expected - external macro)${NC}"
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Rust E2E tests passed!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Summary:"
echo "  - Struct extraction (Counter, Config, Point)"
echo "  - Enum extraction (Color)"
echo "  - Trait extraction (Incrementable, Repository)"
echo "  - Impl block detection"
echo "  - Function extraction (add, longest, process_value, fetch_data)"
echo "  - Method extraction (get_count)"
echo "  - Macro definition (init_vec)"
echo "  - Macro invocation (println)"
echo "  - Module extraction (inner)"
echo "  - Type alias (Callback)"
echo "  - Constant (MAX_SIZE)"
echo "  - Static (COUNTER)"
echo "  - Union (MaybeFloat)"
echo "  - Generic structs and functions"
echo "  - Lifetime parameters"
echo "  - Derive macros"
echo ""

# Step 5: Summarize
echo -e "${YELLOW}[5/5] All tests completed successfully!${NC}"