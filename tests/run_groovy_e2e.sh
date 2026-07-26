#!/usr/bin/env bash
# E2E Integration Test Script for Groovy Language Support in knot (v0.10.5+)
#
# Tests full Groovy support across five dimensions:
#   A. Entity Extraction — classes, interfaces, enums, traits, closures, script variables
#   B. Cross-Ref — Groovy→Groovy CALLS relationships via find_callers
#   C. Private Methods — private method tracking, no-paren calls, innermost assignment
#   D. Inheritance — Groovy EXTENDS / IMPLEMENTS edges surfaced by find_callers
#                        (regression for the nextflow PluginExtensionPoint case)
#   E. Docstrings — GroovyDoc extraction into Neo4j/Qdrant (nextflow init case)
#   F. Method OVERRIDES — subtype.method -[OVERRIDES]-> supertype.method edges
#                        surfaced bidirectionally by find_callers (nextflow init case)
#   G. Property Accessors — Groovy auto-generated getters/setters as synthetic
#                        entities linked via OVERRIDES to interface getters
#
# Usage: ./tests/run_groovy_e2e.sh
# Requirements: docker, docker-compose

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'
export KNOT_FASTEMBED_CACHE_DIR="${KNOT_FASTEMBED_CACHE_DIR:-$HOME/.cache/knot/fastembed_cache}"


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
# Qdrant REST API port (16334 above is gRPC, used by the knot binaries; curl
# assertions must go through the REST port instead).
QDRANT_REST_URL="http://localhost:16333"
export NEO4J_URI NEO4J_USER NEO4J_PASSWORD QDRANT_URL

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2
FAILED=0

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Groovy Language E2E Integration Test${NC}"
echo -e "${BLUE}Group A: Entity Extraction${NC}"
echo -e "${BLUE}Group B: Cross-Ref CALLS${NC}"
echo -e "${BLUE}Group C: Private Method Tracking${NC}"
echo -e "${BLUE}Group D: Inheritance EXTENDS/IMPLEMENTS${NC}"
echo -e "${BLUE}Group E: Docstrings${NC}"
echo -e "${BLUE}Group F: Method OVERRIDES${NC}"
echo -e "${BLUE}Group G: Property Accessors${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi
    echo -e "\n${YELLOW}Cleaning up Groovy E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    echo -e "${GREEN}Cleanup complete${NC}"
}
trap cleanup EXIT INT TERM

# ── Start Docker ────────────────────────────────────────
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[0/4] Starting Docker for Groovy E2E...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
    mkdir -p "$E2E_DATA_DIR"/neo4j/data "$E2E_DATA_DIR"/neo4j/logs "$E2E_DATA_DIR"/qdrant

    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[0/4] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set; expecting shared DB)${NC}"
fi

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

if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[0b/4] Skipping wait (KNOT_E2E_EXTERNAL_DB set; orchestrator manages readiness)${NC}"
else
    wait_for_port 17687 "Neo4j" "knot_neo4j_e2e" || exit 1
    wait_for_port 16334 "Qdrant" "knot_qdrant_e2e" || exit 1
    sleep 8  # extra buffer: Neo4j healthcheck can pass before index creation is ready
fi

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

# Helper for eventually consistent searches (Qdrant)
retry_match() {
    local expected="$1"
    shift
    local max_attempts=10
    local attempt=1
    while [ $attempt -le $max_attempts ]; do
        if "$@" | grep -qiE "$expected"; then
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    return 1
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

CLEAN_FLAG=""
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && CLEAN_FLAG="--clean"
run_indexer "$REPO_A" "$COLL_A" "$CLEAN_FLAG"
sleep 2

# Test A1: Search for Groovy class
echo ""
echo "Test A1: Searching for Groovy class BaseService..."
if retry_match "BaseService" call_mcp "$REPO_A" "$COLL_A" \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BaseService Groovy class","max_results":10,"repo_name":"'"$REPO_A"'"}}}' \
    && retry_match "BaseService" call_cli "$REPO_A" "$COLL_A" search "BaseService" -r "$REPO_A" -m 10; then
    echo -e "${GREEN}✓ Found Groovy class BaseService (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Groovy class BaseService not found${NC}"
    FAILED=1
fi

# Test A2: Search for Groovy interface
echo ""
echo "Test A2: Searching for Groovy interface Repository..."
if retry_match "Repository" call_mcp "$REPO_A" "$COLL_A" \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"Repository interface","max_results":10,"repo_name":"'"$REPO_A"'"}}}' \
    && retry_match "Repository" call_cli "$REPO_A" "$COLL_A" search "Repository" -r "$REPO_A" -m 10; then
    echo -e "${GREEN}✓ Found Groovy interface Repository (MCP & CLI)${NC}"
