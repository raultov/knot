#!/usr/bin/env bash
# E2E regression test: Groovy -> Groovy cross-class reference resolution
# Reproduces bug: ad-hoc parser doesn't extract reference intents from
# method bodies, so inter-class Groovy calls are not stored as CALLS.
#
# Usage: ./tests/run_groovy_cross_ref_e2e.sh
set -eu

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_groovy_xref_data"
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_groovy_xref_repo"

NEO4J_PORT=17988; NEO4J_HTTP_PORT=17989
NEO4J_URI="bolt://localhost:${NEO4J_PORT}"
NEO4J_USER="neo4j"; NEO4J_PASSWORD="e2e_test_password"
QDRANT_PORT=16535; QDRANT_GRPC_PORT=16536
QDRANT_URL="http://localhost:${QDRANT_PORT}"
QDRANT_COLLECTION="knot_groovy_xref_e2e"
REPO_NAME="groovy_xref_e2e"

cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Groovy cross-ref E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up:${NC}"
        echo "  docker compose -f $E2E_DATA_DIR/docker-compose.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}
trap cleanup EXIT INT TERM

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Groovy Cross-Ref E2E Regression Test${NC}"
echo -e "${BLUE}(Groovy -> Groovy CALLS relationship)${NC}"
echo -e "${BLUE}========================================${NC}"

# 1. Docker
echo -e "\n${YELLOW}[1/3] Starting Docker...${NC}"
mkdir -p "$E2E_DATA_DIR"/{neo4j/data,neo4j/logs,qdrant/storage}

cat > "$E2E_DATA_DIR/docker-compose.yml" <<YML
services:
  neo4j:
    image: neo4j:5.26-community
    container_name: knot_neo4j_grv_xref_e2e
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
    healthcheck:
      test: ["CMD","cypher-shell","-u","${NEO4J_USER}","-p","${NEO4J_PASSWORD}","CALL db.ping()"]
      interval: 5s
      timeout: 5s
      retries: 10
  qdrant:
    image: qdrant/qdrant:v1.13.5
    container_name: knot_qdrant_grv_xref_e2e
    ports:
      - "${QDRANT_PORT}:6334"
      - "${QDRANT_GRPC_PORT}:6335"
    volumes:
      - ${E2E_DATA_DIR}/qdrant/storage:/qdrant/storage
YML

docker compose -f "$E2E_DATA_DIR/docker-compose.yml" up -d

for svc in "Neo4j $NEO4J_PORT" "Qdrant $QDRANT_PORT"; do
    set -- $svc; name=$1; port=$2; elapsed=0
    echo -n "Waiting for $name on port $port"
    while [ $elapsed -lt 120 ]; do
        if nc -z localhost "$port" 2>/dev/null; then
            echo -e "\n${GREEN}✓ $name ready${NC}"; break
        fi
        sleep 2; elapsed=$((elapsed+2)); echo -n "."
    done
    [ $elapsed -lt 120 ] || { echo -e "${RED}$name timeout${NC}"; exit 1; }
done
sleep 5

# 2. Create repo: Service.groovy (callee) + Client.groovy (caller, uses `def`)
echo -e "\n${YELLOW}[2/3] Creating Groovy test repo...${NC}"
rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR/src/main/groovy/com/example"

# Callee: a simple class that tree-sitter CAN parse
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/Calculator.groovy" <<'GROOVY'
package com.example

class Calculator {
    int add(int a, int b) {
        return a + b
    }

    int multiply(int x, int y) {
        return x * y
    }
}
GROOVY

# Caller A: uses `def` methods -> tree-sitter FAILS, ad-hoc parser rescues entities
# but does NOT extract reference intents. This is the bug.
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/ClientA.groovy" <<'GROOVY'
package com.example

class ClientA {
    def compute(Calculator calc) {
        def sum = calc.add(5, 10)
        def product = calc.multiply(3, 7)
        return sum + product
    }
}
GROOVY

# Caller B: typed methods -> tree-sitter CAN parse this
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/ClientB.groovy" <<'GROOVY'
package com.example

class ClientB {
    int run(Calculator calc) {
        int result = calc.add(1, 2)
        return result
    }
}
GROOVY

# 3. Index
echo -e "\n${YELLOW}[3/3] Indexing and verifying...${NC}"
export KNOT_NEO4J_URI KNOT_NEO4J_USER KNOT_NEO4J_PASSWORD
export KNOT_QDRANT_URL KNOT_QDRANT_COLLECTION
export KNOT_REPO_PATH="$TMP_REPO_DIR"
export KNOT_REPO_NAME="$REPO_NAME"

cd "$PROJECT_ROOT"
cargo run --release --bin knot-indexer -- --clean 2>/dev/null

echo -e "${GREEN}✓ Indexed${NC}"

call_mcp() {
    echo "$1" | cargo run --release --bin knot-mcp 2>/dev/null | tail -1
}

FAILED=0

# Test A: find_callers for Calculator.add — should have callers from BOTH ClientA and ClientB
echo -e "\n${BLUE}Test A: find_callers for Calculator.add${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"add","repo_name":"'"$REPO_NAME"'"}}}')

echo "Response: $(echo "$MCP_RESP" | head -c 400)"

if echo "$MCP_RESP" | grep -qi "ClientB"; then
    echo -e "${GREEN}✓ ClientB (typed) found as caller of add${NC}"
else
    echo -e "${YELLOW}⚠ ClientB NOT found — tree-sitter may have failed on typed Groovy too${NC}"
fi

if echo "$MCP_RESP" | grep -qi "ClientA"; then
    echo -e "${GREEN}✓ ClientA (def) found as caller of add — cross-ref works!${NC}"
else
    echo -e "${RED}✗ BUG CONFIRMED: ClientA (def) NOT found as caller of add${NC}"
    echo -e "${RED}  Ad-hoc parser extracts entities but NOT reference intents from method bodies${NC}"
    FAILED=1
fi

# Test B: find_callers for Calculator.multiply
echo -e "\n${BLUE}Test B: find_callers for Calculator.multiply${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"multiply","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qi "ClientA"; then
    echo -e "${GREEN}✓ ClientA found as caller of multiply${NC}"
else
    echo -e "${RED}✗ BUG: ClientA NOT found as caller of multiply${NC}"
    FAILED=1
fi

# Test C: explore_file on ClientA.groovy to confirm entities were extracted
echo -e "\n${BLUE}Test C: explore_file on ClientA.groovy (def-based)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMP_REPO_DIR"'/src/main/groovy/com/example/ClientA.groovy","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -q "compute"; then
    echo -e "${GREEN}✓ ClientA entities extracted by ad-hoc parser${NC}"
else
    echo -e "${RED}✗ ClientA entities NOT found — ad-hoc parser failed${NC}"
    FAILED=1
fi

# Cleanup
docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v 2>/dev/null || true

if [ "$FAILED" -eq 0 ]; then
    echo -e "\n${GREEN}✓ All Groovy cross-ref E2E tests passed!${NC}"
    exit 0
else
    echo -e "\n${RED}✗ Groovy cross-ref E2E tests FAILED — ad-hoc parser needs reference extraction${NC}"
    exit 1
fi
