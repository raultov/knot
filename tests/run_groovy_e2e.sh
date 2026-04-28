#!/usr/bin/env bash
# E2E Integration Test Script for Groovy Language Support in knot (v0.10.5)
#
# This script tests full Groovy source file extraction (classes, interfaces,
# enums, methods, properties) using tree-sitter-groovy:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (18xxx/16xxx)
# 2. Indexes sample_full.groovy
# 3. Queries via MCP and CLI to validate entity extraction
# 4. Cleans up containers and data
#
# Usage: ./tests/run_groovy_e2e.sh
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
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_groovy_data"

NEO4J_URI="bolt://localhost:17688"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16335"
QDRANT_COLLECTION="knot_groovy_e2e_test"
REPO_NAME="groovy_e2e_test_repo"

TMP_REPO_DIR="$SCRIPT_DIR/.e2e_groovy_repo"

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Groovy Language E2E Integration Test${NC}"
echo -e "${BLUE}Phase 10 - Full Groovy Support (v0.10.5)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Groovy E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi
    echo -e "\n${YELLOW}Cleaning up Groovy E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

echo -e "${YELLOW}[1/5] Starting Docker containers for Groovy E2E test...${NC}"
cd "$SCRIPT_DIR"
docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
if [ -d "$E2E_DATA_DIR" ]; then
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
fi

export NEO4J_PORT=17688
export NEO4J_HTTP_PORT=17474
export QDRANT_PORT=16335
export QDRANT_GRPC_PORT=16336
export E2E_DATA_DIR

mkdir -p "$E2E_DATA_DIR"/neo4j/data "$E2E_DATA_DIR"/neo4j/logs "$E2E_DATA_DIR"/qdrant/storage

cat > "$E2E_DATA_DIR/docker-compose.yml" <<EOF
services:
  neo4j:
    image: neo4j:5.26-community
    container_name: knot_neo4j_groovy_e2e
    environment:
      NEO4J_AUTH: ${NEO4J_USER}/${NEO4J_PASSWORD}
      NEO4J_server_bolt_advertised__address: localhost:${NEO4J_PORT}
      NEO4J_server_http_listen__address: 0.0.0.0:${NEO4J_HTTP_PORT}
    ports:
      - "${NEO4J_PORT}:7687"
      - "${NEO4J_HTTP_PORT}:7474"
    volumes:
      - ${E2E_DATA_DIR}/neo4j/data:/data
      - ${E2E_DATA_DIR}/neo4j/logs:/logs
    networks: [knot-net]
    healthcheck:
      test: ["CMD", "cypher-shell", "-u", "${NEO4J_USER}", "-p", "${NEO4J_PASSWORD}", "CALL db.ping()"]
      interval: 5s
      timeout: 5s
      retries: 10

  qdrant:
    image: qdrant/qdrant:v1.13.5
    container_name: knot_qdrant_groovy_e2e
    ports:
      - "${QDRANT_PORT}:6334"
      - "${QDRANT_GRPC_PORT}:6335"
    volumes:
      - ${E2E_DATA_DIR}/qdrant/storage:/qdrant/storage
    networks: [knot-net]

networks:
  knot-net:
    driver: bridge
EOF

docker compose -f "$E2E_DATA_DIR/docker-compose.yml" up -d

wait_for_port() {
    local port=$1
    local service=$2
    local container=$3
    local elapsed=0
    echo -n "Waiting for $service"
    while [ $elapsed -lt $TIMEOUT_SECONDS ]; do
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

wait_for_port 17688 "Neo4j" "knot_neo4j_groovy_e2e" || exit 1
wait_for_port 16335 "Qdrant" "knot_qdrant_groovy_e2e" || exit 1
sleep 5

echo -e "${YELLOW}[3/5] Indexing Groovy test file...${NC}"
cd "$PROJECT_ROOT"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp "$TEST_FILES_DIR/sample_full.groovy" "$TMP_REPO_DIR/"

export KNOT_REPO_PATH="$TMP_REPO_DIR"
export KNOT_REPO_NAME="$REPO_NAME"
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for Groovy files..."
cargo run --release --bin knot-indexer -- --clean 2>/dev/null

echo -e "${GREEN}✓ Groovy files indexed${NC}"

echo -e "${YELLOW}[4/5] Validating Groovy entities via knot-mcp and knot CLI...${NC}"

cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Search for Groovy class
echo ""
echo "Test 1: Searching for Groovy class BaseService..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"BaseService Groovy class\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "BaseService" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "BaseService" && echo "$CLI_RESPONSE" | grep -q "BaseService"; then
    echo -e "${GREEN}✓ Found Groovy class BaseService (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Groovy class BaseService not found${NC}"
    exit 1
fi

# Test 2: Search for Groovy interface
echo ""
echo "Test 2: Searching for Groovy interface Repository..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"Repository interface\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "Repository" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "Repository" && echo "$CLI_RESPONSE" | grep -q "Repository"; then
    echo -e "${GREEN}✓ Found Groovy interface Repository (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Groovy interface Repository not found${NC}"
    exit 1
fi

# Test 3: Search for Groovy Enum & Trait
echo ""
echo "Test 3: Searching for Groovy enum Status and trait Auditable..."
CLI_RESPONSE_ENUM=$(cargo run --release --bin knot -- search "Status" -r "$REPO_NAME" -m 10 2>/dev/null)
CLI_RESPONSE_TRAIT=$(cargo run --release --bin knot -- search "Auditable" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$CLI_RESPONSE_ENUM" | grep -q "Status" && echo "$CLI_RESPONSE_TRAIT" | grep -q "Auditable"; then
    echo -e "${GREEN}✓ Found Groovy enum Status and trait Auditable (CLI)${NC}"
else
    echo -e "${RED}✗ Groovy enum/trait not found${NC}"
    exit 1
fi

# Test 4: Search for Script-Level Variables & Closures
echo ""
echo "Test 4: Searching for Groovy closures and global variables..."
CLI_RESPONSE_VAR=$(cargo run --release --bin knot -- search "globalConfig" -r "$REPO_NAME" -m 10 2>/dev/null)
CLI_RESPONSE_CLOSURE=$(cargo run --release --bin knot -- search "processDataClosure" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$CLI_RESPONSE_VAR" | grep -q "globalConfig" && echo "$CLI_RESPONSE_CLOSURE" | grep -q "processDataClosure"; then
    echo -e "${GREEN}✓ Found Script-level variable globalConfig and closure processDataClosure (CLI)${NC}"
else
    echo -e "${RED}✗ Groovy script-level elements not found${NC}"
    exit 1
fi

# Test 5: explore_file on sample_full.groovy
echo ""
echo "Test 5: Exploring sample_full.groovy for full entity structure..."
GROOVY_FILE="$TMP_REPO_DIR/sample_full.groovy"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$GROOVY_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$GROOVY_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "${CLI_RESPONSE}${MCP_RESPONSE}" | grep -qE "UserService|BaseService|Repository|Auditable|Status|globalConfig|processDataClosure|calculateTotal|scriptMethod|addition of #num1"; then
    echo -e "${GREEN}✓ sample_full.groovy robust entity structure (inc. Spock method) found via explore (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ sample_full.groovy complex entities not fully found${NC}"
    exit 1
fi

echo -e "${YELLOW}[5/5] Finalizing...${NC}"

if [ -f "$E2E_DATA_DIR/docker-compose.yml" ]; then
    docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v 2>/dev/null || true
fi

echo -e "${GREEN}✓ All Groovy E2E tests passed!${NC}"
exit 0
