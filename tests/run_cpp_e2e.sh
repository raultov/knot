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
    fi
    docker compose -f "$E2E_DATA_DIR/docker-compose.yml" down -v 2>/dev/null || true
    sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    rm -rf "$TMP_REPO_DIR" 2>/dev/null || true
}
trap cleanup EXIT

echo -e "${BLUE}C/C++ E2E Test${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Set up infra ────────────────────────────────────────
sudo rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" "$TMP_REPO_DIR" 2>/dev/null || true
mkdir -p "$E2E_DATA_DIR" "$TMP_REPO_DIR/src"

cat > "$E2E_DATA_DIR/docker-compose.yml" << 'DOCKEREOF'
services:
  neo4j:
    image: neo4j:5.26-community
    container_name: knot_neo4j_cpp_e2e
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
    container_name: knot_qdrant_cpp_e2e
    ports:
      - "16550:6334"
      - "16551:6333"
DOCKEREOF

# Helper for eventually consistent searches (Qdrant/Neo4j index delays)
retry_match() {
    local expected="$1"
    shift
    local max_attempts=10
    local attempt=1
    while [ $attempt -le $max_attempts ]; do
        if "$@" 2>/dev/null | grep -qiE "$expected"; then
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    return 1
}

# Helper that retries until a command output contains at least N matches of a pattern
retry_count() {
    local expected_count="$1"
    local pattern="$2"
    shift 2
    local max_attempts=10
    local attempt=1
    while [ $attempt -le $max_attempts ]; do
        local count
        count=$("$@" 2>/dev/null | grep -cE "$pattern" || echo "0")
        if [ "$count" -ge "$expected_count" ]; then
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    return 1
}


echo -n "Starting Neo4j + Qdrant... "
docker compose -f "$E2E_DATA_DIR/docker-compose.yml" up -d
for i in $(seq 1 30); do
    if docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "RETURN 1" > /dev/null 2>&1; then
        break
    fi
    sleep 2
done
sleep 8
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

# File with operator overloads and reference return types (regression)
cat > "$TMP_REPO_DIR/src/string_ops.cpp" << 'EOF'
class String {
public:
    String & copy(const char *pstr, unsigned int length) { return *this; }
    String & operator =(const char *pstr) { if (pstr) copy(pstr, 10); return *this; }
    void clear() { copy("", 0); }
};
EOF

# Exact WString.cpp pattern: qualified definitions + macros + casts
cat > "$TMP_REPO_DIR/src/WString.cpp" << 'WEOF'
#include <string.h>

#define PGM_P const char *
#define strlen_P(s) strlen((s))

class __FlashStringHelper;

class String {
public:
    String & copy(const __FlashStringHelper *pstr, unsigned int length);
    String & operator = (const __FlashStringHelper *pstr);
    unsigned char reserve(unsigned int size);
    void invalidate();
    unsigned char * wbuffer();
    void setLen(unsigned int len);
};

// Out-of-class definition (exact WString.cpp pattern)
String & String::copy(const __FlashStringHelper *pstr, unsigned int length) {
    if (!reserve(length)) {
        invalidate();
        return *this;
    }
    memcpy(wbuffer(), (PGM_P)pstr, length + 1);
    setLen(length);
    return *this;
}

String & String::operator = (const __FlashStringHelper *pstr)
{
    if (pstr) copy(pstr, strlen_P((PGM_P)pstr));
    else invalidate();

    return *this;
}
WEOF

cat > "$TMP_REPO_DIR/src/overload_test.cpp" << 'EOF'
class Printer {
public:
    size_t write(uint8_t c);
    size_t write(const uint8_t *buffer, size_t size);
    size_t write(const char *str);
};

size_t Printer::write(uint8_t c) { return 1; }

size_t Printer::write(const uint8_t *buffer, size_t size) { return size; }

size_t Printer::write(const char *str) {
    return write((const uint8_t *)str, 10);
}

size_t Printer::printf(const char *fmt) {
    uint8_t buf[16];
    return write(buf, 16);
}

void test_overloads() {
    Printer p;
    p.write('a');
}
EOF

# ── Print.h / Print.cpp pattern: header (.h) with class + out-of-class
# definitions (.cpp). Reproduces bug where .h parsed as C corrupts entities.
cat > "$TMP_REPO_DIR/src/Print.h" << 'EOF'
#ifndef PRINT_H
#define PRINT_H