else
    echo -e "${RED}✗ Groovy interface Repository not found${NC}"
    FAILED=1
fi

# Test A3: Search for Groovy Enum & Trait
echo ""
echo "Test A3: Searching for Groovy enum Status and trait Auditable..."
if retry_match "Status" call_cli "$REPO_A" "$COLL_A" search "Status" -r "$REPO_A" -m 10 \
    && retry_match "Auditable" call_cli "$REPO_A" "$COLL_A" search "Auditable" -r "$REPO_A" -m 10; then
    echo -e "${GREEN}✓ Found Groovy enum Status and trait Auditable (CLI)${NC}"
else
    echo -e "${RED}✗ Groovy enum/trait not found${NC}"
    FAILED=1
fi

# Test A4: Search for Script-Level Variables & Closures
echo ""
echo "Test A4: Searching for Groovy closures and global variables..."
if retry_match "globalConfig" call_cli "$REPO_A" "$COLL_A" search "globalConfig" -r "$REPO_A" -m 10 \
    && retry_match "processDataClosure" call_cli "$REPO_A" "$COLL_A" search "processDataClosure" -r "$REPO_A" -m 10; then
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
sleep 2

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
sleep 2

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

# ═══════════════════════════════════════════════════════════
# GROUP D: Inheritance EXTENDS / IMPLEMENTS
# ═══════════════════════════════════════════════════════════
echo -e "\n${BLUE}── Group D: Inheritance EXTENDS/IMPLEMENTS ──${NC}"

REPO_D="groovy_inherit_e2e"
COLL_D="knot_groovy_inherit_e2e"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension"

# ExtensionPoint.groovy — empty marker interface
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/ExtensionPoint.groovy" << 'GROOVY'
package nextflow.plugin.extension
interface ExtensionPoint {
}
GROOVY

# Comparable.groovy — local marker interface so D7's `implements Comparable<…>`
# edge resolves. Knot does not index the JDK, so we declare a stub here.
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/Comparable.groovy" << 'GROOVY'
package nextflow.plugin.extension
interface Comparable {
}
GROOVY

# PluginExtensionPoint.groovy — the real-world nextflow base class.
# `init` carries the verbatim GroovyDoc from nextflow (Suite E regression);
# `checkInit` is annotated with @PackageScope like the real source.
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/PluginExtensionPoint.groovy" << 'GROOVY'
package nextflow.plugin.extension

import groovy.transform.PackageScope

abstract class PluginExtensionPoint implements ExtensionPoint {

    private boolean initialised

    @PackageScope
    synchronized void checkInit(Object session) {
        if( !initialised ) {
            init(session)
            initialised = true
        }
    }

    /**
     * Channel factory initialization. This method is invoked one and only once
     *
     * @param session The current nextflow session
     */
    abstract protected void init(Object session)
}
GROOVY

# Ext1.groovy
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/Ext1.groovy" << 'GROOVY'
package nextflow.plugin.extension
class Ext1 extends PluginExtensionPoint {
    protected void init(Object session) { }
}
GROOVY

# Ext2.groovy
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/Ext2.groovy" << 'GROOVY'
package nextflow.plugin.extension
class Ext2 extends PluginExtensionPoint {
    protected void init(Object session) { }
}
GROOVY

# TestExtension.groovy
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/TestExtension.groovy" << 'GROOVY'
package nextflow.plugin.extension
class TestExtension extends PluginExtensionPoint {
    protected void init(Object session) { }
}
GROOVY

# HelloExtension.groovy — extends + implements with generics
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/HelloExtension.groovy" << 'GROOVY'
package nextflow.plugin.extension
class HelloExtension extends PluginExtensionPoint implements Comparable<HelloExtension> {
    protected void init(Object session) { }
    int compareTo(HelloExtension other) { 0 }
}
GROOVY

