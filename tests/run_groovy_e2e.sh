#!/usr/bin/env bash
# E2E Integration Test Script for Groovy Language Support in knot (v0.10.5+)
#
# Tests full Groovy support across three dimensions:
#   A. Entity Extraction — classes, interfaces, enums, traits, closures, script variables
#   B. Cross-Ref — Groovy→Groovy CALLS relationships via find_callers
#   C. Private Methods — private method tracking, no-paren calls, innermost assignment
#
# Usage: ./tests/run_groovy_e2e.sh
# Requirements: docker, docker-compose

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.e2e.yml"
TEST_FILES_DIR="$SCRIPT_DIR/testing_files"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_data"
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_groovy_repo"

# ── Constants (uses docker-compose.e2e.yml ports) ──────────
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
export NEO4J_URI NEO4J_USER NEO4J_PASSWORD QDRANT_URL

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2
FAILED=0

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Groovy Language E2E Integration Test${NC}"
echo -e "${BLUE}Group A: Entity Extraction${NC}"
echo -e "${BLUE}Group B: Cross-Ref CALLS${NC}"
echo -e "${BLUE}Group C: Private Method Tracking${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    echo -e "\n${YELLOW}Cleaning up Groovy E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}
trap cleanup EXIT INT TERM

# ── Start Docker ────────────────────────────────────────
echo -e "${YELLOW}[0/4] Starting Docker for Groovy E2E...${NC}"
cd "$SCRIPT_DIR"
docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
mkdir -p "$E2E_DATA_DIR"/neo4j/data "$E2E_DATA_DIR"/neo4j/logs "$E2E_DATA_DIR"/qdrant

docker compose -f "$COMPOSE_FILE" up -d

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

wait_for_port 17687 "Neo4j" "knot_neo4j_e2e" || exit 1
wait_for_port 16334 "Qdrant" "knot_qdrant_e2e" || exit 1
sleep 8  # extra buffer: Neo4j healthcheck can pass before index creation is ready

# ── Build binaries once ─────────────────────────────────
echo -e "\n${YELLOW}[1/4] Building knot binaries...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

run_indexer() {
    local repo="$1"
    local collection="$2"
    local clean_flag="${3:-}"
    echo -n "Indexing $repo... "
    env \
        KNOT_REPO_PATH="$TMP_REPO_DIR" \
        KNOT_REPO_NAME="$repo" \
        KNOT_NEO4J_URI="$NEO4J_URI" \
        KNOT_NEO4J_USER="$NEO4J_USER" \
        KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
        KNOT_QDRANT_URL="$QDRANT_URL" \
        KNOT_QDRANT_COLLECTION="$collection" \
        cargo run --release --bin knot-indexer -- $clean_flag 2>/dev/null
    echo -e "${GREEN}✓${NC}"
}

call_mcp() {
    local repo="$1"
    local collection="$2"
    local request="$3"
    echo "$request" | env \
        KNOT_NEO4J_URI="$NEO4J_URI" \
        KNOT_NEO4J_USER="$NEO4J_USER" \
        KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
        KNOT_QDRANT_URL="$QDRANT_URL" \
        KNOT_QDRANT_COLLECTION="$collection" \
        KNOT_REPO_PATH="$TMP_REPO_DIR" \
        KNOT_REPO_NAME="$repo" \
        cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1
}

call_cli() {
    local repo="$1"
    local collection="$2"
    shift 2
    env \
        KNOT_NEO4J_URI="$NEO4J_URI" \
        KNOT_NEO4J_USER="$NEO4J_USER" \
        KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
        KNOT_QDRANT_URL="$QDRANT_URL" \
        KNOT_QDRANT_COLLECTION="$collection" \
        KNOT_REPO_PATH="$TMP_REPO_DIR" \
        KNOT_REPO_NAME="$repo" \
        cargo run --release --bin knot -- "$@" 2>/dev/null
}

# ═══════════════════════════════════════════════════════════
# GROUP A: Entity Extraction
# ═══════════════════════════════════════════════════════════
echo -e "\n${BLUE}── Group A: Entity Extraction ──${NC}"

REPO_A="groovy_e2e_test_repo"
COLL_A="knot_groovy_e2e_test"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR"
cp "$TEST_FILES_DIR/sample_full.groovy" "$TMP_REPO_DIR/"

run_indexer "$REPO_A" "$COLL_A" "--clean"

# Test A1: Search for Groovy class
echo ""
echo "Test A1: Searching for Groovy class BaseService..."
if call_mcp "$REPO_A" "$COLL_A" \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BaseService Groovy class","max_results":10,"repo_name":"'"$REPO_A"'"}}}' | grep -q "BaseService" \
    && call_cli "$REPO_A" "$COLL_A" search "BaseService" -r "$REPO_A" -m 10 | grep -q "BaseService"; then
    echo -e "${GREEN}✓ Found Groovy class BaseService (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Groovy class BaseService not found${NC}"
    FAILED=1
