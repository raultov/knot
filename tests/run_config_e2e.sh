#!/usr/bin/env bash
# E2E Integration Test Script for Configuration File Support in knot (v1.2.6)
#
# This script tests YAML, JSON, .properties, and package.json extraction:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports
# 2. Indexes sample YAML, JSON, .properties, and package.json fixtures
# 3. Queries via MCP and CLI to validate ConfigProperty and BuildDependency extraction
# 4. Cleans up containers and data
#
# Usage: ./tests/run_config_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_config_data"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_config_e2e_test"
REPO_NAME="config_e2e_test_repo"

TMP_REPO_DIR="$SCRIPT_DIR/.e2e_config_repo"

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Configuration Files E2E Test${NC}"
echo -e "${BLUE}Phase B — YAML, JSON, .properties, package.json${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    local exit_code=$?

    # Always tear down containers — leaving them up blocks the shared port 17687
    # for subsequent suites in run_all_e2e.sh and causes false cascading failures.
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Config E2E tests failed!${NC}"
        echo -e "${YELLOW}Test data preserved at $E2E_DATA_DIR for inspection.${NC}"
        echo -e "${YELLOW}Manual cleanup:  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR${NC}"
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Config E2E test environment...${NC}"
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

echo -e "${YELLOW}[1/5] Starting Docker containers for Config E2E test...${NC}"
cd "$SCRIPT_DIR"
docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
if [ -d "$E2E_DATA_DIR" ]; then
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
fi

docker compose -f "$COMPOSE_FILE" up -d

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

echo -e "${YELLOW}[3/5] Phase A — Indexing WITHOUT --include-config-files (skip mode)...${NC}"
cd "$PROJECT_ROOT"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp "$TEST_FILES_DIR/sample_application.yml" "$TMP_REPO_DIR/application.yml"
cp "$TEST_FILES_DIR/sample_config.json" "$TMP_REPO_DIR/config.json"
cp "$TEST_FILES_DIR/sample_app.properties" "$TMP_REPO_DIR/app.properties"
cp "$TEST_FILES_DIR/sample_package.json" "$TMP_REPO_DIR/package.json"

export KNOT_REPO_PATH="$TMP_REPO_DIR"
export KNOT_REPO_NAME="$REPO_NAME"
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer WITHOUT --include-config-files (config files should be skipped)..."
cargo run --release --bin knot-indexer -- --clean

echo -e "${GREEN}✓ Phase A indexing complete (config files skipped by default)${NC}"

echo -e "${YELLOW}[3b/5] Validating skip-mode — config files NOT indexed, build-system files ARE...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test A1: Search for YAML config property — should NOT be found
echo ""
echo "Test A1: Searching for YAML property 'datasource url' (should NOT be found)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":101,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"datasource url\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "datasource url" -r "$REPO_NAME" -m 10 2>/dev/null || true)

# YAML entities should NOT appear since --include-config-files was not passed
if echo "$MCP_RESPONSE" | grep -qE "datasource|spring\.datasource" 2>/dev/null; then
    echo -e "${RED}✗ UNEXPECTED: YAML config property was indexed (should have been skipped)${NC}"
    echo "MCP: $MCP_RESPONSE"
    exit 1
fi
if echo "$CLI_RESPONSE" | grep -qE "application\.yml" 2>/dev/null; then
    echo -e "${RED}✗ UNEXPECTED: YAML config file appears in CLI results (should have been skipped)${NC}"
    exit 1
fi
echo -e "${GREEN}✓ YAML config properties correctly skipped${NC}"

# Test A2: Search for .properties entry — should NOT be found
echo ""
echo "Test A2: Searching for properties 'app.name' (should NOT be found)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":102,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"app.name MyApplication\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -qi "MyApplication" 2>/dev/null; then
    echo -e "${RED}✗ UNEXPECTED: .properties config was indexed (should have been skipped)${NC}"
    exit 1
fi
echo -e "${GREEN}✓ .properties config correctly skipped${NC}"

# Test A3: Search for generic JSON config — should NOT be found
echo ""
echo "Test A3: Searching for generic JSON 'compilerOptions' (should NOT be found)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":103,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"compilerOptions\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "compilerOptions" -r "$REPO_NAME" -m 10 2>/dev/null || true)

if echo "$MCP_RESPONSE" | grep -qi "compilerOptions" 2>/dev/null; then
    echo -e "${RED}✗ UNEXPECTED: generic JSON config was indexed (should have been skipped)${NC}"
    exit 1
fi
if echo "$CLI_RESPONSE" | grep -qi "compilerOptions" 2>/dev/null; then
    echo -e "${RED}✗ UNEXPECTED: generic JSON config in CLI results${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Generic JSON config correctly skipped${NC}"

# Test A4: Search for package.json dependency — MUST be found (build-system, always indexed)
echo ""
echo "Test A4: Searching for npm dependency 'express' (MUST be found — build-system)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":104,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"express dependency\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "express" -r "$REPO_NAME" -m 10 2>/dev/null || true)