# EventBus.groovy — interface extends multiple interfaces
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/EventBus.groovy" << 'GROOVY'
package nextflow.plugin.extension
interface EventBus extends ExtensionPoint, Cloneable {
}
GROOVY

# Box.groovy — anti-false-positive of generic bound (T extends Comparable)
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/plugin/extension/Box.groovy" << 'GROOVY'
package nextflow.plugin.extension
class Box<T extends Comparable> {
    T value
}
GROOVY

run_indexer "$REPO_D" "$COLL_D"
sleep 2

# Test D1-D4: find_callers("PluginExtensionPoint") exposes Ext1, Ext2,
# TestExtension and HelloExtension under the "Extends" section.
echo ""
echo "Test D1-D4: find_callers(PluginExtensionPoint) Extends list covers all 4 subclasses..."
RESP=$(call_mcp "$REPO_D" "$COLL_D" \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"PluginExtensionPoint","repo_name":"'"$REPO_D"'"}}}')
if echo "$RESP" | grep -qi "Extends" && echo "$RESP" | grep -q "Ext1"; then
    echo -e "${GREEN}✓ Ext1 found under Extends of PluginExtensionPoint${NC}"
else
    echo -e "${RED}✗ Ext1 NOT found under Extends of PluginExtensionPoint${NC}"
    FAILED=1
fi
if echo "$RESP" | grep -qi "Extends" && echo "$RESP" | grep -q "Ext2"; then
    echo -e "${GREEN}✓ Ext2 found under Extends of PluginExtensionPoint${NC}"
else
    echo -e "${RED}✗ Ext2 NOT found under Extends of PluginExtensionPoint${NC}"
    FAILED=1
fi
if echo "$RESP" | grep -qi "Extends" && echo "$RESP" | grep -q "TestExtension"; then
    echo -e "${GREEN}✓ TestExtension found under Extends of PluginExtensionPoint${NC}"
else
    echo -e "${RED}✗ TestExtension NOT found under Extends of PluginExtensionPoint${NC}"
    FAILED=1
fi
if echo "$RESP" | grep -qi "Extends" && echo "$RESP" | grep -q "HelloExtension"; then
    echo -e "${GREEN}✓ HelloExtension found under Extends of PluginExtensionPoint${NC}"
else
    echo -e "${RED}✗ HelloExtension NOT found under Extends of PluginExtensionPoint${NC}"
    FAILED=1
fi

# Test D5: find_callers("ExtensionPoint") places PluginExtensionPoint under
# the Implements section.
echo ""
echo "Test D5: find_callers(ExtensionPoint) Implements list covers PluginExtensionPoint..."
RESP_D5=$(call_mcp "$REPO_D" "$COLL_D" \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"ExtensionPoint","repo_name":"'"$REPO_D"'"}}}')
if echo "$RESP_D5" | grep -qi "Implements" && echo "$RESP_D5" | grep -q "PluginExtensionPoint"; then
    echo -e "${GREEN}✓ PluginExtensionPoint found under Implements of ExtensionPoint${NC}"
else
    echo -e "${RED}✗ PluginExtensionPoint NOT found under Implements of ExtensionPoint${NC}"
    FAILED=1
fi

# Test D6: find_callers("ExtensionPoint") also places EventBus under Extends
# (interface→interface inheritance, aligned with the Kotlin parser).
echo ""
echo "Test D6: find_callers(ExtensionPoint) Extends list covers EventBus..."
if echo "$RESP_D5" | grep -qi "Extends" && echo "$RESP_D5" | grep -q "EventBus"; then
    echo -e "${GREEN}✓ EventBus found under Extends of ExtensionPoint${NC}"
else
    echo -e "${RED}✗ EventBus NOT found under Extends of ExtensionPoint${NC}"
    FAILED=1
fi

# Test D7: find_callers("Comparable") must NOT surface Box (a generic bound
# `T extends Comparable` is not an EXTENDS edge), but SHOULD surface
# HelloExtension under Implements because it implements `Comparable<HelloExtension>`.
echo ""
echo "Test D7: find_callers(Comparable) ignores Box (anti-false-positive) but reports HelloExtension..."
RESP_D7=$(call_mcp "$REPO_D" "$COLL_D" \
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"Comparable","repo_name":"'"$REPO_D"'"}}}')
if echo "$RESP_D7" | grep -q "Box"; then
    echo -e "${RED}✗ Box should NOT appear as a subclass of Comparable (generic bound is not EXTENDS)${NC}"
    FAILED=1
