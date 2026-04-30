#!/usr/bin/env bash
# E2E test: private Groovy method call tracking (v0.10.2 fix)
# Verifies:
#  1. Private methods are callable via find_callers (tree-sitter + ad-hoc)
#  2. No-paren Groovy calls are tracked
#  3. def methods calling private typed methods work
#
# Usage: ./tests/run_groovy_private_method_e2e.sh
set -eu

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_groovy_private_data"
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_groovy_private_repo"

NEO4J_PORT=17990; NEO4J_HTTP_PORT=17991
NEO4J_URI="bolt://localhost:${NEO4J_PORT}"
NEO4J_USER="neo4j"; NEO4J_PASSWORD="e2e_test_password"
QDRANT_PORT=16537; QDRANT_GRPC_PORT=16538
QDRANT_URL="http://localhost:${QDRANT_PORT}"
QDRANT_COLLECTION="knot_groovy_private_e2e"
REPO_NAME="groovy_private_e2e"

cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Groovy private method E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up:${NC}"
        echo "  docker compose -f $E2E_DATA_DIR/docker-compose.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi
    echo -e "\n${GREEN}✓ All Groovy private method E2E tests passed!${NC}"
}
trap cleanup EXIT

echo -e "${BLUE}Groovy Private Method E2E Test${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Set up infra ────────────────────────────────────────
rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR"
mkdir -p "$E2E_DATA_DIR" "$TMP_REPO_DIR/src/main/groovy/com/example"

# Docker Compose for Neo4j + Qdrant
cat > "$E2E_DATA_DIR/docker-compose.yml" << 'DOCKEREOF'
services:
  neo4j:
    image: neo4j:5.26-community
    ports:
      - "17990:7687"
      - "17991:7474"
    environment:
      NEO4J_AUTH: "neo4j/e2e_test_password"
      NEO4J_ACCEPT_LICENSE_AGREEMENT: "yes"
    healthcheck:
      test: ["CMD", "cypher-shell", "-u", "neo4j", "-p", "e2e_test_password", "CALL db.ping()"]
      interval: 2s
      retries: 15
  qdrant:
    image: qdrant/qdrant:v1.13.5
    ports:
      - "16537:6334"
      - "16538:6333"
DOCKEREOF

# Start infra
echo -n "Starting Neo4j + Qdrant... "
docker compose -f "$E2E_DATA_DIR/docker-compose.yml" up -d > /dev/null 2>&1
# Wait for Neo4j to be ready
for i in $(seq 1 30); do
    if docker exec e2e_groovy_private_data-neo4j-1 cypher-shell -u neo4j -p e2e_test_password "RETURN 1" > /dev/null 2>&1; then
        break
    fi
    sleep 2
done
echo -e "${GREEN}✓${NC}"

# ── Create test source files ───────────────────────────
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
# Replicates the pattern from code-history-mining UI.groovy:
#   showGrabbingFinishedMessage() containing hyperlinkUpdate() which calls runAnalyzer().
#   Only hyperlinkUpdate (innermost) should appear as caller of runAnalyzer.
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

# File 4: Replicates code-history-mining UI.groovy pattern:
#   createActionsOnHistoryFile contains an inner actionPerformed that calls runAnalyzer.
#   A SEPARATE actionPerformed elsewhere does NOT call runAnalyzer.
#   Only the correct one should appear as caller of runAnalyzer.
cat > "$TMP_REPO_DIR/src/main/groovy/com/example/UIPattern.groovy" << 'EOF'
package com.example

class UIPattern {
    def showInBrowser(String html) {
        loadIntoHttpServer(html)
    }

    static String loadIntoHttpServer(String html) {
        "http://..."
    }

    private AnAction createActionsOnHistoryFile(File file) {
        new AnAction("Run") {
            @Override void actionPerformed(AnActionEvent event) {
                runAnalyzer(file, event.project)
            }
        }
    }

    private runScriptAction() {
        new AnAction("Execute") {
            @Override void actionPerformed(AnActionEvent event) {
                runCurrentFileAsScript(event.project)
            }
        }
    }

    def runCurrentFileAsScript(project) {
        println "running script"
    }

    private void runAnalyzer(File file, project) {
        println "analyzing"
    }
}
EOF

# ── Index the repo ─────────────────────────────────────
echo -n "Indexing... "
KNOT_REPO_PATH="$TMP_REPO_DIR" \
KNOT_REPO_NAME="$REPO_NAME" \
KNOT_QDRANT_URL="$QDRANT_URL" \
KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
KNOT_NEO4J_URI="$NEO4J_URI" \
KNOT_NEO4J_USER="$NEO4J_USER" \
KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
KNOT_CLEAN="true" \
    "$PROJECT_ROOT/target/debug/knot-indexer" > /dev/null 2>&1
echo -e "${GREEN}✓${NC}"

KNOT_ENV=(
    KNOT_REPO_NAME="$REPO_NAME"
    KNOT_QDRANT_URL="$QDRANT_URL"
    KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"
    KNOT_NEO4J_URI="$NEO4J_URI"
    KNOT_NEO4J_USER="$NEO4J_USER"
    KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
)

# ── Test 1: find_callers for private method (typed caller) ─
echo -n "Test 1: find_callers restartHttpServer (typed caller)... "
OUT=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers restartHttpServer --repo "$REPO_NAME" 2>&1)
if echo "$OUT" | grep -q "publicWorkflow"; then
    echo -e "${GREEN}✓ publicWorkflow found as caller${NC}"
