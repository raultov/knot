#!/usr/bin/env bash
# E2E regression test: Cross-language (Java -> Groovy) reference resolution
# Reproduces bug where Java code calling a Groovy static method 
# does not get stored as a CALLS relationship in Neo4j.
#
# Usage: ./tests/run_cross_lang_ref_e2e.sh
set -eu

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_crosslang_data"
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_crosslang_repo"

NEO4J_PORT=17888; NEO4J_HTTP_PORT=17974
NEO4J_URI="bolt://localhost:${NEO4J_PORT}"
NEO4J_USER="neo4j"; NEO4J_PASSWORD="e2e_test_password"
QDRANT_PORT=16435; QDRANT_GRPC_PORT=16436
QDRANT_URL="http://localhost:${QDRANT_PORT}"
QDRANT_COLLECTION="knot_crosslang_e2e"
REPO_NAME="cross_lang_e2e"

cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Cross-lang reference E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi
    echo -e "\n${YELLOW}Cleaning up cross-lang reference E2E environment...${NC}"
    docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}
trap cleanup EXIT INT TERM

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Cross-Lang Reference E2E Regression Test${NC}"
echo -e "${BLUE}(Java -> Groovy CALLS relationship)${NC}"
echo -e "${BLUE}========================================${NC}"

# 1. Set up Docker
echo -e "\n${YELLOW}[1/4] Starting Docker containers...${NC}"
mkdir -p "$E2E_DATA_DIR"/{neo4j/data,neo4j/logs,qdrant/storage}

cat > "$E2E_DATA_DIR/docker-compose.yml" <<EOF
services:
  neo4j:
    image: neo4j:5.26-community
    container_name: knot_neo4j_crosslang_e2e
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
    container_name: knot_qdrant_crosslang_e2e
    ports:
      - "${QDRANT_PORT}:6334"
      - "${QDRANT_GRPC_PORT}:6335"
    volumes:
      - ${E2E_DATA_DIR}/qdrant/storage:/qdrant/storage
EOF

docker compose -f "$E2E_DATA_DIR/docker-compose.yml" up -d

# Wait for services
for svc in "Neo4j $NEO4J_PORT" "Qdrant $QDRANT_PORT"; do
    set -- $svc; name=$1; port=$2
    elapsed=0
    echo -n "Waiting for $name on port $port"
    while [ $elapsed -lt 120 ]; do
        if nc -z localhost "$port" 2>/dev/null; then
            echo -e "\n${GREEN}✓ $name is ready${NC}"; break
        fi
        sleep 2; elapsed=$((elapsed+2)); echo -n "."
    done
    [ $elapsed -lt 120 ] || { echo -e "${RED}$name timeout${NC}"; exit 1; }
done
sleep 5

# 2. Create the test repo with a Groovy class and a Java caller
echo -e "\n${YELLOW}[2/4] Creating test repo (Groovy class + Java caller)...${NC}"
rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR/src/main/groovy/com/example/utils" \
         "$TMP_REPO_DIR/src/main/java/com/example/app"

# Groovy class with a static method
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/utils/Helper.groovy" <<'GROOVY'
package com.example.utils

class Helper {
    static String greet(String name) {
        return "Hello, ${name}!"
    }

    static int add(int a, int b) {
        return a + b
    }
}
GROOVY

# Java class that calls the Groovy static method
cat > "$TMP_REPO_DIR/src/main/java/com/example/app/Main.java" <<'JAVA'
package com.example.app;

import com.example.utils.Helper;

public class Main {
    public static void main(String[] args) {
        String msg = Helper.greet("World");
        System.out.println(msg);
        
        int result = Helper.add(3, 4);
        System.out.println(result);
    }
}
JAVA

# Groovy class with a regular (non-static) method
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/utils/Parser.groovy" <<'GROOVY'
package com.example.utils

class Parser {
    String parse(String input) {
        return input.trim().toUpperCase()
    }
}
GROOVY

# Java class calling the Groovy instance method
cat > "$TMP_REPO_DIR/src/main/java/com/example/app/Consumer.java" <<'JAVA'
package com.example.app;

import com.example.utils.Parser;

