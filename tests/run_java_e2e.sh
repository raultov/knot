#!/usr/bin/env bash
# E2E Integration Test Script for Java Support in knot
#
# This script tests Java-specific features:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes Java test file (test_java.java)
# 3. Queries via MCP to validate Java entity extraction
# 4. Tests class, interface, annotation, FQN resolution, and inheritance
# 5. Cleans up containers and data
#
# Usage: ./tests/run_java_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/java"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_java_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_java_e2e_test"
REPO_NAME="java_e2e_test_repo"

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
echo -e "${BLUE}knot Java E2E Integration Test${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Java E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Java E2E test environment...${NC}"
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
    echo -e "${YELLOW}[1/5] Starting Docker containers for Java E2E test...${NC}"
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

# Step 3: Index Java test file
echo -e "${YELLOW}[3/5] Indexing Java test file (test_java.java)...${NC}"
cd "$PROJECT_ROOT"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for Java files..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"

echo -e "${GREEN}✓ Java file indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating Java entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Search for UserService in Java
echo ""
echo "Test 1: Searching for UserService in Java files..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"UserService\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "UserService" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "UserService"; then
    echo -e "${GREEN}✓ MCP: Found UserService in search results${NC}"
else
    echo -e "${RED}✗ MCP: UserService not found${NC}"
    exit 1
fi

if echo "$CLI_RESPONSE" | grep -q "UserService"; then
    echo -e "${GREEN}✓ CLI: Found UserService in search results${NC}"
else
    echo -e "${RED}✗ CLI: UserService not found${NC}"
    exit 1
fi

# Test 2: Java package FQN resolution
echo ""
echo "Test 2: Verifying Java FQN includes package name..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"UserService\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
if echo "$MCP_RESPONSE" | grep -q "com.example.knot.test.UserService"; then
    echo -e "${GREEN}✓ FQN includes package prefix com.example.knot.test${NC}"
else
    echo -e "${RED}✗ FQN missing package prefix${NC}"
    exit 1
fi

# Test 3: Java field_access and FQN resolution for method calls
echo ""
echo "Test 3: Searching for callers of ChatMemory.add..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"ChatMemory.add\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "ChatMemory.add" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "before" || echo "$CLI_RESPONSE" | grep -q "before"; then
    echo -e "${GREEN}✓ Found ChatMemoryAdvisor.before calling ChatMemory.add${NC}"
else
    echo -e "${RED}✗ ChatMemoryAdvisor.before calling ChatMemory.add not found${NC}"
    exit 1
fi

# Test 4: Java method signature search with parameter types
echo ""
echo "Test 4: Searching for Java method by full signature registerUser(String..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"registerUser(String\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "registerUser(String" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "createUser" || echo "$CLI_RESPONSE" | grep -q "createUser"; then
    echo -e "${GREEN}✓ Found callers of registerUser by signature${NC}"
else
    echo -e "${RED}✗ Signature-based search for registerUser failed${NC}"
    exit 1
fi

# Test 5: Java inheritance — IMPLEMENTS edge (LoggingHandler → MessageHandler)
echo ""
echo "Test 5: Verifying Java IMPLEMENTS edge for LoggingHandler → MessageHandler..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"MessageHandler\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "MessageHandler" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "LoggingHandler" || echo "$CLI_RESPONSE" | grep -q "LoggingHandler"; then
    echo -e "${GREEN}✓ Found IMPLEMENTS edge LoggingHandler → MessageHandler${NC}"
else
    echo -e "${RED}✗ Missing IMPLEMENTS edge LoggingHandler → MessageHandler${NC}"
    exit 1
fi

# Test 6: Java inheritance — IMPLEMENTS edge (UserRepository → Repository)
echo ""
echo "Test 6: Verifying Java IMPLEMENTS edge for UserRepository → Repository..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Repository\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "Repository" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "UserRepository" || echo "$CLI_RESPONSE" | grep -q "UserRepository"; then
    echo -e "${GREEN}✓ Found IMPLEMENTS edge UserRepository → Repository${NC}"
else
    echo -e "${RED}✗ Missing IMPLEMENTS edge UserRepository → Repository${NC}"
    exit 1
fi

# Test 7: Java inheritance — EXTENDS edge (AdminUser → User)
echo ""
echo "Test 7: Verifying Java EXTENDS edge for AdminUser → User..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"User\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "User" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "AdminUser" || echo "$CLI_RESPONSE" | grep -q "AdminUser"; then
    echo -e "${GREEN}✓ Found EXTENDS edge AdminUser → User${NC}"
else
    echo -e "${RED}✗ Missing EXTENDS edge AdminUser → User${NC}"
    exit 1
fi

# Test 8: Java interface extending interface (AuditableRepository → Repository)
echo ""
echo "Test 8: Verifying Java interface EXTENDS edge AuditableRepository → Repository..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Repository\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "Repository" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "AuditableRepository" || echo "$CLI_RESPONSE" | grep -q "AuditableRepository"; then
    echo -e "${GREEN}✓ Found EXTENDS edge AuditableRepository → Repository${NC}"
else
    echo -e "${RED}✗ Missing EXTENDS edge AuditableRepository → Repository${NC}"
    exit 1
fi

# Test 9: Java anonymous class implements interface (handle invocation)
echo ""
echo "Test 9: Verifying anonymous class implementing interface (EventBroadcaster calls handle on MessageHandler)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"handle\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "handle" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "sendMessage" || echo "$CLI_RESPONSE" | grep -q "sendMessage"; then
    echo -e "${GREEN}✓ Found caller sendMessage invoking handle on anonymous MessageHandler${NC}"
else
    echo -e "${RED}✗ Anonymous class handle() invocation not tracked${NC}"
    exit 1
fi

# Test 10: Java search_hybrid_context finds MessageHandler interface
echo ""
echo "Test 10: Verifying MessageHandler interface is searchable..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"MessageHandler\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
if echo "$MCP_RESPONSE" | grep -q "MessageHandler"; then
    echo -e "${GREEN}✓ MessageHandler interface found in search results${NC}"
else
    echo -e "${RED}✗ MessageHandler interface not found in search${NC}"
    exit 1
fi

# Step 5: Summarize
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Java E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated Java features:"
echo "  ✓ Java class discovery (UserService)"
echo "  ✓ Java package FQN resolution (com.example.knot.test.*)"
echo "  ✓ Java FQN field-access resolution (ChatMemory.add)"
echo "  ✓ Java method signature search (registerUser(String...))"
echo "  ✓ Java class IMPLEMENTS interface (LoggingHandler → MessageHandler)"
echo "  ✓ Java class IMPLEMENTS interface (UserRepository → Repository)"
echo "  ✓ Java class EXTENDS class (AdminUser → User)"
echo "  ✓ Java interface EXTENDS interface (AuditableRepository → Repository)"
echo "  ✓ Java anonymous class interface implementation"
echo "  ✓ Java interface search (MessageHandler)"
echo ""

exit 0