if echo "$MCP_RESPONSE" | grep -qi "express" 2>/dev/null; then
    echo -e "${GREEN}✓ package.json dependency found (build-system files always indexed)${NC}"
else
    echo -e "${RED}✗ package.json dependency NOT found (build-system files should always be indexed)${NC}"
    exit 1
fi

# Test A5: Explore package.json — should show entities
echo ""
echo "Test A5: Exploring package.json (should show entities)..."
PKG_FILE="$TMP_REPO_DIR/package.json"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":105,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$PKG_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$PKG_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null || true)

if echo "$MCP_RESPONSE" | grep -qE "express|jest" 2>/dev/null && echo "$CLI_RESPONSE" | grep -qE "express|jest|configuration" 2>/dev/null; then
    echo -e "${GREEN}✓ package.json entities found via explore (build-system always indexed)${NC}"
else
    echo -e "${RED}✗ package.json entities not found${NC}"
    exit 1
fi

# Test A6: Explore application.yml — should return empty or not be listed
echo ""
echo "Test A6: Exploring application.yml (should NOT show config entities)..."
YML_FILE="$TMP_REPO_DIR/application.yml"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":106,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$YML_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -qE "port|datasource|config_property" 2>/dev/null; then
    echo -e "${RED}✗ UNEXPECTED: YAML config entities found (should have been skipped)${NC}"
    exit 1
fi
echo -e "${GREEN}✓ YAML config entities correctly skipped in explore${NC}"

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}Phase A passed — config files skipped by default${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""

# Now re-index with --include-config-files for the full suite
echo -e "${YELLOW}[3c/5] Phase B — Re-indexing WITH --include-config-files...${NC}"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp "$TEST_FILES_DIR/sample_application.yml" "$TMP_REPO_DIR/application.yml"
cp "$TEST_FILES_DIR/sample_config.json" "$TMP_REPO_DIR/tsconfig.json"
cp "$TEST_FILES_DIR/sample_app.properties" "$TMP_REPO_DIR/app.properties"
cp "$TEST_FILES_DIR/sample_package.json" "$TMP_REPO_DIR/package.json"

echo "Running indexer WITH --include-config-files..."
cargo run --release --bin knot-indexer -- --clean --include-config-files

echo -e "${GREEN}✓ Config files indexed${NC}"

echo -e "${YELLOW}[4/5] Validating config entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Search for YAML property
echo ""
echo "Test 1: Searching for YAML property datasource url..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"datasource url\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "datasource url" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qE "datasource|url" && echo "$CLI_RESPONSE" | grep -qE "datasource|url"; then
    echo -e "${GREEN}✓ Found YAML config property (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ YAML config property not found${NC}"
    exit 1
fi

# Test 2: Explore application.yml
echo ""
echo "Test 2: Exploring application.yml..."
YML_FILE="$TMP_REPO_DIR/application.yml"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$YML_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$YML_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qE "port|datasource|config_property" && echo "$CLI_RESPONSE" | grep -qE "Configuration Properties|config_property"; then
    echo -e "${GREEN}✓ application.yml config properties found via explore (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ application.yml config properties not found${NC}"
    exit 1
fi

# Test 3: Search for JSON config property (tsconfig)
echo ""
echo "Test 3: Searching for JSON config compilerOptions..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"compilerOptions target\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "compilerOptions" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qi "compilerOptions" && echo "$CLI_RESPONSE" | grep -qi "target"; then
    echo -e "${GREEN}✓ Found JSON config property compilerOptions (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ JSON config property compilerOptions not found${NC}"
    exit 1
fi

# Test 4: Search for npm dependency in package.json
echo ""
echo "Test 4: Searching for npm dependency express..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"express dependency\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "express" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qi "express" && echo "$CLI_RESPONSE" | grep -qi "express"; then
    echo -e "${GREEN}✓ Found npm dependency express (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ npm dependency express not found${NC}"
    exit 1
fi

# Test 5: Explore app.properties
echo ""
echo "Test 5: Exploring app.properties..."
PROPS_FILE="$TMP_REPO_DIR/app.properties"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$PROPS_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$PROPS_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qE "datasource|config_property" && echo "$CLI_RESPONSE" | grep -qE "Configuration Properties|config_property"; then
    echo -e "${GREEN}✓ app.properties config properties found via explore (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ app.properties config properties not found${NC}"
    exit 1
fi

# Test 6: Explore package.json — dependencies + scripts
echo ""
echo "Test 6: Exploring package.json for dependencies and scripts..."
PKG_FILE="$TMP_REPO_DIR/package.json"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$PKG_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$PKG_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qE "express|jest" && echo "$CLI_RESPONSE" | grep -qE "express|jest|configuration"; then
    echo -e "${GREEN}✓ package.json dependencies and scripts found via explore (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ package.json entities not found${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Config Files E2E tests passed!${NC}"
echo -e "${GREEN}========================================${NC}"