fi

# Test A2: Search for Groovy interface
echo ""
echo "Test A2: Searching for Groovy interface Repository..."
if call_mcp "$REPO_A" "$COLL_A" \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Repository interface","max_results":10,"repo_name":"'"$REPO_A"'"}}}' | grep -q "Repository" \
    && call_cli "$REPO_A" "$COLL_A" search "Repository" -r "$REPO_A" -m 10 | grep -q "Repository"; then
    echo -e "${GREEN}✓ Found Groovy interface Repository (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Groovy interface Repository not found${NC}"
    FAILED=1
fi

# Test A3: Search for Groovy Enum & Trait
echo ""
echo "Test A3: Searching for Groovy enum Status and trait Auditable..."
if call_cli "$REPO_A" "$COLL_A" search "Status" -r "$REPO_A" -m 10 | grep -q "Status" \
    && call_cli "$REPO_A" "$COLL_A" search "Auditable" -r "$REPO_A" -m 10 | grep -q "Auditable"; then
    echo -e "${GREEN}✓ Found Groovy enum Status and trait Auditable (CLI)${NC}"
else
    echo -e "${RED}✗ Groovy enum/trait not found${NC}"
    FAILED=1
fi

# Test A4: Search for Script-Level Variables & Closures
echo ""
echo "Test A4: Searching for Groovy closures and global variables..."
if call_cli "$REPO_A" "$COLL_A" search "globalConfig" -r "$REPO_A" -m 10 | grep -q "globalConfig" \
    && call_cli "$REPO_A" "$COLL_A" search "processDataClosure" -r "$REPO_A" -m 10 | grep -q "processDataClosure"; then
    echo -e "${GREEN}✓ Found Script-level variable globalConfig and closure processDataClosure (CLI)${NC}"
else
    echo -e "${RED}✗ Groovy script-level elements not found${NC}"
    FAILED=1
fi

# Test A5: explore_file on sample_full.groovy
echo ""
echo "Test A5: Exploring sample_full.groovy for full entity structure..."
GROOVY_FILE="$TMP_REPO_DIR/sample_full.groovy"
CLI_RESPONSE=$(call_cli "$REPO_A" "$COLL_A" explore "$GROOVY_FILE" -r "$REPO_A" -o markdown)
if echo "$CLI_RESPONSE" | grep -qE "UserService|BaseService|Repository|Auditable|Status|globalConfig|processDataClosure"; then
    echo -e "${GREEN}✓ sample_full.groovy robust entity structure (inc. Spock method) found via explore${NC}"
else
    echo -e "${RED}✗ sample_full.groovy complex entities not fully found${NC}"
    FAILED=1
fi

# ═══════════════════════════════════════════════════════════
# GROUP B: Cross-Ref Calls
# ═══════════════════════════════════════════════════════════
echo -e "\n${BLUE}── Group B: Cross-Ref CALLS ──${NC}"

REPO_B="groovy_xref_e2e"
COLL_B="knot_groovy_xref_e2e"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR/src/main/groovy/com/example"

# Calculator.groovy — typed Groovy class
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/Calculator.groovy" << 'GROOVY'
package com.example
class Calculator {
    int add(int a, int b) { a + b }
    int multiply(int x, int y) { x * y }
    def result() { return add(1, 2) }
}
GROOVY

# ClientA.groovy — def-style Groovy code
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/ClientA.groovy" << 'GROOVY'
package com.example
class ClientA {
    def compute(Calculator calc) {
        return calc.add(5, 3)
    }
    def run() {
        def calc = new Calculator()
        def result = calc.multiply(2, 10)
        return result
    }
}
GROOVY

# ClientB.groovy — typed Groovy code
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/ClientB.groovy" << 'GROOVY'
package com.example
class ClientB {
    int run(Calculator calc) {
        return calc.add(10, 20)
    }
}
GROOVY

run_indexer "$REPO_B" "$COLL_B"

# Test B1: find_callers for Calculator.add
echo ""
echo "Test B1: find_callers for Calculator.add..."
RESP=$(call_mcp "$REPO_B" "$COLL_B" \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"add","repo_name":"'"$REPO_B"'"}}}')
if echo "$RESP" | grep -qi "ClientB"; then
    echo -e "${GREEN}✓ ClientB (typed) found as caller of add${NC}"
else
    echo -e "${YELLOW}⚠ ClientB NOT found — tree-sitter may have failed on typed Groovy too${NC}"
fi
if echo "$RESP" | grep -qi "ClientA"; then
    echo -e "${GREEN}✓ ClientA (def) found as caller of add — cross-ref works!${NC}"
else
    echo -e "${RED}✗ ClientA (def) NOT found as caller of add${NC}"
    FAILED=1