public class Consumer {
    void process(String data) {
        Parser p = new Parser();
        String result = p.parse(data);
        System.out.println(result);
    }
}
JAVA

# 3. Index
echo -e "\n${YELLOW}[3/4] Indexing test repo...${NC}"
export KNOT_NEO4J_URI KNOT_NEO4J_USER KNOT_NEO4J_PASSWORD
export KNOT_QDRANT_URL KNOT_QDRANT_COLLECTION
export KNOT_REPO_PATH="$TMP_REPO_DIR"
export KNOT_REPO_NAME="$REPO_NAME"

cd "$PROJECT_ROOT"
cargo run --release --bin knot-indexer -- --clean 2>/dev/null

echo -e "${GREEN}✓ Indexed${NC}"

# 4. Verify with knot-mcp find_callers
echo -e "\n${YELLOW}[4/4] Verifying cross-language references via find_callers...${NC}"

call_mcp() {
    local json_req="$1"
    echo "$json_req" | cargo run --release --bin knot-mcp 2>/dev/null | tail -1
}

# 4a: Should find callers of greet() from Main.java
echo -e "\n${BLUE}Test A: find_callers for Helper.greet (static method called from Java)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"greet","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "Main|callers|CALLS"; then
    echo -e "${GREEN}✓ PASS: Helper.greet has callers${NC}"
else
    echo -e "${RED}✗ FAIL: Helper.greet has NO callers — cross-lang reference NOT stored${NC}"
    echo "Response: $MCP_RESP" | head -c 300
    FAILED=1
fi

# 4b: Should find callers of add() from Main.java
echo -e "\n${BLUE}Test B: find_callers for Helper.add (static method called from Java)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"add","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "Main|callers|CALLS"; then
    echo -e "${GREEN}✓ PASS: Helper.add has callers${NC}"
else
    echo -e "${RED}✗ FAIL: Helper.add has NO callers — cross-lang reference NOT stored${NC}"
    echo "Response: $MCP_RESP" | head -c 300
    FAILED=1
fi

# 4c: Should find callers of parse() 
echo -e "\n${BLUE}Test C: find_callers for Parser.parse (instance method from Java)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"parse","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "Consumer|callers|CALLS"; then
    echo -e "${GREEN}✓ PASS: Parser.parse has callers${NC}"
else
    echo -e "${RED}✗ FAIL: Parser.parse has NO callers — cross-lang reference NOT stored${NC}"
    echo "Response: $MCP_RESP" | head -c 300
    FAILED=1
fi

# 4d: Search for the Groovy class via semantic search (should always work)
echo -e "\n${BLUE}Test D: search_hybrid_context for Helper (semantic, should always work)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Helper Groovy class greet","max_results":5,"repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "Helper|greet"; then
    echo -e "${GREEN}✓ PASS: Helper found via semantic search${NC}"
else
    echo -e "${RED}✗ FAIL: Helper NOT found via semantic search — indexing may have failed${NC}"
    FAILED=1
fi

# 4e: explore_file on Helper.groovy 
echo -e "\n${BLUE}Test E: explore_file on Helper.groovy${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMP_REPO_DIR"'/src/main/groovy/com/example/utils/Helper.groovy","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -q "Helper"; then
    echo -e "${GREEN}✓ PASS: Helper.groovy entities found${NC}"
else
    echo -e "${RED}✗ FAIL: Helper.groovy NOT explored correctly${NC}"
    FAILED=1
fi

# 4f: explore_file on Main.java
echo -e "\n${BLUE}Test F: explore_file on Main.java${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMP_REPO_DIR"'/src/main/java/com/example/app/Main.java","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -q "Main"; then
    echo -e "${GREEN}✓ PASS: Main.java entities found${NC}"
else
    echo -e "${RED}✗ FAIL: Main.java NOT explored correctly${NC}"
    FAILED=1
fi

# Final
docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v 2>/dev/null || true

if [ "${FAILED:-0}" -eq 0 ]; then
    echo -e "\n${GREEN}✓ All cross-lang reference E2E tests passed!${NC}"
    exit 0
else
    echo -e "\n${RED}✗ Cross-lang reference E2E tests FAILED${NC}"
    exit 1
fi