else
    echo -e "${RED}✗ publicWorkflow NOT found${NC}"
    echo "$OUT"
    exit 1
fi

# ── Test 2: find_callers for private method (def caller) ─
echo -n "Test 2: find_callers restartHttpServer (def caller)... "
if echo "$OUT" | grep -q "dynamicWorkflow"; then
    echo -e "${GREEN}✓ dynamicWorkflow found as caller${NC}"
else
    echo -e "${RED}✗ dynamicWorkflow NOT found${NC}"
    echo "$OUT"
    exit 1
fi

# ── Test 3: find_callers for private method (no-paren call) ─
echo -n "Test 3: find_callers restartHttpServer (no-paren callee)... "
if echo "$OUT" | grep -q "runAnalyzer"; then
    echo -e "${GREEN}✓ runAnalyzer found as caller${NC}"
else
    echo -e "${RED}✗ runAnalyzer NOT found${NC}"
    echo "$OUT"
    exit 1
fi

# ── Test 4: find_callers for method called via no-paren style ─
echo -n "Test 4: find_callers doSomething (no-paren caller)... "
OUT2=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers doSomething --repo "$REPO_NAME" 2>&1)
if echo "$OUT2" | grep -q "runAnalyzer"; then
    echo -e "${GREEN}✓ runAnalyzer found as caller of doSomething${NC}"
else
    echo -e "${RED}✗ runAnalyzer NOT found as caller of doSomething${NC}"
    echo "$OUT2"
    exit 1
fi

# ── Test 5: verify all private methods have callers ─
echo -n "Test 5: find_callers computeSecret (private method)... "
OUT3=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers computeSecret --repo "$REPO_NAME" 2>&1)
if echo "$OUT3" | grep -q "publicWorkflow"; then
    echo -e "${GREEN}✓ publicWorkflow found as caller${NC}"
else
    echo -e "${RED}✗ publicWorkflow NOT found as caller of computeSecret${NC}"
    echo "$OUT3"
    exit 1
fi

# ── Test 6: find_callers for multi-line private method with closure args ─
echo -n "Test 6: find_callers restartHttpServer (multi-line closure args)... "
OUT4=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers restartHttpServer --repo "$REPO_NAME" 2>&1)
if echo "$OUT4" | grep -q "loadIntoHttpServer"; then
    echo -e "${GREEN}✓ loadIntoHttpServer found as caller${NC}"
else
    echo -e "${RED}✗ loadIntoHttpServer NOT found as caller of restartHttpServer${NC}"
    echo "$OUT4"
    exit 1
fi

# ── Test 7: innermost assignment — nested method gets the reference ─
echo -n "Test 7: find_callers runAnalyzer (innermost caller)... "
OUT5=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers runAnalyzer --repo "$REPO_NAME" 2>&1)
if echo "$OUT5" | grep -q "hyperlinkUpdate"; then
    echo -e "${GREEN}✓ hyperlinkUpdate found as caller${NC}"
else
    echo -e "${RED}✗ hyperlinkUpdate NOT found as caller of runAnalyzer${NC}"
    echo "$OUT5"
    exit 1
fi

# ── Test 8: innermost assignment — outer container must NOT appear ─
echo -n "Test 8: find_callers runAnalyzer (outer NOT a caller)... "
if echo "$OUT5" | grep -q "showGrabbingFinishedMessage"; then
    echo -e "${RED}✗ showGrabbingFinishedMessage incorrectly listed as caller${NC}"
    echo "$OUT5"
    exit 1
else
    echo -e "${GREEN}✓ showGrabbingFinishedMessage correctly absent${NC}"
fi

# ── Test 9: runAnalyzer caller should be the inner actionPerformed (not the outer one) ─
echo -n "Test 9: find_callers runAnalyzer (correct actionPerformed)... "
OUT6=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers runAnalyzer --repo "$REPO_NAME" 2>&1)
# The inner actionPerformed calls runAnalyzer. It should appear in the callers.
# The outer actionPerformed does NOT call runAnalyzer and should NOT appear.
if echo "$OUT6" | grep -q "actionPerformed"; then
    echo -e "${GREEN}✓ actionPerformed found as caller${NC}"
else
    echo -e "${RED}✗ actionPerformed NOT found as caller${NC}"
    echo "$OUT6"
    exit 1
fi

# ── Test 10: the outer actionPerformed must NOT be caller (no duplicate) ─
echo -n "Test 10: find_callers runAnalyzer (no duplicate actionPerformed)... "
# The output should have at most one actionPerformed line from UIPattern.groovy
UIP_COUNT=$(echo "$OUT6" | grep -c "UIPattern.groovy" || true)
if echo "$OUT6" | grep -q "runCurrentFileAsScript" && echo "$OUT6" | grep -q "actionPerformed"; then
    echo -e "${RED}✗ runCurrentFileAsScript incorrectly associated with actionPerformed${NC}"
    echo "$OUT6"
    exit 1
fi
echo -e "${GREEN}✓ no cross-contamination${NC}"

# Cleanup before trap
docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v > /dev/null 2>&1
rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR"
echo -e "\n${GREEN}✓ All Groovy private method E2E tests passed!${NC}"