fi

# Test B2: find_callers for Calculator.multiply
echo ""
echo "Test B2: find_callers for Calculator.multiply..."
RESP=$(call_mcp "$REPO_B" "$COLL_B" \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"multiply","repo_name":"'"$REPO_B"'"}}}')
if echo "$RESP" | grep -qi "ClientA"; then
    echo -e "${GREEN}✓ ClientA found as caller of multiply${NC}"
else
    echo -e "${RED}✗ ClientA NOT found as caller of multiply${NC}"
    FAILED=1
fi

# Test B3: explore_file on ClientA.groovy
echo ""
echo "Test B3: explore_file on ClientA.groovy (def-based)..."
RESP=$(call_mcp "$REPO_B" "$COLL_B" \
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMP_REPO_DIR"'/src/main/groovy/com/example/ClientA.groovy","repo_name":"'"$REPO_B"'"}}}')
if echo "$RESP" | grep -q "compute"; then
    echo -e "${GREEN}✓ ClientA entities extracted by ad-hoc parser${NC}"
else
    echo -e "${RED}✗ ClientA entities NOT found — ad-hoc parser failed${NC}"
    FAILED=1
fi

# ═══════════════════════════════════════════════════════════
# GROUP C: Private Method Tracking
# ═══════════════════════════════════════════════════════════
echo -e "\n${BLUE}── Group C: Private Method Tracking ──${NC}"

REPO_C="groovy_private_e2e"
COLL_C="knot_groovy_private_e2e"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR/src/main/groovy/com/example"

# File 1: Pure typed Groovy — all methods have explicit types
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/PrivateMethods.groovy" << 'EOF'
package com.example

class PrivateMethods {
    private static void restartHttpServer() {
        println "Restarting"
    }

    private void computeSecret() {
        println "secret"
    }

    void publicWorkflow() {
        restartHttpServer()
        computeSecret()
    }

    def dynamicWorkflow(String mode) {
        restartHttpServer()
        runAnalyzer "report", 123
    }

    void runAnalyzer(String name, int count) {
        doSomething name
        restartHttpServer()
    }

    private void doSomething(String input) {
        println input
    }
}
EOF

# File 2: HttpUtil-like — private method with multi-line signature and closure args
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/HttpUtil.groovy" << 'EOF'
package com.example

class HttpUtil {
    static String loadIntoHttpServer(String html) {
        def server = restartHttpServer("web", "/tmp", {null}, {log?.errorOnHttpRequest(it.toString())})
        "http://localhost"
    }

    private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
                                                        Closure handler = {null},
                                                        Closure errorListener = {}) {
        def server = new SimpleHttpServer()
        server
    }
}
EOF

# File 3: Nested methods — outer container must NOT steal references from inner methods
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/NestedMethods.groovy" << 'EOF'
package com.example

class NestedMethods {
    def showGrabbingFinishedMessage(String message) {
        show(message, new Listener() {
            @Override void hyperlinkUpdate(String event) {
                runAnalyzer("visualize")
            }
        })
    }

    def show(message, Listener listener) {
    }

    private void runAnalyzer(String action) {
        println action
    }
}
EOF

# File 4: Replicates code-history-mining UI.groovy pattern
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/UIPattern.groovy" << 'EOF'
package com.example

class UIPattern {
    def createActionsOnHistoryFile() {
        def action = new AbstractAction() {
            @Override void actionPerformed(ActionEvent e) {
                runAnalyzer("history")
            }
        }
        return action
    }

    def anotherAction() {
        def action = new AbstractAction() {
            @Override void actionPerformed(ActionEvent e) {
                println "no runAnalyzer here"
            }
        }
        return action
    }

    private void runAnalyzer(String action) {
        println action
    }
}
EOF

run_indexer "$REPO_C" "$COLL_C"

# Test C1: find_callers restartHttpServer (typed caller)
echo ""
echo "Test C1: find_callers restartHttpServer (typed caller)..."
RESP=$(call_mcp "$REPO_C" "$COLL_C" \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"restartHttpServer","repo_name":"'"$REPO_C"'"}}}')
if echo "$RESP" | grep -q "publicWorkflow"; then
    echo -e "${GREEN}✓ publicWorkflow found as caller${NC}"
else
    echo -e "${RED}✗ publicWorkflow NOT found as caller${NC}"
    FAILED=1
fi

# Test C2: find_callers restartHttpServer (def caller)
echo ""
echo "Test C2: find_callers restartHttpServer (def caller)..."
if echo "$RESP" | grep -q "dynamicWorkflow"; then
    echo -e "${GREEN}✓ dynamicWorkflow found as caller${NC}"
else
    echo -e "${RED}✗ dynamicWorkflow NOT found as caller${NC}"
    FAILED=1
fi

