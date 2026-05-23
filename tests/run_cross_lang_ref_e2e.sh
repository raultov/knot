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
    fi
    docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Cross-Lang Reference E2E Regression Test${NC}"
echo -e "${BLUE}(Java/Kotlin/Groovy CALLS relationships)${NC}"
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

# Wait for services using Docker health checks for Neo4j
echo -n "Waiting for Neo4j (health check)"
elapsed=0
while [ $elapsed -lt 120 ]; do
    status=$(docker inspect --format='{{.State.Health.Status}}' knot_neo4j_crosslang_e2e 2>/dev/null || echo "starting")
    if [ "$status" = "healthy" ]; then
        echo -e "\n${GREEN}✓ Neo4j is ready (healthy)${NC}"; break
    fi
    sleep 3; elapsed=$((elapsed+3)); echo -n "."
done
[ $elapsed -lt 120 ] || { echo -e "\n${RED}Neo4j timeout${NC}"; exit 1; }

echo -n "Waiting for Qdrant on port $QDRANT_PORT"
elapsed=0
while [ $elapsed -lt 120 ]; do
    if nc -z localhost "$QDRANT_PORT" 2>/dev/null; then
        echo -e "\n${GREEN}✓ Qdrant is ready${NC}"; break
    fi
    sleep 2; elapsed=$((elapsed+2)); echo -n "."
done
[ $elapsed -lt 120 ] || { echo -e "\n${RED}Qdrant timeout${NC}"; exit 1; }
sleep 5

# 2. Create the test repo with Groovy, Java, and Kotlin classes
echo -e "\n${YELLOW}[2/4] Creating test repo (Groovy + Java + Kotlin classes)...${NC}"
rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR/src/main/groovy/com/example/utils" \
         "$TMP_REPO_DIR/src/main/java/com/example/app" \
         "$TMP_REPO_DIR/src/main/kotlin/com/example/kotlin"

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

# Kotlin class that calls a Groovy static method (Kotlin → Groovy)
cat > "$TMP_REPO_DIR/src/main/kotlin/com/example/kotlin/KotlinClient.kt" <<'KOTLIN'
package com.example.kotlin

import com.example.utils.Helper

class KotlinClient {
    fun callGroovyStatic(): String {
        return Helper.greet("Kotlin")
    }

    fun callGroovyAdd(): Int {
        return Helper.add(10, 20)
    }
}
KOTLIN

# Kotlin class that calls a Groovy instance method (Kotlin → Groovy)
cat > "$TMP_REPO_DIR/src/main/kotlin/com/example/kotlin/KotlinParser.kt" <<'KOTLIN'
package com.example.kotlin

import com.example.utils.Parser

class KotlinParser {
    fun parseGroovy(input: String): String {
        val p = Parser()
        return p.parse(input)
    }
}
KOTLIN

# Kotlin utility class called from Java (Java → Kotlin)
cat > "$TMP_REPO_DIR/src/main/kotlin/com/example/kotlin/StringUtils.kt" <<'KOTLIN'
package com.example.kotlin

object StringUtils {
    fun capitalize(text: String): String {
        return text.replaceFirstChar { it.uppercase() }
    }

    fun countWords(text: String): Int {
        return text.split(" ").size
    }
}
KOTLIN

# Java class that calls Kotlin utility (Java → Kotlin)
cat > "$TMP_REPO_DIR/src/main/java/com/example/app/JavaUser.java" <<'JAVA'
package com.example.app;

import com.example.kotlin.StringUtils;

public class JavaUser {
    public String formatName(String name) {
        return StringUtils.INSTANCE.capitalize(name);
    }
}
JAVA

# Groovy class that calls Kotlin (Groovy → Kotlin)
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/utils/GroovyUser.groovy" <<'GROOVY'
package com.example.utils

import com.example.kotlin.StringUtils

class GroovyUser {
    String analyze(String text) {
        def words = StringUtils.INSTANCE.countWords(text)
        return "Text has ${words} words"
    }
}
GROOVY

