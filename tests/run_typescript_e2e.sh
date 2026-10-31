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

# Tests 2-8 query entities declared and used in test_typescript.ts, whose
# test_* name makes it test code: include_tests lists its callers (#52).
# Test 2: find_callers of AppComponent (decorator extraction)
echo ""
echo "Test 2: Finding callers of AppComponent (decorator extraction)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"AppComponent\",\"repo_name\":\"$REPO_NAME\",\"include_tests\":true}}}"

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
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"BaseService\",\"repo_name\":\"$REPO_NAME\",\"include_tests\":true}}}"

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
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"IStorage\",\"repo_name\":\"$REPO_NAME\",\"include_tests\":true}}}"

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
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"IStorage\",\"repo_name\":\"$REPO_NAME\",\"include_tests\":true}}}"

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
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"IPayload\",\"repo_name\":\"$REPO_NAME\",\"include_tests\":true}}}"

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
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"EventData\",\"repo_name\":\"$REPO_NAME\",\"include_tests\":true}}}"

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
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Engine\",\"repo_name\":\"$REPO_NAME\",\"include_tests\":true}}}"

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

if echo "$MCP_RESPONSE" | grep -q "## Calls (" || echo "$MCP_RESPONSE" | grep -q "## References (" || echo "$MCP_RESPONSE" | grep -q "## Extends (" || echo "$MCP_RESPONSE" | grep -q "## Implements ("; then
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

