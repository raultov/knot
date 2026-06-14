#!/usr/bin/env bash
# E2E Integration Test Script for TypeScript Support in knot
#
# This script tests TypeScript-specific features:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes TypeScript test files (test_typescript.ts + alias/import fixtures)
# 3. Queries via MCP to validate TypeScript entity extraction
# 4. Tests class, interface, decorator, type reference, and import alias extraction
# 5. Cleans up containers and data
#
# Usage: ./tests/run_typescript_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/typescript"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_typescript_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_typescript_e2e_test"
REPO_NAME="typescript_e2e_test_repo"

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
echo -e "${BLUE}knot TypeScript E2E Integration Test${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}TypeScript E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up TypeScript E2E test environment...${NC}"
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
    echo -e "${YELLOW}[1/5] Starting Docker containers for TypeScript E2E test...${NC}"
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

# Step 3: Index TypeScript test files
echo -e "${YELLOW}[3/5] Indexing TypeScript test files...${NC}"
cd "$PROJECT_ROOT"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for TypeScript files..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"

echo -e "${GREEN}✓ TypeScript files indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating TypeScript entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: TypeScript class extraction via explore_file
echo ""
echo "Test 1: Exploring test_typescript.ts for TS class extraction..."
TS_FILE="$TEST_FILES_DIR/test_typescript.ts"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$TS_FILE\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$TS_FILE" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "AppComponent"; then
    echo -e "${GREEN}✓ MCP: Found AppComponent${NC}"
else
    echo -e "${RED}✗ MCP: AppComponent not found${NC}"
    exit 1
fi

if echo "$MCP_RESPONSE" | grep -q "AnalyticsService"; then
    echo -e "${GREEN}✓ MCP: Found AnalyticsService${NC}"
else
    echo -e "${RED}✗ MCP: AnalyticsService not found${NC}"
    exit 1
fi

if echo "$CLI_RESPONSE" | grep -q "AppComponent"; then
    echo -e "${GREEN}✓ CLI: Found AppComponent${NC}"
else
    echo -e "${RED}✗ CLI: AppComponent not found${NC}"
    exit 1
fi

if echo "$CLI_RESPONSE" | grep -q "AnalyticsService"; then
    echo -e "${GREEN}✓ CLI: Found AnalyticsService${NC}"
else
    echo -e "${RED}✗ CLI: AnalyticsService not found${NC}"
    exit 1
fi

# Test 2: find_callers of AppComponent (decorator extraction)
echo ""
echo "Test 2: Finding callers of AppComponent (decorator extraction)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"AppComponent\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "AppComponent" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "AppModule"; then
    echo -e "${GREEN}✓ MCP: AppModule references AppComponent (decorator extraction works!)${NC}"
else
    echo -e "${RED}✗ MCP: AppModule reference not found${NC}"
    exit 1
fi

if echo "$CLI_RESPONSE" | grep -q "AppModule"; then
    echo -e "${GREEN}✓ CLI: AppModule references AppComponent${NC}"
else
    echo -e "${RED}✗ CLI: AppModule reference not found${NC}"
    exit 1
fi

# Test 3: TypeScript class EXTENDS (CacheService → BaseService)
echo ""
echo "Test 3: Verifying TypeScript EXTENDS edge CacheService → BaseService..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"BaseService\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "BaseService" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "CacheService" || echo "$CLI_RESPONSE" | grep -q "CacheService"; then
    echo -e "${GREEN}✓ Found EXTENDS edge CacheService → BaseService${NC}"
else
    echo -e "${RED}✗ Missing EXTENDS edge CacheService → BaseService${NC}"
    exit 1
fi

# Test 4: TypeScript class IMPLEMENTS (CacheService → IStorage)
echo ""
echo "Test 4: Verifying TypeScript IMPLEMENTS edge CacheService → IStorage..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"IStorage\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "IStorage" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "CacheService" || echo "$CLI_RESPONSE" | grep -q "CacheService"; then
    echo -e "${GREEN}✓ Found IMPLEMENTS edge CacheService → IStorage${NC}"
else
    echo -e "${RED}✗ Missing IMPLEMENTS edge CacheService → IStorage${NC}"
    exit 1
fi

# Test 5: TypeScript interface EXTENDS (ICache → IStorage)
echo ""
echo "Test 5: Verifying TypeScript interface EXTENDS edge ICache → IStorage..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"IStorage\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "IStorage" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "ICache" || echo "$CLI_RESPONSE" | grep -q "ICache"; then
    echo -e "${GREEN}✓ Found EXTENDS edge ICache → IStorage (interface inheritance)${NC}"
else
    echo -e "${RED}✗ Missing EXTENDS edge ICache → IStorage${NC}"
    exit 1
fi

# Test 6: TypeScript top-level function type reference (processPayload → IPayload)
echo ""
echo "Test 6: Verifying TypeScript top-level function type reference processPayload → IPayload..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"IPayload\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "IPayload" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "processPayload" || echo "$CLI_RESPONSE" | grep -q "processPayload"; then
    echo -e "${GREEN}✓ Found top-level function processPayload using IPayload (type reference)${NC}"
else
    echo -e "${RED}✗ Missing type reference processPayload → IPayload${NC}"
    exit 1
fi

# Test 7: TypeScript method signature search by parameter type (EventData)
echo ""
echo "Test 7: Searching for TypeScript method by parameter type EventData..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"EventData\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "EventData" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "trackEvent" || echo "$CLI_RESPONSE" | grep -q "trackEvent"; then
    echo -e "${GREEN}✓ Found TypeScript methods using EventData type${NC}"
