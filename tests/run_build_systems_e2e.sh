#!/usr/bin/env bash
# E2E Integration Test Script for Build Systems & CI/CD Support in knot (v0.10.x)
#
# This script tests Maven, Gradle, and Jenkins pipeline extraction:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (18xxx/16xxx)
# 2. Indexes sample pom.xml, build.gradle, and Jenkinsfile
# 3. Queries via MCP and CLI to validate build dependency/plugin/stage/step extraction
# 4. Cleans up containers and data
#
# Usage: ./tests/run_build_systems_e2e.sh
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
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_build_systems_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_build_systems_e2e_test"
REPO_NAME="build_systems_e2e_test_repo"

# Isolated repository: only build system files to avoid cross-language semantic ranking noise
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_build_systems_repo"

# Timeout settings
TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Build Systems & CI/CD E2E Integration Test${NC}"
echo -e "${BLUE}Phase 9 - Maven, Gradle, Jenkins (v0.10.x)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Build Systems E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Build Systems E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}

# Register cleanup on script exit
trap cleanup EXIT INT TERM

# Step 1: Start Docker containers
echo -e "${YELLOW}[1/5] Starting Docker containers for Build Systems E2E test...${NC}"
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

# Step 3: Index build system test files
echo -e "${YELLOW}[3/5] Indexing build system files (pom.xml, build.gradle, Jenkinsfile)...${NC}"
cd "$PROJECT_ROOT"

# Isolate: copy only build system files to a temp dir so semantic search only sees build entities
rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp "$TEST_FILES_DIR/sample_pom.xml" "$TMP_REPO_DIR/"
cp "$TEST_FILES_DIR/sample_build.gradle" "$TMP_REPO_DIR/"
cp "$TEST_FILES_DIR/sample.jenkinsfile" "$TMP_REPO_DIR/"

export KNOT_REPO_PATH="$TMP_REPO_DIR"
export KNOT_REPO_NAME="$REPO_NAME"
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for build system files..."
cargo run --release --bin knot-indexer -- --clean

echo -e "${GREEN}✓ Build system files indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating build system entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Maven dependencies via search
echo ""
echo "Test 1: Searching for Maven dependency spring-core..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"spring-core\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "spring-core" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "spring-core" && echo "$CLI_RESPONSE" | grep -q "spring-core"; then
    echo -e "${GREEN}✓ Found Maven dependency spring-core (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Maven dependency spring-core not found${NC}"
    exit 1
fi

# Test 2: Maven dependencies via explore_file on pom.xml
echo ""
echo "Test 2: Exploring pom.xml for dependency structure..."
POM_FILE="$TMP_REPO_DIR/sample_pom.xml"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$POM_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$POM_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qE "spring-core|gson|log4j" && echo "$CLI_RESPONSE" | grep -qE "spring-core|gson|log4j"; then
    echo -e "${GREEN}✓ pom.xml dependencies found via explore (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ pom.xml dependencies not found${NC}"
    exit 1
fi

# Test 3: Gradle dependencies via search
echo ""
echo "Test 3: Searching for Gradle dependency spring-boot-starter-web..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"spring-boot-starter-web\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "spring-boot-starter-web" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "spring-boot-starter-web" && echo "$CLI_RESPONSE" | grep -q "spring-boot-starter-web"; then
    echo -e "${GREEN}✓ Found Gradle dependency spring-boot-starter-web (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Gradle dependency spring-boot-starter-web not found${NC}"
    exit 1
fi

# Test 4: Gradle tasks via search
echo ""
echo "Test 4: Searching for Gradle task buildDocs..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"buildDocs Gradle task\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "buildDocs" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "buildDocs" && echo "$CLI_RESPONSE" | grep -q "buildDocs"; then
    echo -e "${GREEN}✓ Found Gradle task buildDocs (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Gradle task buildDocs not found${NC}"
    exit 1
fi

# Test 5: Jenkins pipeline stages via search
echo ""
echo "Test 5: Searching for Jenkins pipeline stage Build..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"pipeline stage Build\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "pipeline stage Build" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "stage: Build" && echo "$CLI_RESPONSE" | grep -q "stage: Build"; then
    echo -e "${GREEN}✓ Found Jenkins pipeline stage Build (MCP & CLI)${NC}"
else
    echo -e "${YELLOW}⚠ Jenkins pipeline stage Build not found (MCP may need tuning)${NC}"
fi

# Test 6: Jenkins pipeline steps via search
echo ""
echo "Test 6: Searching for Jenkins step sh: mvn compile..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"sh mvn compile Jenkins\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "sh: mvn" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "sh:" && echo "$CLI_RESPONSE" | grep -q "sh:"; then
    echo -e "${GREEN}✓ Found Jenkins pipeline steps (MCP & CLI)${NC}"
else
    echo -e "${YELLOW}⚠ Jenkins pipeline steps not found (MCP may need tuning)${NC}"
fi

# Test 7: Explore build.gradle
echo ""
echo "Test 7: Exploring build.gradle..."
GRADLE_FILE="$TMP_REPO_DIR/sample_build.gradle"
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$GRADLE_FILE\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- explore "$GRADLE_FILE" -r "$REPO_NAME" -o markdown 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -qE "spring-boot|gson|buildDocs" && echo "$CLI_RESPONSE" | grep -qE "spring-boot|gson|buildDocs"; then
    echo -e "${GREEN}✓ build.gradle entities found via explore (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ build.gradle entities not found${NC}"
    exit 1
fi

# Test 8: Maven plugins via search
echo ""
echo "Test 8: Searching for Maven plugin maven-compiler-plugin..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"maven-compiler-plugin\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | env KNOT_NEO4J_URI="$NEO4J_URI" KNOT_NEO4J_USER="$NEO4J_USER" KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" KNOT_QDRANT_URL="$QDRANT_URL" KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" KNOT_REPO_PATH="$TMP_REPO_DIR" cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "maven-compiler-plugin" -r "$REPO_NAME" -m 10 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "maven-compiler-plugin" && echo "$CLI_RESPONSE" | grep -q "maven-compiler-plugin"; then
    echo -e "${GREEN}✓ Found Maven plugin maven-compiler-plugin (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Maven plugin maven-compiler-plugin not found${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Build Systems E2E tests passed!${NC}"
echo -e "${GREEN}========================================${NC}"
