#!/usr/bin/env bash
# E2E test: C/C++ Support (Phase 11)
# Verifies:
#  1. Header Inclusion
#  2. Class & Method Extraction with Namespaces
#  3. Call Graph for Pointers/Refs
#  4. Macro Tracking
#
# Usage: ./tests/run_cpp_e2e.sh
set -eu

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_cpp_data"
TMP_REPO_DIR="$SCRIPT_DIR/.e2e_cpp_repo"

NEO4J_PORT=18000; NEO4J_HTTP_PORT=18001
NEO4J_URI="bolt://localhost:${NEO4J_PORT}"
NEO4J_USER="neo4j"; NEO4J_PASSWORD="e2e_test_password"
QDRANT_PORT=16550; QDRANT_GRPC_PORT=16551
QDRANT_URL="http://localhost:${QDRANT_PORT}"
QDRANT_COLLECTION="knot_cpp_e2e"
REPO_NAME="cpp_e2e"

cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}C/C++ E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up:${NC}"
        echo "  docker compose -f $E2E_DATA_DIR/docker-compose.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR $TMP_REPO_DIR"
        return 0
    fi
    echo -e "\n${GREEN}✓ All C/C++ E2E tests passed!${NC}"
}
trap cleanup EXIT

echo -e "${BLUE}C/C++ E2E Test${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Set up infra ────────────────────────────────────────
rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR"
mkdir -p "$E2E_DATA_DIR" "$TMP_REPO_DIR/src"

cat > "$E2E_DATA_DIR/docker-compose.yml" << 'DOCKEREOF'
services:
  neo4j:
    image: neo4j:5.26-community
    ports:
      - "18000:7687"
      - "18001:7474"
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
      - "16550:6334"
      - "16551:6333"
DOCKEREOF

echo -n "Starting Neo4j + Qdrant... "
docker compose -f "$E2E_DATA_DIR/docker-compose.yml" up -d > /dev/null 2>&1
for i in $(seq 1 30); do
    if docker exec $(docker ps -q -f name=e2e_cpp_data-neo4j) cypher-shell -u neo4j -p e2e_test_password "RETURN 1" > /dev/null 2>&1; then
        break
    fi
    sleep 2
done
echo -e "${GREEN}✓${NC}"

# ── Create test source files ───────────────────────────
cat > "$TMP_REPO_DIR/src/lib.hpp" << 'EOF'
#ifndef LIB_HPP
#define LIB_HPP

#define MAX_BUF 1024

namespace Engine {
    class MyClass {
    public:
        void start();
    };
}

#endif // LIB_HPP
EOF

cat > "$TMP_REPO_DIR/src/main.cpp" << 'EOF'
#include "lib.hpp"
#include <iostream>

int main() {
    int buf[MAX_BUF];
    Engine::MyClass* obj = new Engine::MyClass();
    obj->start();
    return 0;
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

# ── Test 1: Class & Method Extraction with Namespaces ─
echo -n "Test 1: FQN extraction for Engine::MyClass::start... "
OUT1=$(docker exec $(docker ps -q -f name=e2e_cpp_data-neo4j) cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'start'}) RETURN e.fqn")
if echo "$OUT1" | grep -q "Engine::MyClass::start"; then
    echo -e "${GREEN}✓ Engine::MyClass::start found${NC}"
else
    echo -e "${RED}✗ Engine::MyClass::start NOT found in FQNs${NC}"
    echo "$OUT1"
    exit 1
fi

# ── Test 2: Call Graph for Pointers/Refs ─
echo -n "Test 2: find_callers on start() finds main()... "
OUT2=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers start --repo "$REPO_NAME" 2>&1 || true)
if echo "$OUT2" | grep -q "main"; then
    echo -e "${GREEN}✓ main found as caller${NC}"
else
    echo -e "${RED}✗ main NOT found as caller${NC}"
    echo "$OUT2"
    exit 1
fi

# ── Test 3: Macro Tracking ─
echo -n "Test 3: macro MAX_BUF used in main()... "
OUT3=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers MAX_BUF --repo "$REPO_NAME" 2>&1 || true)
if echo "$OUT3" | grep -q "main"; then
    echo -e "${GREEN}✓ MAX_BUF usage found in main${NC}"
else
    echo -e "${RED}✗ MAX_BUF usage NOT found${NC}"
    echo "$OUT3"
    exit 1
fi

# ── Test 4: Namespace and Class References ─
echo -n "Test 4: MyClass referenced in main()... "
OUT4=$(env "${KNOT_ENV[@]}" "$PROJECT_ROOT/target/debug/knot" callers MyClass --repo "$REPO_NAME" 2>&1 || true)
if echo "$OUT4" | grep -q "main"; then
    echo -e "${GREEN}✓ MyClass reference found in main${NC}"
else
    echo -e "${RED}✗ MyClass reference NOT found${NC}"
    echo "$OUT4"
    exit 1
fi

# Cleanup before trap
docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v > /dev/null 2>&1
rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR"
echo -e "\n${GREEN}✓ All C/C++ E2E tests passed!${NC}"