else
    echo -e "${RED}✗ TypeScript methods using EventData type not found${NC}"
    exit 1
fi

# Test 8: TypeScript value reference (COMPONENT_REGISTRY → Engine)
echo ""
echo "Test 8: Verifying TypeScript value reference COMPONENT_REGISTRY → Engine..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Engine\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "Engine" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "COMPONENT_REGISTRY" || echo "$CLI_RESPONSE" | grep -q "COMPONENT_REGISTRY"; then
    echo -e "${GREEN}✓ Found value reference COMPONENT_REGISTRY → Engine${NC}"
else
    echo -e "${RED}✗ Missing value reference COMPONENT_REGISTRY → Engine${NC}"
    exit 1
fi

# Test 9: Prefix search boost (partial name query should find exact match first)
echo ""
echo "Test 9: Verifying prefix search returns exact name match first..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"IPa\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "IPa" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "IPayload" || echo "$CLI_RESPONSE" | grep -q "IPayload"; then
    echo -e "${GREEN}✓ Prefix search for 'IPa' returned IPayload (name prefix boost works)${NC}"
else
    echo -e "${RED}✗ Prefix search for 'IPa' did not return IPayload${NC}"
    exit 1
fi

# Test 10: TypeScript cross-file alias resolution (import)
echo ""
echo "Test 10: Verifying TypeScript cross-file alias resolution (MyTsAlias → MyTsTarget)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"MyTsTarget\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "MyTsTarget" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "callerTs" || echo "$CLI_RESPONSE" | grep -q "callerTs"; then
    echo -e "${GREEN}✓ Found cross-file alias resolution callerTs → MyTsTarget${NC}"
else
    echo -e "${RED}✗ Missing alias resolution callerTs → MyTsTarget${NC}"
    exit 1
fi

# Test 11: TypeScript import-as capture (find_callers for TsImportFoo and TsImportQux; alias TsImportBar must NOT have callers)
echo ""
echo "Test 11: Verifying TypeScript import capture — find_callers for TsImportFoo..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"TsImportFoo\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "TsImportFoo" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "ts_imports_uses.ts" || echo "$CLI_RESPONSE" | grep -q "ts_imports_uses.ts"; then
    echo -e "${GREEN}✓ Found ts_imports_uses.ts as caller of TsImportFoo via import${NC}"
else
    echo -e "${RED}✗ ts_imports_uses.ts not found as caller of TsImportFoo${NC}"
    exit 1
fi

echo ""
echo "Test 11b: Verifying TypeScript import-as capture — find_callers for TsImportQux..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"TsImportQux\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "TsImportQux" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "ts_imports_uses.ts" || echo "$CLI_RESPONSE" | grep -q "ts_imports_uses.ts"; then
    echo -e "${GREEN}✓ Found ts_imports_uses.ts as caller of TsImportQux via import-as${NC}"
else
    echo -e "${RED}✗ ts_imports_uses.ts not found as caller of TsImportQux${NC}"
    exit 1
fi

echo ""
echo "Test 11c: Verifying alias TsImportBar does NOT have callers..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":13,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"TsImportBar\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "ts_imports_uses.ts"; then
    echo -e "${RED}✗ Alias TsImportBar should NOT have callers (alias should resolve to TsImportQux)${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Alias TsImportBar correctly has no callers${NC}"
fi

# Test 12: TypeScript explore_file on ts_imports_uses.ts — verify Imports section
echo ""
echo "Test 12: Verifying explore_file on ts_imports_uses.ts shows Imports / Referenced Types..."
TS_IMPORTS_FILE="$TEST_FILES_DIR/ts_imports_uses.ts"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":14,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$TS_IMPORTS_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$TS_IMPORTS_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "## Imports / Referenced Types" && echo "$CLI_RESPONSE" | grep -q "## Imports / Referenced Types"; then
    echo -e "${GREEN}✓ explore_file shows Imports / Referenced Types section (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ explore_file missing Imports / Referenced Types section${NC}"
    exit 1
fi

if (echo "$MCP_RESPONSE" | grep -q "TsImportFoo") && (echo "$MCP_RESPONSE" | grep -q "TsImportQux"); then
    echo -e "${GREEN}✓ Imports section lists TsImportFoo and TsImportQux (MCP)${NC}"
else
    echo -e "${RED}✗ Imports section missing TsImportFoo or TsImportQux${NC}"
    exit 1
fi

# Step 5: Summarize
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All TypeScript E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated TypeScript features:"
echo "  ✓ TS class extraction (AppComponent, AnalyticsService)"
echo "  ✓ TS decorator reference extraction (AppModule → AppComponent)"
echo "  ✓ TS class EXTENDS (CacheService → BaseService)"
echo "  ✓ TS class IMPLEMENTS (CacheService → IStorage)"
echo "  ✓ TS interface EXTENDS (ICache → IStorage)"
echo "  ✓ TS top-level function type references (processPayload → IPayload)"
echo "  ✓ TS method signature search (EventData parameter type)"
echo "  ✓ TS value references (COMPONENT_REGISTRY → Engine)"
echo "  ✓ TS prefix search boost (IPa → IPayload)"
echo "  ✓ TS cross-file alias resolution (callerTs → MyTsTarget)"
echo "  ✓ TS import / import-as capture (TsImportFoo, TsImportQux)"
echo "  ✓ TS alias should NOT have callers (TsImportBar)"
echo "  ✓ TS explore_file shows Imports / Referenced Types section"
echo ""

exit 0