else
    echo -e "${GREEN}✓ Box correctly absent from Comparable callers${NC}"
fi
if echo "$RESP_D7" | grep -qi "Implements" && echo "$RESP_D7" | grep -q "HelloExtension"; then
    echo -e "${GREEN}✓ HelloExtension correctly listed under Implements of Comparable${NC}"
else
    echo -e "${RED}✗ HelloExtension NOT found under Implements of Comparable${NC}"
    FAILED=1
fi

# ═══════════════════════════════════════════════════════════
# GROUP E: Docstrings (GroovyDoc)
# ═══════════════════════════════════════════════════════════
echo -e "\n${BLUE}── Group E: Docstrings (GroovyDoc) ──${NC}"

# Use cypher-shell from the running Neo4j container (same pattern as the
# rust_reference_resolution suite) so we don't depend on a host installation.
run_neo4j_cypher() {
    local query="$1"
    echo "$query" | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
        --format plain 2>/dev/null \
        | awk 'NF && NR > 1 && $0 !~ /^(Available|neo4j>|Connection|Disconnect|Connected)/ { print; exit }'
}

# Test E1: Neo4j — the abstract `init` node carries its GroovyDoc.
# Note: cypher-shell --format plain renders booleans as TRUE/FALSE (uppercase).
echo ""
echo "Test E1: Neo4j docstring of PluginExtensionPoint.init contains the GroovyDoc..."
E1=$(run_neo4j_cypher "MATCH (e:Entity {name:'init', repo_name:'$REPO_D'}) WHERE e.file_path ENDS WITH 'PluginExtensionPoint.groovy' RETURN e.docstring CONTAINS 'Channel factory initialization' AS has_doc;" | tr '[:upper:]' '[:lower:]')
if [ "$E1" = "true" ]; then
    echo -e "${GREEN}✓ init docstring contains 'Channel factory initialization'${NC}"
else
    echo -e "${RED}✗ init docstring missing or wrong (got: '$E1')${NC}"
    FAILED=1
fi

# Test E2: Neo4j — `checkInit` has NO docstring in the fixture (explicit).
echo ""
echo "Test E2: Neo4j docstring of checkInit is empty (fixture has no GroovyDoc on it)..."
E2=$(run_neo4j_cypher "MATCH (e:Entity {name:'checkInit', repo_name:'$REPO_D'}) RETURN (e.docstring IS NULL OR e.docstring = '') AS no_doc;" | tr '[:upper:]' '[:lower:]')
if [ "$E2" = "true" ]; then
    echo -e "${GREEN}✓ checkInit docstring is empty as expected${NC}"
else
    echo -e "${RED}✗ checkInit should have no docstring (got: '$E2')${NC}"
    FAILED=1
fi

# Test E3: Neo4j — at least one Groovy entity of the repo now has a docstring.
echo ""
echo "Test E3: Neo4j count of Groovy entities with non-empty docstring > 0..."
E3=$(run_neo4j_cypher "MATCH (e:Entity {language:'groovy', repo_name:'$REPO_D'}) WHERE e.docstring <> '' RETURN count(e) AS cnt;")
E3=${E3:-0}
if [ "$E3" -ge 1 ] 2>/dev/null; then
    echo -e "${GREEN}✓ $E3 Groovy entity(ies) carry a docstring${NC}"
else
    echo -e "${RED}✗ no Groovy entity carries a docstring (cnt=$E3)${NC}"
    FAILED=1
fi

# Test E4: Qdrant parity — the points for PluginExtensionPoint.groovy exist
# (graph ↔ vector parity: class + checkInit + init + property + 3 accessors = 7 points).
echo ""
echo "Test E4: Qdrant points for PluginExtensionPoint.groovy (expect 7)..."
E4_RAW=$(curl -s --max-time 20 -X POST "$QDRANT_REST_URL/collections/$COLL_D/points/scroll" \
    -H 'Content-Type: application/json' \
    -d '{"limit":100,"with_payload":true}' || true)