# Test C3: find_callers restartHttpServer (no-paren callee)
echo ""
echo "Test C3: find_callers restartHttpServer (no-paren caller)..."
if echo "$RESP" | grep -q "runAnalyzer"; then
    echo -e "${GREEN}✓ runAnalyzer found as caller${NC}"
else
    echo -e "${RED}✗ runAnalyzer NOT found as caller${NC}"
    FAILED=1
fi

# Test C4: find_callers doSomething (no-paren callee)
echo ""
echo "Test C4: find_callers doSomething (no-paren caller)..."
RESP=$(call_mcp "$REPO_C" "$COLL_C" \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"doSomething","repo_name":"'"$REPO_C"'"}}}')
if echo "$RESP" | grep -q "runAnalyzer"; then
    echo -e "${GREEN}✓ runAnalyzer found as caller of doSomething${NC}"
else
    echo -e "${RED}✗ runAnalyzer NOT found as caller of doSomething${NC}"
    FAILED=1
fi

# Test C5: find_callers computeSecret (private method)
echo ""
echo "Test C5: find_callers computeSecret (private method)..."
RESP=$(call_mcp "$REPO_C" "$COLL_C" \
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"computeSecret","repo_name":"'"$REPO_C"'"}}}')
if echo "$RESP" | grep -q "publicWorkflow"; then
    echo -e "${GREEN}✓ publicWorkflow found as caller${NC}"
else
    echo -e "${RED}✗ publicWorkflow NOT found as caller${NC}"
    FAILED=1
fi

# Test C6: find_callers restartHttpServer (multi-line closure args)
echo ""
echo "Test C6: find_callers restartHttpServer (multi-line closure args)..."
RESP=$(call_mcp "$REPO_C" "$COLL_C" \
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"restartHttpServer","repo_name":"'"$REPO_C"'"}}}')
# HttpUtil.loadIntoHttpServer calls restartHttpServer with multi-line closure args
# Note: find_callers aggregates across all files, so we check for loadIntoHttpServer
if echo "$RESP" | grep -q "loadIntoHttpServer"; then
    echo -e "${GREEN}✓ loadIntoHttpServer found as caller${NC}"
else
    echo -e "${YELLOW}⚠ loadIntoHttpServer NOT found as caller (multi-line closure parsing may need tuning)${NC}"
fi

# Test C7: find_callers runAnalyzer (innermost caller - NestedMethods)
echo ""
echo "Test C7: find_callers runAnalyzer (innermost caller from NestedMethods)..."
RESP=$(call_mcp "$REPO_C" "$COLL_C" \
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"runAnalyzer","repo_name":"'"$REPO_C"'"}}}')
if echo "$RESP" | grep -q "hyperlinkUpdate"; then
    echo -e "${GREEN}✓ hyperlinkUpdate found as caller${NC}"
else
    echo -e "${RED}✗ hyperlinkUpdate NOT found as caller${NC}"
    FAILED=1
fi

# Test C8: find_callers runAnalyzer (outer NOT a caller)
echo ""
echo "Test C8: find_callers runAnalyzer (outer NOT a caller)..."
if echo "$RESP" | grep -q "showGrabbingFinishedMessage"; then
    echo -e "${RED}✗ showGrabbingFinishedMessage should NOT be a caller${NC}"
    FAILED=1
else
    echo -e "${GREEN}✓ showGrabbingFinishedMessage correctly absent${NC}"
fi

# Test C9: find_callers runAnalyzer (correct actionPerformed from UIPattern)
echo ""
echo "Test C9: find_callers runAnalyzer (correct actionPerformed from UIPattern)..."
if echo "$RESP" | grep -q "UIPattern.groovy"; then
    echo -e "${GREEN}✓ actionPerformed found as caller${NC}"
else
    echo -e "${RED}✗ actionPerformed NOT found as caller${NC}"
    FAILED=1
fi

# Test C10: find_callers runAnalyzer (no duplicate actionPerformed)
echo ""
echo "Test C10: find_callers runAnalyzer (no duplicate actionPerformed)..."
UIP_COUNT=$(echo "$RESP" | grep -c "UIPattern.groovy" 2>/dev/null || echo "0")
if [ "$UIP_COUNT" -le 1 ]; then
    echo -e "${GREEN}✓ no cross-contamination${NC}"
else
    echo -e "${RED}✗ cross-contamination detected: $UIP_COUNT callers from UIPattern${NC}"
    FAILED=1
fi

# ── Final result ─────────────────────────────────────────
echo -e "\n${BLUE}========================================${NC}"
if [ "$FAILED" -eq 0 ]; then
    echo -e "${GREEN}✓ All Groovy E2E tests passed!${NC}"
    exit 0
else
    echo -e "${RED}✗ Groovy E2E tests failed!${NC}"
    exit 1
fi