#include <stdint.h>
#include <stddef.h>

class Print {
public:
    virtual size_t write(uint8_t) = 0;
    size_t write(const uint8_t *buffer, size_t size);

    size_t print(const char str[]);
    size_t print(char c);
    size_t print(int n, int base);

    size_t println(const char[]);
    size_t println(char);
    size_t println(int, int);
    size_t println(void);
};

#endif
EOF

cat > "$TMP_REPO_DIR/src/Print.cpp" << 'EOF'
#include "Print.h"

size_t Print::write(const uint8_t *buffer, size_t size) {
    size_t n = 0;
    while(size--) {
        n += write(*buffer++);
    }
    return n;
}

size_t Print::print(const char str[]) {
    return write(str);
}

size_t Print::print(char c) {
    return write(c);
}

size_t Print::print(int n, int base) {
    return 0;
}

size_t Print::println(const char c[]) {
    size_t n = print(c);
    n += println();
    return n;
}

size_t Print::println(char c) {
    size_t n = print(c);
    n += println();
    return n;
}

size_t Print::println(int num, int base) {
    size_t n = print(num, base);
    n += println();
    return n;
}

size_t Print::println(void) {
    return print("\r\n");
}
EOF

# ── Index the repo ─────────────────────────────────────
echo -n "Indexing... "
cd "$PROJECT_ROOT"
IDX_OUT=$(env \
    KNOT_REPO_PATH="$TMP_REPO_DIR" \
    KNOT_REPO_NAME="$REPO_NAME" \
    KNOT_QDRANT_URL="$QDRANT_URL" \
    KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
    KNOT_NEO4J_URI="$NEO4J_URI" \
    KNOT_NEO4J_USER="$NEO4J_USER" \
    KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
    KNOT_CLEAN="true" \
    cargo run --release --bin knot-indexer -- --clean 2>&1)
IDX_EXIT=$?
if [ $IDX_EXIT -eq 0 ] && echo "$IDX_OUT" | grep -q "Incremental\|initial"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ Indexing failed (exit: $IDX_EXIT)${NC}"
    echo "$IDX_OUT"
    exit 1
fi

sleep 2

KNOT_ENV=(
    KNOT_REPO_NAME="$REPO_NAME"
    KNOT_QDRANT_URL="$QDRANT_URL"
    KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"
    KNOT_NEO4J_URI="$NEO4J_URI"
    KNOT_NEO4J_USER="$NEO4J_USER"
    KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
)

run_knot() {
    env "${KNOT_ENV[@]}" cargo run --release --bin knot -- "$@" 2>/dev/null
}

run_mcp() {
    echo "$1" | env "${KNOT_ENV[@]}" cargo run --release --bin knot-mcp 2>/dev/null | tail -1
}

# ── Test 1: Class & Method Extraction with Namespaces ─
echo -n "Test 1: FQN extraction for Engine::MyClass::start... "
if retry_match "Engine::MyClass::start" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'start'}) RETURN e.fqn"; then
    echo -e "${GREEN}✓ Engine::MyClass::start found${NC}"
else
    echo -e "${RED}✗ Engine::MyClass::start NOT found in FQNs${NC}"
    exit 1
fi

# ── Test 2: Call Graph for Pointers/Refs ─
echo -n "Test 2: find_callers on start() finds main()... "
if retry_match "main" run_knot callers start --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓ main found as caller${NC}"
else
    echo -e "${RED}✗ main NOT found as caller${NC}"
    exit 1
fi

# ── Test 3: Macro Tracking ─
echo -n "Test 3: macro MAX_BUF used in main()... "
if retry_match "main" run_knot callers MAX_BUF --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓ MAX_BUF usage found in main${NC}"
else
    echo -e "${RED}✗ MAX_BUF usage NOT found${NC}"
    exit 1
fi

# ── Test 4: Namespace and Class References ─
echo -n "Test 4: MyClass referenced in main()... "
if retry_match "main" run_knot callers MyClass --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓ MyClass reference found in main${NC}"
else
    echo -e "${RED}✗ MyClass reference NOT found${NC}"
    exit 1
fi