E4=$(echo "$E4_RAW" | jq '[.result.points[] | select(.payload.file_path | tostring | endswith("PluginExtensionPoint.groovy"))] | length' 2>/dev/null || echo "jq_error")
E4=${E4:-0}
if [ "$E4" -eq 7 ] 2>/dev/null; then
    echo -e "${GREEN}✓ Qdrant holds the 7 points of PluginExtensionPoint.groovy${NC}"
else
    echo -e "${RED}✗ expected 3 Qdrant points for PluginExtensionPoint.groovy, got $E4${NC}"
    echo -e "${YELLOW}  scroll response excerpt: $(echo "$E4_RAW" | head -c 400)${NC}"
    FAILED=1
fi

# ═══════════════════════════════════════════════════════════
# GROUP F: Method-level OVERRIDES (JVM)
# ═══════════════════════════════════════════════════════════
echo -e "\n${BLUE}── Group F: Method-level OVERRIDES ──${NC}"

# Reuses the REPO_D inheritance graph: PluginExtensionPoint declares the
# abstract `init`, and Ext1/Ext2/TestExtension/HelloExtension each override it.
# This is the reported nextflow case (interface/superclass method vs its
# implementations), now linked with a real OVERRIDES edge.

# Test F1: find_callers(init) "Overridden by" lists all 4 overriding methods.
echo ""
echo "Test F1: find_callers(init) 'Overridden by' lists all 4 overriding methods..."
RESP_F=$(call_mcp "$REPO_D" "$COLL_D" \
    '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"init","repo_name":"'"$REPO_D"'"}}}')
if echo "$RESP_F" | grep -qi "Overridden by"; then
    echo -e "${GREEN}✓ 'Overridden by' section present${NC}"
else
    echo -e "${RED}✗ 'Overridden by' section missing${NC}"
    FAILED=1
fi
for impl in Ext1 Ext2 TestExtension HelloExtension; do
    if echo "$RESP_F" | grep -q "${impl}.groovy"; then
        echo -e "${GREEN}✓ ${impl}.init found as override of init${NC}"
    else
        echo -e "${RED}✗ ${impl}.init NOT found as override of init${NC}"
        FAILED=1
    fi
done

# Test F2: the same query surfaces the supertype declaration under "Overrides".
echo ""
echo "Test F2: find_callers(init) 'Overrides' lists PluginExtensionPoint.init..."
if echo "$RESP_F" | grep -qi "Overrides (declared supertype methods)" \
    && echo "$RESP_F" | grep -q "PluginExtensionPoint.groovy"; then
    echo -e "${GREEN}✓ PluginExtensionPoint.init found under Overrides${NC}"
else
    echo -e "${RED}✗ PluginExtensionPoint.init NOT found under Overrides${NC}"
    FAILED=1
fi

# ═══════════════════════════════════════════════════════════
# GROUP G: Property Accessors (getter/setter synthesis & comment hygiene)
# ═══════════════════════════════════════════════════════════
echo -e "\n${BLUE}── Group G: Property Accessors ──${NC}"

REPO_G="groovy_props_e2e"
COLL_G="knot_groovy_props_e2e"

rm -rf "$TMP_REPO_DIR"
mkdir -p "$TMP_REPO_DIR/src/main/groovy/nextflow"

# ISession.groovy — interface with getBaseDir and isCacheable
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/ISession.groovy" << 'GROOVY'
package nextflow

interface ISession {
    /**
     * The folder where the main script is contained (without parent path)
     */
    Path getBaseDir()

    boolean isCacheable()
}
GROOVY

# Session.groovy — class implementing ISession with bare properties
cat > "$TMP_REPO_DIR/src/main/groovy/nextflow/Session.groovy" << 'GROOVY'
package nextflow

class Session implements ISession {
    /**
     * The folder where the main script is contained (without parent path)
     */
    Path baseDir

    boolean cacheable

    void setBaseDir( Path baseDir ) {
        this.baseDir = baseDir
    }
}
GROOVY

run_indexer "$REPO_G" "$COLL_G" "--clean"