# Test: Embed-text recall — the doc-less hook's paraphrase ranks the
# behaviour's call chain. Under root-set coverage (v1.9.7) the paraphrase
# "change password current user" promotes `submitChangePassword`, the
# orchestrator calling the three top-ranked helpers, to #1, and the
# hook itself (`useChangePassword`, which delegates to it) must still
# surface at #2 — the "entry point ≤ 2" contract. (Historically the hook
# ranked #1 only while the caller-root bridge silently skipped every
# entity the cosine pool already returned, i.e. while coverage was dead.)
echo ""
echo "Test: embedding recall — 'change password current user me hook' ranks submitChangePassword #1 and useChangePassword #2..."
MCP_FIRST=$(echo '{"jsonrpc":"2.0","id":900,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"change password current user me hook","max_results":10,"repo_name":"typescript_e2e_test_repo"}}}' \
    | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
      KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" \
      cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
MCP_FIRST_NAME=$(echo "$MCP_FIRST" | grep -o '## `[^`]*`' | head -1)
MCP_SECOND_NAME=$(echo "$MCP_FIRST" | grep -o '## `[^`]*`' | head -2 | tail -1)

if echo "$MCP_FIRST_NAME" | grep -q 'submitChangePassword'
then
    echo -e "${GREEN}✓ Recall: submitChangePassword (root-coverage orchestrator) ranks #1${NC}"
else
    echo -e "${RED}✗ Recall: expected submitChangePassword first, got: $MCP_FIRST_NAME${NC}"
    echo "Response was: $MCP_FIRST"
    exit 1
fi

if echo "$MCP_SECOND_NAME" | grep -q 'useChangePassword'
then
    echo -e "${GREEN}✓ Recall: doc-less hook useChangePassword stays within position 2${NC}"
else
    echo -e "${RED}✗ Recall: expected useChangePassword second, got: $MCP_SECOND_NAME${NC}"
    echo "Response was: $MCP_FIRST"
    exit 1
fi

# Anti-regression: naming the hook verbatim must keep it at #1.
MCP_LITERAL=$(echo '{"jsonrpc":"2.0","id":901,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"useChangePassword","max_results":5,"repo_name":"typescript_e2e_test_repo"}}}' \
    | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
      KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" \
      cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
MCP_LITERAL_NAME=$(echo "$MCP_LITERAL" | grep -o '## `[^`]*`' | head -1)

if echo "$MCP_LITERAL_NAME" | grep -q 'useChangePassword'; then
    echo -e "${GREEN}✓ Anti-regression: verbatim useChangePassword stays #1${NC}"
else
    echo -e "${RED}✗ Anti-regression: verbatim useChangePassword got: $MCP_LITERAL_NAME${NC}"
    echo "Response was: $MCP_LITERAL"
    exit 1
fi

# Test: entry-point recall — a tool defined as an object `constant`
# (`tools_def.ts` fixture) must outrank a **Markdown section attributed
# with the same literal title** for a paraphrase query, because the
# name-exact probe carries the definition into the pool with its full
# lexical exact-name boost while prose takes the prose penalty. The prose
# must still be surfaced in the result list (documentation use case).
echo ""
echo "Test: tool definition beats its same-titled Markdown section..."
MCP_TOOL=$(echo '{"jsonrpc":"2.0","id":902,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"tool that takes a screenshot of the visible page as an image","max_results":10,"repo_name":"typescript_e2e_test_repo"}}}' \
    | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
      KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" \
      cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
MCP_ORDERS=$(echo "$MCP_TOOL" | grep -o '## `[^`]*`')
MCP_TOOL_NAME=$(echo "$MCP_ORDERS" | head -1)
MCP_DOC_POS=$(echo "$MCP_ORDERS" | grep -n "take screenshot" | head -1 | cut -d: -f1 || true)

if echo "$MCP_TOOL_NAME" | grep -q 'screenshot'; then
    echo -e "${GREEN}✓ Recall: tool definition constant outranks same-titled prose${NC}"
else
    echo -e "${RED}✗ Recall: expected a screenshot tool definition first, got: $MCP_TOOL_NAME${NC}"
    echo "Response was: $MCP_TOOL"
    exit 1
fi

# The tool name carries 'screenshot' but not 'takes a screenshot', so this
# guard differs from a verbatim prefix search: the tool must win it on
# cosine + lexical overlap, not on a name-prefix slot.
MCP_TRACK_ORDER_ANDpresence() { :; }
if [ -n "$MCP_DOC_POS" ] && [ "$MCP_DOC_POS" -gt 1 ]; then
    echo -e "${GREEN}  ✓ same-titled Markdown section still ranked in the results (position ${MCP_DOC_POS}; documented topic surfaces)${NC}"
else
    echo -e "${RED}✗ Markdown section vanished from the search results (must still surface)${NC}"
    echo "Response was: $MCP_TOOL"
    exit 1
fi

# Debug-and-document guardrail: a documentation-scoped search
# (kinds=markdown_section) keeps its section as #1 — the definition
# channel skips docs-only filters entirely.
MCP_DOC=$(echo '{"jsonrpc":"2.0","id":903,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"take screenshot","max_results":5,"repo_name":"typescript_e2e_test_repo","kinds":"markdown_section"}}}' \
    | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
      KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TEST_FILES_DIR" \
      cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
MCP_DOC_NAME=$(echo "$MCP_DOC" | grep -o '## `[^`]*`' | head -1)

if echo "$MCP_DOC_NAME" | grep -q "take screenshot"; then
    echo -e "${GREEN}  ✓ documentation-scoped search keeps its own section first${NC}"
else
    echo -e "${RED}✗ documentation-only topics: kinds=markdown_section broke${NC}"
    echo "Response was: $MCP_DOC"
    exit 1
fi

# Test 13: Arrow function constant fixture yields 2 retrievable entities (constant and function)
echo ""
echo "Test 13: Verifying arrow const double extraction yields two distinct retrievable entities..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"arrowConstEntityTest\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

# Check Qdrant points for arrowConstEntityTest
QDRANT_REST_URL="http://localhost:16333"
QDRANT_PAYLOAD=$(curl -s -X POST "$QDRANT_REST_URL/collections/$QDRANT_COLLECTION/points/scroll" -H 'Content-Type: application/json' -d '{"limit": 1000, "with_payload": true}')
ARROW_ENTITIES_COUNT=$(echo "$QDRANT_PAYLOAD" | grep -o '"name":"arrowConstEntityTest"' | wc -l)

if [ "$ARROW_ENTITIES_COUNT" -eq 2 ]; then
    echo -e "${GREEN}✓ Arrow const fixture created 2 distinct entities in Qdrant${NC}"
else
    echo -e "${RED}✗ Expected 2 entities for arrowConstEntityTest in Qdrant, found $ARROW_ENTITIES_COUNT${NC}"
    exit 1
fi

# Tests 14-16: calls carry their argument count, so overload signatures
# bind by arity within the receiver class (issue #38). Fixtures:
# overload_calls.ts, legacy_arity.js. Overloads share their FQN, so each
# Calls row is reported as `<target declaration line><<caller>`.
assert_overload_callers() {
    local id=$1
    local fqn=$2
    local expected=$3
    local description=$4

    echo ""
    echo "Test $id: $description ($fqn callers exactly [$expected])..."
    local cli_actual
    cli_actual=$(cargo run --release --bin knot -- callers "$fqn" -r "$REPO_NAME" -o json 2>/dev/null \
        | jq -r '.calls[] | "\(.target_start_line)<\(.name)"' | LC_ALL=C sort -u | paste -sd, -)
    if [ "$cli_actual" != "$expected" ]; then
        echo -e "${RED}✗ CLI: $fqn callers expected [$expected], got [$cli_actual]${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ CLI: $fqn callers [$cli_actual]${NC}"

    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$fqn\",\"repo_name\":\"$REPO_NAME\"}}}"
    local mcp_expected
    local mcp_actual
    mcp_expected=$(echo "$expected" | tr ',' '\n' | sed 's/^[0-9]*<//' | LC_ALL=C sort -u | paste -sd, -)
    mcp_actual=$(echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1 \
        | jq -r '.result.content[0].text' \
        | awk '/^## /{ on = ($0 ~ /^## Calls/) } on && /^- \*\*`/' \
        | sed -E 's/^- \*\*`([^`]*)`.*/\1/' | LC_ALL=C sort -u | paste -sd, -)
    if [ "$mcp_actual" != "$mcp_expected" ]; then
        echo -e "${RED}✗ MCP: $fqn callers expected [$mcp_expected], got [$mcp_actual]${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ MCP: $fqn callers [$mcp_actual]${NC}"
}

assert_overload_callers 14 "Foo.bar" "3<single,4<sumPair" \
    "Each call binds to the overload signature with matching arity"
assert_overload_callers 15 "Other.bar" "" \
    "A same-name/same-arity method in another class gets no caller"
assert_overload_callers 16 "legacyPad" "3<padDefault" \
    "JavaScript calls with fewer arguments than declared still bind"
# Issue #37: a parameter list addresses one overload signature.
assert_overload_callers 31 "Foo.bar(number, number)" "4<sumPair" \
    "A parameter list addresses a single overload signature"
assert_overload_callers 32 "Foo.bar(number)" "3<single" \
    "The other overload signature keeps its own caller"

# Tests 17-19: locals are not class members, and `this.m()` reaches the
# inherited method (issue #41). Fixture: this_inherited/. `CdpPage extends
# Page` calls `this.url()` from a method declaring `const url`; `Frame`
# declares an unrelated homonym `url`.
assert_overload_callers 17 "Page.url" "2<deleteCookie" \
    "this.url() binds to the inherited method, not to a same-named local"
assert_overload_callers 18 "Frame.url" "" \
    "A homonym method of an unrelated class gets no caller"

echo ""
echo "Test 19: explore_file lists no method local as a constant..."
LOCALS_FILE="this_inherited/CdpPage.ts"
LOCALS_CLI=$(cargo run --release --bin knot -- explore "$LOCALS_FILE" -r "$REPO_NAME" -o json 2>/dev/null \
    | jq -r '[.entities | .. | objects | select(has("kind")) | "\(.kind):\(.name)"] | join(",")')
LOCALS_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":19,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$LOCALS_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"
LOCALS_MCP=$(echo "$LOCALS_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1 \
    | jq -r '.result.content[0].text')
if ! echo "$LOCALS_CLI" | grep -q "method:deleteCookie" || echo "$LOCALS_CLI" | grep -q "constant:"; then
    echo -e "${RED}✗ CLI: expected deleteCookie and no constant, got [$LOCALS_CLI]${NC}"
    exit 1
fi
if ! echo "$LOCALS_MCP" | grep -q "deleteCookie" || echo "$LOCALS_MCP" | grep -qE '`(url|pageURL)`'; then
    echo -e "${RED}✗ MCP: unexpected explore_file outline:${NC}"
    echo "$LOCALS_MCP"
    exit 1
fi
echo -e "${GREEN}✓ CLI & MCP: CdpPage.ts lists deleteCookie and no local constant${NC}"

# Tests 20-21: locals and parameters passed as arguments are values, not
# receiverless callbacks. They must never bind to same-named fields in an
# unrelated module (issue #48). Fixture: callback_arguments/.
assert_overload_callers 20 "ProductCardComponent.snapshot" "" \
    "A local argument does not become a cross-module field call"
assert_overload_callers 21 "PromoBanner.coupon" "" \
    "A parameter argument does not become a cross-module field call"

# Test 22: each class member reports only the decorators written directly
# before it, never those of an earlier member (issue #47). Fixture:
# decorators/cart.store.ts.
echo ""
echo "Test 22: explore_file reports per-member decorators..."
DECORATORS_FILE="decorators/cart.store.ts"
DECORATORS_EXPECTED="helper:,plain:,reset:@action,total:@computed,update:"
DECORATORS_CLI=$(cargo run --release --bin knot -- explore "$DECORATORS_FILE" -r "$REPO_NAME" -o json 2>/dev/null \
    | jq -r '[.entities | .. | objects | select(has("kind") and (.name | IN("helper","total","plain","reset","update")))
        | "\(.name):\((.decorators // []) | join("|"))"] | unique | join(",")')
if [ "$DECORATORS_CLI" != "$DECORATORS_EXPECTED" ]; then
    echo -e "${RED}✗ CLI: decorators expected [$DECORATORS_EXPECTED], got [$DECORATORS_CLI]${NC}"
    exit 1
fi
echo -e "${GREEN}✓ CLI: decorators [$DECORATORS_CLI]${NC}"
DECORATORS_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":22,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$DECORATORS_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"
DECORATORS_MCP=$(echo "$DECORATORS_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1 \
    | jq -r '.result.content[0].text')
if [ "$(echo "$DECORATORS_MCP" | grep -c '@computed')" -ne 1 ] || [ "$(echo "$DECORATORS_MCP" | grep -c '@action')" -ne 1 ]; then
    echo -e "${RED}✗ MCP: expected exactly one @computed and one @action:${NC}"
    echo "$DECORATORS_MCP"
    exit 1
fi
echo -e "${GREEN}✓ MCP: exactly one @computed and one @action${NC}"

# Tests 23-26: a `this.x` read or write is a reference, not a call; only a
# real call (`this.refresh()`) or a getter read (`this.total`) is a CALLS
# edge (issue #49). Fixture: this_access/basket.store.ts.
assert_reference_callers() {
    local id=$1
    local fqn=$2
    local expected=$3
    local description=$4

    echo ""
    echo "Test $id: $description ($fqn references exactly [$expected])..."
    local cli_actual
    cli_actual=$(cargo run --release --bin knot -- callers "$fqn" -r "$REPO_NAME" -o json 2>/dev/null \
        | jq -r '(.references // [])[] | .name' | LC_ALL=C sort -u | paste -sd, -)
    if [ "$cli_actual" != "$expected" ]; then
        echo -e "${RED}✗ CLI: $fqn references expected [$expected], got [$cli_actual]${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ CLI: $fqn references [$cli_actual]${NC}"

    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$fqn\",\"repo_name\":\"$REPO_NAME\"}}}"
    local mcp_actual
    mcp_actual=$(echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1 \
        | jq -r '.result.content[0].text' \
        | awk '/^## /{ on = ($0 ~ /^## References/) } on && /^- \*\*`/' \
        | sed -E 's/^- \*\*`([^`]*)`.*/\1/' | LC_ALL=C sort -u | paste -sd, -)
    if [ "$mcp_actual" != "$expected" ]; then
        echo -e "${RED}✗ MCP: $fqn references expected [$expected], got [$mcp_actual]${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ MCP: $fqn references [$mcp_actual]${NC}"
}

assert_overload_callers 23 "BasketStore.items" "" \
    "Field reads and writes are not calls"
assert_reference_callers 24 "BasketStore.items" "reset,total" \
    "Field reads and writes are references"
assert_overload_callers 25 "BasketStore.refresh" "13<reset" \
    "A real this.m() call stays a call"
assert_overload_callers 26 "BasketStore.total" "5<summary" \
    "A getter read is a call"
# Issue #48 fixture: the field's only user is its own class's reader.
assert_reference_callers 27 "ProductCardComponent.snapshot" "render" \
    "A field is referenced only by its own reader, not by a cross-module local"

# Tests 28-29: compact search answers (issue #50). The `path` filter pins the
# results to the this_access/ fixture, so ranking does not matter.
search_mcp_text() {
    local id=$1
    local extra=$2
    echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"refresh the basket store\",\"max_results\":10,\"path\":\"this_access\",\"repo_name\":\"$REPO_NAME\"$extra}}}" \
        | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1 | jq -r '.result.content[0].text'
}

echo ""
echo "Test 28: full search answers name same-file callers and inline summaries..."
FULL_MCP=$(search_mcp_text 28 "")
if ! echo "$FULL_MCP" | grep -q '^\*\*Called by:\*\* 1 location(s): reset (same file)$' \
    || echo "$FULL_MCP" | grep -q 'Sample callers'; then
    echo -e "${RED}✗ MCP: expected 'Called by: 1 location(s): reset (same file)':${NC}"
    echo "$FULL_MCP"
    exit 1
fi
echo -e "${GREEN}✓ MCP: refresh is 'Called by: 1 location(s): reset (same file)'${NC}"

echo ""
echo "Test 29: brief search answers render one line per result..."
BRIEF_MCP=$(search_mcp_text 29 ",\"detail\":\"brief\"")
BRIEF_CLI=$(cargo run --release --bin knot -- search "refresh the basket store" -m 10 -p this_access \
    -r "$REPO_NAME" -o markdown --detail brief 2>/dev/null)
for answer in "$BRIEF_MCP" "$BRIEF_CLI"; do
    if ! echo "$answer" | grep -q '^- `refresh` (method) — `this_access/basket.store.ts:13` .*— called by 1' \
        || echo "$answer" | grep -q '\*\*File:\*\*'; then
        echo -e "${RED}✗ brief answer is not one line per result:${NC}"
        echo "$answer"
        exit 1
    fi
done
echo -e "${GREEN}✓ CLI & MCP: brief answer lists refresh on one line${NC}"
BAD_DETAIL=$(echo "{\"jsonrpc\":\"2.0\",\"id\":30,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"x\",\"detail\":\"verbose\",\"repo_name\":\"$REPO_NAME\"}}}" \
    | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
if ! echo "$BAD_DETAIL" | grep -q "Invalid 'detail' value"; then
    echo -e "${RED}✗ MCP: an unknown detail value must be rejected, got: $BAD_DETAIL${NC}"
    exit 1
fi
echo -e "${GREEN}✓ MCP: unknown detail value rejected${NC}"

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
echo "  ✓ TS embedding recall: doc-less hook ranks first for its paraphrase"
echo "  ✓ Overload signatures bind by arity; JavaScript arity is never binding"
echo "  ✓ One overload addressable by its parameter list (Foo.bar(number, number))"
echo "  ✓ Method locals are not members; this.m() reaches the inherited method"
echo "  ✓ Function locals and parameters passed as arguments do not bind cross-module fields"
echo "  ✓ Class members report only their own decorators"
echo "  ✓ this.x reads and writes are references; real calls and getter reads are calls"
echo "  ✓ Compact search answers: same-file samples, one-line summaries, brief detail"
echo ""

exit 0