# ── Test 5: Operator Overload & Reference Return ─
echo -n "Test 5: operator = extracted with FQN... "
if retry_match "String::operator =" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'operator ='}) RETURN e.fqn"; then
    echo -e "${GREEN}✓ operator = extracted${NC}"
else
    echo -e "${RED}✗ operator = NOT found${NC}"
    exit 1
fi

echo -n "Test 6: copy() ref-return method extracted... "
if retry_match "String::copy" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'copy'}) RETURN e.fqn"; then
    echo -e "${GREEN}✓ ref-return method extracted${NC}"
else
    echo -e "${RED}✗ ref-return method NOT found${NC}"
    exit 1
fi

echo -n "Test 7: find_callers copy finds clear()... "
if retry_match "clear" run_knot callers copy --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓ clear() calls copy()${NC}"
else
    echo -e "${RED}✗ clear() NOT found as caller of copy()${NC}"
    exit 1
fi

echo -n "Test 8: find_callers copy finds operator =... "
if retry_match "operator" run_knot callers copy --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓ operator = calls copy()${NC}"
else
    echo -e "${RED}✗ operator = NOT found as caller of copy()${NC}"
    exit 1
fi

# ── Test 9: WString.cpp qualified definitions in Neo4j ─
echo -n "Test 9: String::copy in Neo4j... "
if retry_match "String::copy" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'copy'}) RETURN e.fqn"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ String::copy NOT found${NC}"
    exit 1
fi

echo -n "Test 9: String::operator = in Neo4j... "
if retry_match "String::operator =" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'operator ='}) RETURN e.fqn"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ String::operator = NOT found${NC}"
    exit 1
fi

echo -n "Test 9: operator = CALLS copy edge in Neo4j... "
if retry_match "CALLS" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (c:Entity {name:'operator ='})-[r:CALLS]->(p:Entity {name:'copy'}) RETURN r"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ operator = -[CALLS]-> copy NOT found${NC}"
    exit 1
fi

echo -n "Test 10: Overload resolution - 3 write() overloads in Neo4j... "
if retry_count 3 "Printer::write" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'write'}) RETURN e.fqn"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ Expected 3 write overloads in Neo4j${NC}"
    exit 1
fi

echo -n "Test 10: Overload resolution - find_callers write... "
if retry_match "printf" run_knot callers write --repo "$REPO_NAME" && retry_match "test_overloads" run_knot callers write --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ find_callers write missing expected callers${NC}"
    exit 1
fi

echo -n "Test 10: Overload resolution - printf CALLS 2-arg write... "
if retry_match "CALLS" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (c:Entity {name:'printf'})-[r:CALLS]->(e:Entity {name:'write'}) WHERE c.fqn CONTAINS 'Printer' RETURN r"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ printf not calling write overload${NC}"
    exit 1
fi

echo -n "Test 10: Overload resolution - write(char*) CALLS write (any overload)... "
if retry_match "CALLS" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (c:Entity)-[r:CALLS]->(e:Entity) WHERE c.name='write' AND e.name='write' AND c.fqn CONTAINS 'Printer' RETURN r"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ write(char*) not calling another write overload${NC}"
    exit 1
fi

echo -n "Test 10: Overload resolution - test_overloads CALLS write (any overload)... "
if retry_match "CALLS" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (c:Entity {name:'test_overloads'})-[r:CALLS]->(e:Entity {name:'write'}) RETURN r"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ test_overloads not calling write${NC}"
    exit 1
fi

# ── Test 11: Signature-based overload disambiguation ─
echo -n "Test 11: Signature stored for C++ write overloads... "
if retry_match "write" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'write'}) WHERE e.signature IS NOT NULL RETURN e.name"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ No write entities have signature stored${NC}"
    exit 1
fi

echo -n "Test 11: Signature contains parameter info (const uint8_t)... "
if retry_match "Printer::write" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity {name: 'write', fqn: 'Printer::write'}) WHERE e.signature CONTAINS 'const uint8_t' RETURN e.fqn"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ No write signature contains 'const uint8_t'${NC}"
    exit 1
fi

echo -n "Test 11: find_callers with signature fragment finds printf... "
if retry_match "printf" run_knot callers "write(const uint8_t" --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ find_callers 'write(const uint8_t' did not find printf${NC}"
    exit 1