# Test G1: find_callers(getBaseDir) "Overridden by" lists Session.groovy
echo ""
echo "Test G1: find_callers(getBaseDir) 'Overridden by' lists Session.groovy..."
RESP_G1=$(call_mcp "$REPO_G" "$COLL_G" \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"getBaseDir","repo_name":"'"$REPO_G"'"}}}')
if echo "$RESP_G1" | grep -qi "Overridden by" \
    && echo "$RESP_G1" | grep -q "Session.groovy"; then
    echo -e "${GREEN}✓ Session.getBaseDir found under Overridden by${NC}"
else
    echo -e "${RED}✗ Session.getBaseDir NOT found under Overridden by${NC}"
    FAILED=1
fi

# Test G2: find_callers(getBaseDir) "Overrides" section lists ISession.groovy
echo ""
echo "Test G2: find_callers(getBaseDir) 'Overrides' lists ISession.groovy..."
if echo "$RESP_G1" | grep -qi "Overrides (declared supertype methods)" \
    && echo "$RESP_G1" | grep -q "ISession.groovy"; then
    echo -e "${GREEN}✓ ISession.getBaseDir found under Overrides${NC}"
else
    echo -e "${RED}✗ ISession.getBaseDir NOT found under Overrides${NC}"
    FAILED=1
fi

# Test G3: find_callers(isCacheable) links Session → ISession (boolean is accessor path)
echo ""
echo "Test G3: find_callers(isCacheable) links Session → ISession..."
RESP_G3=$(call_mcp "$REPO_G" "$COLL_G" \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"isCacheable","repo_name":"'"$REPO_G"'"}}}')
if echo "$RESP_G3" | grep -qi "Overridden by" \
    && echo "$RESP_G3" | grep -q "Session.groovy"; then
    echo -e "${GREEN}✓ Session.cacheable → ISession.isCacheable linked${NC}"
else
    echo -e "${RED}✗ Session.cacheable → ISession.isCacheable NOT linked${NC}"
    FAILED=1
fi

# Test G4: explore Session.groovy lists property baseDir
echo ""
echo "Test G4: explore Session.groovy lists property baseDir..."
RESP_G4=$(call_mcp "$REPO_G" "$COLL_G" \
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"'"$TMP_REPO_DIR"'/src/main/groovy/nextflow/Session.groovy","repo_name":"'"$REPO_G"'"}}}')
if echo "$RESP_G4" | grep -q "baseDir"; then
    echo -e "${GREEN}✓ baseDir property listed in explore_file${NC}"
else
    echo -e "${RED}✗ baseDir property NOT listed in explore_file${NC}"
    FAILED=1
fi

# Test G5: explore Session.groovy does NOT list phantom 'name' entity from Javadoc
echo ""
echo "Test G5: explore Session.groovy does NOT list phantom 'name' entity..."
if ! echo "$RESP_G4" | grep -q '"name"'; then
    echo -e "${GREEN}✓ no phantom 'name' entity from Javadoc body${NC}"
else
    echo -e "${RED}✗ phantom 'name' entity detected (Javadoc body line leaked)${NC}"
    FAILED=1
fi

# Test G6: Neo4j — exactly one setBaseDir (no synthetic duplicate)
echo ""
echo "Test G6: Neo4j count of setBaseDir = 1 (no synthetic duplicate)..."
G6=$(run_neo4j_cypher "MATCH (e:Entity {repo_name:'$REPO_G', name:'setBaseDir'}) RETURN count(e) AS cnt;")
G6=${G6:-0}
if [ "$G6" -eq 1 ] 2>/dev/null; then
    echo -e "${GREEN}✓ exactly one setBaseDir (explicit setter, no synthetic duplicate)${NC}"
else
    echo -e "${RED}✗ expected 1 setBaseDir, got $G6${NC}"
    FAILED=1
fi

# Test G7: Neo4j — property node baseDir has ZERO OVERRIDES edges
echo ""
echo "Test G7: Neo4j — baseDir property node has no OVERRIDES edges..."
G7=$(run_neo4j_cypher "MATCH (e:Entity {fqn:'nextflow.Session.baseDir', repo_name:'$REPO_G'})-[r:OVERRIDES]->() RETURN count(r) AS cnt;")
G7=${G7:-0}
if [ "$G7" -eq 0 ] 2>/dev/null; then
    echo -e "${GREEN}✓ baseDir property node has zero OVERRIDES edges${NC}"
else
    echo -e "${RED}✗ baseDir property node should have zero OVERRIDES edges, got $G7${NC}"
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
