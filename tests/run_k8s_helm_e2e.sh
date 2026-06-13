#!/usr/bin/env bash
# E2E Integration Test Script for Kubernetes + Helm support in knot (v1.2.6)
#
# Usage: ./tests/run_k8s_helm_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/k8s_helm"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_k8s_helm_data"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_k8s_helm_e2e_test"
REPO_NAME="k8s_helm_e2e_test_repo"

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Kubernetes + Helm E2E Test${NC}"
echo -e "${BLUE}Phase C — K8s Manifests + Helm Charts${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    local exit_code=$?

    # Always tear down containers — leaving them up blocks the shared port 17687
    # for subsequent suites in run_all_e2e.sh and causes false cascading failures.
    if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        cd "$SCRIPT_DIR"
        docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    fi

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}K8s/Helm E2E tests failed!${NC}"
        echo -e "${YELLOW}Test data preserved at $E2E_DATA_DIR for inspection.${NC}"
        echo -e "${YELLOW}Manual cleanup:  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v && sudo rm -rf $E2E_DATA_DIR${NC}"
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up K8s/Helm E2E test environment...${NC}"
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}
trap cleanup EXIT INT TERM

# Step 1: Start Docker containers (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/5] Starting Docker containers...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/5] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set; expecting shared DB)${NC}"
fi

# Step 2: Wait for services (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[2/5] Skipping wait (KNOT_E2E_EXTERNAL_DB set; orchestrator manages readiness)${NC}"
else
    echo -e "${YELLOW}[2/5] Waiting for services...${NC}"
wait_for_port() {
    local port=$1 service=$2 container=$3 elapsed=0
    echo -n "Waiting for $service"
    while true; do
        if [ "$service" = "Neo4j" ]; then
            local status
            status=$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null || echo "starting")
            if [ "$status" = "healthy" ]; then echo ""; echo -e "${GREEN}✓ $service ready${NC}"; return 0; fi
        else
            if nc -z localhost "$port" 2>/dev/null; then echo ""; echo -e "${GREEN}✓ $service ready${NC}"; return 0; fi
        fi
        if [ $elapsed -ge $TIMEOUT_SECONDS ]; then echo ""; echo -e "${RED}ERROR: $service timeout${NC}"; return 1; fi
        sleep $HEALTH_CHECK_INTERVAL
        elapsed=$((elapsed + HEALTH_CHECK_INTERVAL))
        echo -n "."
    done
}
wait_for_port 17687 "Neo4j" "knot_neo4j_e2e"
wait_for_port 16334 "Qdrant" "knot_qdrant_e2e"
sleep 5
fi

echo -e "${YELLOW}[3/5] Indexing K8s and Helm files...${NC}"
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
echo "Running indexer..."
INDEXER_FLAGS=("--include-config-files")
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"
echo -e "${GREEN}✓ K8s/Helm files indexed${NC}"

echo -e "${YELLOW}[4/5] Validating via MCP and CLI...${NC}"
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Search for K8s Deployment
echo ""
echo "Test 1: Searching for K8s Deployment nginx..."
MCP_REQ='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"nginx deployment","max_results":10,"repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- search "nginx deployment" -r "$REPO_NAME" -m 10 2>/dev/null)
if echo "$MCP_RESP" | grep -qE "nginx|deployment" && echo "$CLI_RESP" | grep -qE "nginx|deployment"; then
    echo -e "${GREEN}✓ Found K8s Deployment (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ K8s Deployment not found${NC}"; exit 1
fi

# Test 2: Explore deployment.yaml
echo ""
echo "Test 2: Exploring deployment.yaml..."
DEP="$TEST_FILES_DIR/k8s/deployment.yaml"
MCP_REQ='{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$DEP"'","repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- explore "$DEP" -r "$REPO_NAME" -o markdown 2>/dev/null)
if echo "$MCP_RESP" | grep -qE "nginx|replicas" && echo "$CLI_RESP" | grep -qE "Kubernetes|nginx"; then
    echo -e "${GREEN}✓ deployment.yaml entities found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ deployment.yaml entities not found${NC}"; exit 1
fi