# 3. Index
echo -e "\n${YELLOW}[3/4] Indexing test repo...${NC}"
cd "$PROJECT_ROOT"
env \
    KNOT_NEO4J_URI="$NEO4J_URI" \
    KNOT_NEO4J_USER="$NEO4J_USER" \
    KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
    KNOT_QDRANT_URL="$QDRANT_URL" \
    KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
    KNOT_REPO_PATH="$TMP_REPO_DIR" \
    KNOT_REPO_NAME="$REPO_NAME" \
    cargo run --release --bin knot-indexer -- --clean 2>&1

IDX_EXIT=$?
if [ $IDX_EXIT -ne 0 ]; then
    echo -e "${RED}✗ Indexing failed (exit code: $IDX_EXIT)${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Indexed${NC}"

# 4. Verify with knot-mcp find_callers
echo -e "\n${YELLOW}[4/4] Verifying cross-language references via find_callers...${NC}"

call_mcp() {
    local json_req="$1"
    echo "$json_req" | env \
        KNOT_NEO4J_URI="$NEO4J_URI" \
        KNOT_NEO4J_USER="$NEO4J_USER" \
        KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
        KNOT_QDRANT_URL="$QDRANT_URL" \
        KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
        KNOT_REPO_PATH="$TMP_REPO_DIR" \
        KNOT_REPO_NAME="$REPO_NAME" \
        cargo run --release --bin knot-mcp 2>/dev/null | tail -1
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

# 4g: Kotlin → Groovy static method call
echo -e "\n${BLUE}Test G: find_callers for Helper.greet — should include KotlinClient (Kotlin → Groovy)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"greet","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "KotlinClient|callGroovyStatic"; then
    echo -e "${GREEN}✓ PASS: KotlinClient found as caller of Helper.greet${NC}"
else
    echo -e "${RED}✗ FAIL: KotlinClient NOT found as caller of Helper.greet${NC}"
    FAILED=1
fi

# 4h: Kotlin → Groovy instance method call
echo -e "\n${BLUE}Test H: find_callers for Parser.parse — should include KotlinParser (Kotlin → Groovy)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"parse","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "KotlinParser|parseGroovy"; then
    echo -e "${GREEN}✓ PASS: KotlinParser found as caller of Parser.parse${NC}"
else
    echo -e "${RED}✗ FAIL: KotlinParser NOT found as caller of Parser.parse${NC}"
    FAILED=1
fi

# 4i: Java → Kotlin object method call
echo -e "\n${BLUE}Test I: find_callers for StringUtils.capitalize — should include JavaUser (Java → Kotlin)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"capitalize","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "JavaUser|formatName"; then
    echo -e "${GREEN}✓ PASS: JavaUser found as caller of StringUtils.capitalize${NC}"
else
    echo -e "${RED}✗ FAIL: JavaUser NOT found as caller of StringUtils.capitalize${NC}"
    FAILED=1
fi

# 4j: Groovy → Kotlin object method call
echo -e "\n${BLUE}Test J: find_callers for StringUtils.countWords — should include GroovyUser (Groovy → Kotlin)${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"countWords","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -qE "GroovyUser|analyze"; then
    echo -e "${GREEN}✓ PASS: GroovyUser found as caller of StringUtils.countWords${NC}"
else
    echo -e "${RED}✗ FAIL: GroovyUser NOT found as caller of StringUtils.countWords${NC}"
    FAILED=1
fi

# 4k: explore_file on KotlinClient.kt
echo -e "\n${BLUE}Test K: explore_file on KotlinClient.kt${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMP_REPO_DIR"'/src/main/kotlin/com/example/kotlin/KotlinClient.kt","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -q "KotlinClient"; then
    echo -e "${GREEN}✓ PASS: KotlinClient.kt entities found${NC}"
else
    echo -e "${RED}✗ FAIL: KotlinClient.kt NOT explored correctly${NC}"
    FAILED=1
fi

# 4l: explore_file on StringUtils.kt
echo -e "\n${BLUE}Test L: explore_file on StringUtils.kt${NC}"
MCP_RESP=$(call_mcp '{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMP_REPO_DIR"'/src/main/kotlin/com/example/kotlin/StringUtils.kt","repo_name":"'"$REPO_NAME"'"}}}')

if echo "$MCP_RESP" | grep -q "StringUtils"; then
    echo -e "${GREEN}✓ PASS: StringUtils.kt entities found${NC}"
else
    echo -e "${RED}✗ FAIL: StringUtils.kt NOT explored correctly${NC}"
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