fi

echo -n "Test 11: find_callers write returns at least 2 callers (printf, write(char*))... "
if retry_match "printf" run_knot callers write --repo "$REPO_NAME" && retry_match "| write " run_knot callers write --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ find_callers write missing expected callers${NC}"
    exit 1
fi

# ── Test 12: Overload isolation via arg_count disambiguation ─
echo -n "Test 12: find_callers 'write(const uint8_t' excludes test_overloads..."
if retry_match "printf" run_knot callers "write(const uint8_t" --repo "$REPO_NAME" && ! run_knot callers "write(const uint8_t" --repo "$REPO_NAME" | grep -q "test_overloads"; then
    echo -e "${GREEN}✓ test_overloads correctly excluded${NC}"
else
    echo -e "${RED}✗ test_overloads should NOT appear for write(const uint8_t)${NC}"
    exit 1
fi

echo -n "Test 12: printf CALLS the 2-arg write (const uint8_t* signature)... "
if retry_match "CALLS" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (c:Entity {name:'printf'})-[r:CALLS]->(e:Entity {name:'write', fqn:'Printer::write'}) WHERE e.signature CONTAINS 'const uint8_t' RETURN r"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ printf does not call write(const uint8_t*)${NC}"
    exit 1
fi

echo -n "Test 12: test_overloads CALLS the 1-arg write (NOT const uint8_t*)... "
# Wait for stabilization, then ensure 0 CALLS to the 2-arg version exist
sleep 2
if ! docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (c:Entity {name:'test_overloads'})-[r:CALLS]->(e:Entity {name:'write', fqn:'Printer::write'}) WHERE e.signature CONTAINS 'const uint8_t' RETURN r" 2>/dev/null | grep -q "CALLS"; then
    echo -e "${GREEN}✓ test_overloads calls 1-arg write, not const uint8_t* version${NC}"
else
    echo -e "${RED}✗ test_overloads should call write(uint8_t), not write(const uint8_t*)${NC}"
    exit 1
fi

# ── Test 13: Print.cpp entities extracted with correct FQN ─
echo -n "Test 13: Print.cpp entities extracted (Print::print and Print::println)... "
if retry_count 3 "Print::print" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password "MATCH (e:Entity) WHERE e.fqn = 'Print::print' AND e.file_path CONTAINS 'Print.cpp' RETURN e.fqn"; then
    echo -e "${GREEN}✓ (print overloads found)${NC}"
else
    echo -e "${RED}✗ Expected Print::print entities in Print.cpp not found${NC}"
    exit 1
fi

# ── Test 14: println CALLS print (internal cross-method call) ─
echo -n "Test 14: println CALLS print in Print.cpp... "
if retry_match "CALLS" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password \
    "MATCH (c:Entity {name:'println'})-[r:CALLS]->(p:Entity {name:'print'}) \
     WHERE c.file_path CONTAINS 'Print.cpp' AND p.fqn = 'Print::print' \
     RETURN r"; then
    echo -e "${GREEN}✓ (CALLS edges found)${NC}"
else
    echo -e "${RED}✗ println does NOT call print in Print.cpp${NC}"
    exit 1
fi

# ── Test 15: find_callers print includes println ─
echo -n "Test 15: find_callers print includes println from Print.cpp... "
if retry_match "println" run_knot callers print --repo "$REPO_NAME"; then
    echo -e "${GREEN}✓${NC}"
else
    echo -e "${RED}✗ find_callers print did not find println${NC}"
    exit 1
fi

# ── Test 16: Print.h parsed correctly (class Print extracted) ─
echo -n "Test 16: Print class extracted from Print.h... "
if retry_match "cpp_class" docker exec knot_neo4j_cpp_e2e cypher-shell -u neo4j -p e2e_test_password \
    "MATCH (e:Entity {name:'Print'}) WHERE e.file_path CONTAINS 'Print.h' RETURN e.kind"; then
    echo -e "${GREEN}✓ Print.h parsed as C++ (CppClass found)${NC}"
else
    echo -e "${RED}✗ Print class NOT extracted from Print.h (may be parsed as C instead of C++)${NC}"
    exit 1
fi

echo -e "\n${GREEN}✓ All C/C++ E2E tests passed!${NC}"