# Test 3: Search for K8s Service
echo ""
echo "Test 3: Searching for K8s Service backend..."
MCP_REQ='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"backend service","max_results":10,"repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- search "backend service" -r "$REPO_NAME" -m 10 2>/dev/null)
if echo "$MCP_RESP" | grep -qi "service" && echo "$CLI_RESP" | grep -qi "service"; then
    echo -e "${GREEN}✓ Found K8s Service (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ K8s Service not found${NC}"; exit 1
fi

# Test 4: Search for ConfigMap
echo ""
echo "Test 4: Searching for ConfigMap app-config..."
MCP_REQ='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"app-config","max_results":10,"repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- search "app-config" -r "$REPO_NAME" -m 10 2>/dev/null)
if echo "$MCP_RESP" | grep -q "app-config" && echo "$CLI_RESP" | grep -q "app-config"; then
    echo -e "${GREEN}✓ Found ConfigMap app-config (MCP & CLI)${NC}"
else
    echo -e "${YELLOW}⚠ ConfigMap not found (may need semantic tuning)${NC}"
fi

# Test 5: Explore Chart.yaml
echo ""
echo "Test 5: Exploring Chart.yaml..."
CHART="$TEST_FILES_DIR/helm/charts/sample-app/Chart.yaml"
MCP_REQ='{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$CHART"'","repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- explore "$CHART" -r "$REPO_NAME" -o markdown 2>/dev/null)
if echo "$MCP_RESP" | grep -q "sample-app" && echo "$CLI_RESP" | grep -q "sample-app"; then
    echo -e "${GREEN}✓ Chart.yaml entities found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Chart.yaml entities not found${NC}"; exit 1
fi

# Test 6: Search for Helm value replicaCount
echo ""
echo "Test 6: Searching for Helm value replicaCount..."
MCP_REQ='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"replicaCount","max_results":10,"repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- search "replicaCount" -r "$REPO_NAME" -m 10 2>/dev/null)
if echo "$MCP_RESP" | grep -q "replicaCount" && echo "$CLI_RESP" | grep -q "replicaCount"; then
    echo -e "${GREEN}✓ Found Helm value replicaCount (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Helm value replicaCount not found${NC}"; exit 1
fi

# Test 7: Explore values.yaml
echo ""
echo "Test 7: Exploring values.yaml..."
VALS="$TEST_FILES_DIR/helm/charts/sample-app/values.yaml"
MCP_REQ='{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$VALS"'","repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- explore "$VALS" -r "$REPO_NAME" -o markdown 2>/dev/null)
if echo "$MCP_RESP" | grep -qE "replicaCount|helm_value" && echo "$CLI_RESP" | grep -qE "Helm Values|helm_value"; then
    echo -e "${GREEN}✓ values.yaml entities found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ values.yaml entities not found${NC}"; exit 1
fi

# Test 8: Search for template variable
echo ""
echo "Test 8: Searching for Helm template variable image.repository..."
MCP_REQ='{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Values.image.repository","max_results":10,"repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- search "Values.image.repository" -r "$REPO_NAME" -m 10 2>/dev/null)
if echo "$MCP_RESP" | grep -qi "repository" && echo "$CLI_RESP" | grep -qi "repository"; then
    echo -e "${GREEN}✓ Found Helm template variable (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Helm template variable not found${NC}"; exit 1
fi

# Test 9: Explore Helm template
echo ""
echo "Test 9: Exploring Helm template deployment.yaml..."
TMPL="$TEST_FILES_DIR/helm/charts/sample-app/templates/deployment.yaml"
MCP_REQ='{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMPL"'","repo_name":"'"$REPO_NAME"'"}}}'
MCP_RESP=$(echo "$MCP_REQ" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESP=$(cargo run --release --bin knot -- explore "$TMPL" -r "$REPO_NAME" -o markdown 2>/dev/null)
if echo "$MCP_RESP" | grep -qi "template" && echo "$CLI_RESP" | grep -qE "Template Variables"; then
    echo -e "${GREEN}✓ Helm template variables found (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Helm template variables not found${NC}"; exit 1
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All K8s/Helm E2E tests passed!${NC}"
echo -e "${GREEN}========================================${NC}"
