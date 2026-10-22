#!/usr/bin/env bash
# E2E Integration Test Script for Java Support in knot
#
# This script tests Java-specific features:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes Java test file (test_java.java)
# 3. Queries via MCP to validate Java entity extraction
# 4. Tests class, interface, annotation, FQN resolution, and inheritance
# 5. Cleans up containers and data
#
# Usage: ./tests/run_java_e2e.sh
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
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/java"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_java_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_java_e2e_test"
REPO_NAME="java_e2e_test_repo"

# Timeout settings
TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

# Export common env for child cargo invocations
export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"
export KNOT_REPO_PATH="$TEST_FILES_DIR"
export KNOT_REPO_NAME="$REPO_NAME"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Java E2E Integration Test${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Java E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Java E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

# Register cleanup on script exit
trap cleanup EXIT INT TERM

# Step 1: Start Docker containers (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/5] Starting Docker containers for Java E2E test...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/5] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set; expecting shared DB)${NC}"
fi

# Step 2: Wait for services to be ready (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[2/5] Skipping wait (KNOT_E2E_EXTERNAL_DB set; orchestrator manages readiness)${NC}"
else
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
fi

# Step 3: Index Java test file
echo -e "${YELLOW}[3/5] Indexing Java test file (test_java.java)...${NC}"
cd "$PROJECT_ROOT"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for Java files..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"

echo -e "${GREEN}✓ Java file indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating Java entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Test 1: Search for UserService in Java
echo ""
echo "Test 1: Searching for UserService in Java files..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"UserService\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "UserService" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "UserService"; then
    echo -e "${GREEN}✓ MCP: Found UserService in search results${NC}"
else
    echo -e "${RED}✗ MCP: UserService not found${NC}"
    exit 1
fi

if echo "$CLI_RESPONSE" | grep -q "UserService"; then
    echo -e "${GREEN}✓ CLI: Found UserService in search results${NC}"
else
    echo -e "${RED}✗ CLI: UserService not found${NC}"
    exit 1
fi

# Test 2: Java package FQN resolution
echo ""
echo "Test 2: Verifying Java FQN includes package name..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"UserService\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
if echo "$MCP_RESPONSE" | grep -q "com.example.knot.test.UserService"; then
    echo -e "${GREEN}✓ FQN includes package prefix com.example.knot.test${NC}"
else
    echo -e "${RED}✗ FQN missing package prefix${NC}"
    exit 1
fi

# Test 3: Java field_access and FQN resolution for method calls
echo ""
echo "Test 3: Searching for callers of ChatMemory.add..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"ChatMemory.add\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "ChatMemory.add" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "before" || echo "$CLI_RESPONSE" | grep -q "before"; then
    echo -e "${GREEN}✓ Found ChatMemoryAdvisor.before calling ChatMemory.add${NC}"
else
    echo -e "${RED}✗ ChatMemoryAdvisor.before calling ChatMemory.add not found${NC}"
    exit 1
fi

# Test 4: Java method signature search with parameter types
echo ""
echo "Test 4: Searching for Java method by full signature registerUser(String..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"registerUser(String\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "registerUser(String" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "createUser" || echo "$CLI_RESPONSE" | grep -q "createUser"; then
    echo -e "${GREEN}✓ Found callers of registerUser by signature${NC}"
else
    echo -e "${RED}✗ Signature-based search for registerUser failed${NC}"
    exit 1
fi

# Test 5: Java inheritance — IMPLEMENTS edge (LoggingHandler → MessageHandler)
echo ""
echo "Test 5: Verifying Java IMPLEMENTS edge for LoggingHandler → MessageHandler..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"MessageHandler\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "MessageHandler" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "LoggingHandler" || echo "$CLI_RESPONSE" | grep -q "LoggingHandler"; then
    echo -e "${GREEN}✓ Found IMPLEMENTS edge LoggingHandler → MessageHandler${NC}"
else
    echo -e "${RED}✗ Missing IMPLEMENTS edge LoggingHandler → MessageHandler${NC}"
    exit 1
fi

# Test 6: Java inheritance — IMPLEMENTS edge (UserRepository → Repository)
echo ""
echo "Test 6: Verifying Java IMPLEMENTS edge for UserRepository → Repository..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Repository\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "Repository" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "UserRepository" || echo "$CLI_RESPONSE" | grep -q "UserRepository"; then
    echo -e "${GREEN}✓ Found IMPLEMENTS edge UserRepository → Repository${NC}"
else
    echo -e "${RED}✗ Missing IMPLEMENTS edge UserRepository → Repository${NC}"
    exit 1
fi

# Test 7: Java inheritance — EXTENDS edge (AdminUser → User)
echo ""
echo "Test 7: Verifying Java EXTENDS edge for AdminUser → User..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"User\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "User" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "AdminUser" || echo "$CLI_RESPONSE" | grep -q "AdminUser"; then
    echo -e "${GREEN}✓ Found EXTENDS edge AdminUser → User${NC}"
else
    echo -e "${RED}✗ Missing EXTENDS edge AdminUser → User${NC}"
    exit 1
fi

# Test 8: Java interface extending interface (AuditableRepository → Repository)
echo ""
echo "Test 8: Verifying Java interface EXTENDS edge AuditableRepository → Repository..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"Repository\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "Repository" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "AuditableRepository" || echo "$CLI_RESPONSE" | grep -q "AuditableRepository"; then
    echo -e "${GREEN}✓ Found EXTENDS edge AuditableRepository → Repository${NC}"
else
    echo -e "${RED}✗ Missing EXTENDS edge AuditableRepository → Repository${NC}"
    exit 1
fi

# Test 9: Java anonymous class implements interface (handle invocation)
echo ""
echo "Test 9: Verifying anonymous class implementing interface (EventBroadcaster calls handle on MessageHandler)..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"handle\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "handle" 2>/dev/null)

if echo "$MCP_RESPONSE" | grep -q "sendMessage" || echo "$CLI_RESPONSE" | grep -q "sendMessage"; then
    echo -e "${GREEN}✓ Found caller sendMessage invoking handle on anonymous MessageHandler${NC}"
else
    echo -e "${RED}✗ Anonymous class handle() invocation not tracked${NC}"
    exit 1
fi

# Test 10: Java search_hybrid_context finds MessageHandler interface
echo ""
echo "Test 10: Verifying MessageHandler interface is searchable..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"MessageHandler\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
if echo "$MCP_RESPONSE" | grep -q "MessageHandler"; then
    echo -e "${GREEN}✓ MessageHandler interface found in search results${NC}"
else
    echo -e "${RED}✗ MessageHandler interface not found in search${NC}"
    exit 1
fi

# Test 11: Kind-aware ranking — the login method definition outranks the
# class, the helper and the markdown doc for a natural-language query.
echo ""
echo "Test 11: Recall — paraphrase of a doc-less definition reaches the top slots..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"authenticate user with email and password\",\"max_results\":8,\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(cargo run --release --bin knot -- search "authenticate user with email and password" -r "$REPO_NAME" -m 8 -o markdown 2>/dev/null)

# The `login` method definition must appear within the leading result
# slots (recall contract). Naming `login` verbatim keeps it at #1.
MCP_HEADERS=$(echo "$MCP_RESPONSE" | grep -o '## `[^`]*`' | head -5)
CLI_HEADERS=$(echo "$CLI_RESPONSE" | grep -o '## `[^`]*`' | head -5)

if echo "$MCP_HEADERS" | grep -q '`login`'; then
    echo -e "${GREEN}✓ MCP: login method definition reaches the top slots${NC}"
else
    echo -e "${RED}✗ MCP: expected login within top-5, got: $MCP_HEADERS${NC}"
    echo "Response was: $MCP_RESPONSE"
    exit 1
fi

if echo "$CLI_HEADERS" | grep -q '`login`'; then
    echo -e "${GREEN}✓ CLI: login method definition reaches the top slots${NC}"
else
    echo -e "${RED}✗ CLI: expected login within top-5, got: $CLI_HEADERS${NC}"
    exit 1
fi

# Tests 12-17: calls through interface-typed fields (issue #28).
# Fixture: testing_files/java/interface_fields/. Every caller name is unique
# in the repository, so its presence in the find_callers output of the
# interface method proves the CALLS edge was created against the interface.
assert_interface_field_caller() {
    local id=$1
    local entity=$2
    local caller=$3
    local description=$4

    echo ""
    echo "Test $id: $description ($caller → $entity)..."
    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$entity\",\"repo_name\":\"$REPO_NAME\"}}}"
    local mcp_response
    local cli_response
    mcp_response=$(echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
    cli_response=$(cargo run --release --bin knot -- callers "$entity" 2>/dev/null)

    if echo "$mcp_response" | grep -q "\*\*\`$caller\`\*\*"; then
        echo -e "${GREEN}✓ MCP: $caller listed as caller of $entity${NC}"
    else
        echo -e "${RED}✗ MCP: $caller missing from callers of $entity${NC}"
        echo "Response was: $mcp_response"
        exit 1
    fi

    if echo "$cli_response" | grep "Calls" | grep -q "$caller"; then
        echo -e "${GREEN}✓ CLI: $caller listed as caller of $entity${NC}"
    else
        echo -e "${RED}✗ CLI: $caller missing from callers of $entity${NC}"
        echo "Response was: $cli_response"
        exit 1
    fi
}

assert_interface_field_caller 12 "CatalogProvider.fetchItems" "buildWithMatchingField" \
    "Interface-typed field named after its type"
assert_interface_field_caller 13 "PreferenceService.store" "storeViaMatchingField" \
    "this.-qualified interface-typed field named after its type, default overload"
assert_interface_field_caller 14 "CatalogProvider.fetchItems" "buildWithRenamedField" \
    "Interface-typed field whose name differs from its type"
assert_interface_field_caller 15 "CatalogProvider.fetchItems" "buildWithThisRenamedField" \
    "this.-qualified interface-typed field whose name differs from its type"
assert_interface_field_caller 16 "CatalogProvider.fetchGroups" "scheduleGroupsViaMethodRef" \
    "Method reference on an interface-typed field"
assert_interface_field_caller 17 "PreferenceService.store" "storeViaRenamedField" \
    "Default overload called through a renamed interface-typed field"

# Test 18: Concrete implementation must NOT receive interface calls as direct Calls edges
echo ""
echo "Test 18: Verifying concrete implementation does not receive interface calls as direct Calls edges..."
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "HttpCatalogProvider.fetchItems" 2>/dev/null)
if echo "$CLI_RESPONSE" | grep "Calls" | grep -q "buildWithRenamedField"; then
    echo -e "${RED}✗ CLI: HttpCatalogProvider.fetchItems erroneously listed interface caller buildWithRenamedField under Calls${NC}"
    echo "Response was: $CLI_RESPONSE"
    exit 1
else
    echo -e "${GREEN}✓ CLI: HttpCatalogProvider.fetchItems does not claim interface caller under Calls${NC}"
fi

# Tests 19-27: interface dispatch on annotated Spring-style types (issue #29).
# Fixture: testing_files/java/interface_dispatch/. `DefaultLookupGateway`
# shares its package with `LookupGateway` and contains its name, so a loose
# substring match on `LookupGateway.listOptions` also hits the implementation.
assert_not_direct_caller() {
    local id=$1
    local entity=$2
    local caller=$3
    local description=$4

    echo ""
    echo "Test $id: $description ($caller must not call $entity directly)..."
    local cli_response
    cli_response=$(cargo run --release --bin knot -- callers "$entity" 2>/dev/null)
    if echo "$cli_response" | grep "Calls " | grep -q "$caller"; then
        echo -e "${RED}✗ CLI: $entity erroneously lists $caller under Calls${NC}"
        echo "Response was: $cli_response"
        exit 1
    fi
    echo -e "${GREEN}✓ CLI: $entity does not list $caller under Calls${NC}"
}

assert_override_relationship() {
    local id=$1
    local entity=$2
    local label=$3
    local related=$4
    local description=$5

    echo ""
    echo "Test $id: $description ($entity $label $related)..."
    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$entity\",\"repo_name\":\"$REPO_NAME\"}}}"
    local mcp_response
    local cli_response
    mcp_response=$(echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
    cli_response=$(cargo run --release --bin knot -- callers "$entity" 2>/dev/null)

    if echo "$cli_response" | grep "$label" | grep -q "$related"; then
        echo -e "${GREEN}✓ CLI: $entity $label $related${NC}"
    else
        echo -e "${RED}✗ CLI: missing '$label' row for $related on $entity${NC}"
        echo "Response was: $cli_response"
        exit 1
    fi

    if echo "$mcp_response" | grep -q "$label" && echo "$mcp_response" | grep -q "$related"; then
        echo -e "${GREEN}✓ MCP: $entity $label $related${NC}"
    else
        echo -e "${RED}✗ MCP: missing '$label' entry for $related on $entity${NC}"
        echo "Response was: $mcp_response"
        exit 1
    fi
}

assert_type_fqn() {
    local id=$1
    local type_name=$2
    local expected_fqn=$3

    echo ""
    echo "Test $id: Annotated type $type_name keeps the FQN $expected_fqn..."
    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"$type_name\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"
    local mcp_response
    mcp_response=$(echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

    if echo "$mcp_response" | grep -q "$expected_fqn.$type_name\b"; then
        echo -e "${RED}✗ MCP: $type_name FQN is doubled ($expected_fqn.$type_name)${NC}"
        echo "Response was: $mcp_response"
        exit 1
    fi
    if echo "$mcp_response" | grep -q "\*\*FQN:\*\* \`$expected_fqn\`"; then
        echo -e "${GREEN}✓ MCP: $type_name FQN is $expected_fqn${NC}"
    else
        echo -e "${RED}✗ MCP: FQN $expected_fqn not found for $type_name${NC}"
        echo "Response was: $mcp_response"
        exit 1
    fi
}

assert_type_fqn 19 "DefaultLookupGateway" "com.example.knot.idispatch.service.DefaultLookupGateway"
assert_type_fqn 20 "OrderLedger" "com.example.knot.idispatch.ledger.OrderLedger"
assert_interface_field_caller 21 "LookupGateway.listOptions" "optionsViaThisField" \
    "this.-qualified field on an annotated controller"
assert_interface_field_caller 22 "LookupGateway.listOptions" "optionsViaInheritedField" \
    "Field inherited from a superclass declared in another file"
assert_not_direct_caller 23 "DefaultLookupGateway.listOptions" "optionsViaInheritedField" \
    "Implementation whose name contains the interface name"
assert_interface_field_caller 24 "OrderLedger.appendEntry" "recordLedgerEntry" \
    "Annotated repository interface called through a field"
assert_override_relationship 25 "DefaultLookupGateway.listOptions" "Overrides" "listOptions" \
    "@Service implementation overrides the interface method"
assert_override_relationship 26 "LookupGateway.listOptions" "Overridden by" "listOptions" \
    "Annotated interface method is overridden by its implementation"
assert_override_relationship 27 "OrderLedger.appendEntry" "Overridden by" "appendEntry" \
    "@Repository interface method is overridden by its implementation"

# Tests 28-31: calls that reach the queried method only through dynamic
# dispatch are listed as "Dispatched calls", naming the method they are
# statically bound to; sibling implementations never leak.
assert_dispatched_caller() {
    local id=$1
    local entity=$2
    local caller=$3
    local bound=$4
    local description=$5

    echo ""
    echo "Test $id: $description ($caller reaches $entity via $bound)..."
    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$entity\",\"repo_name\":\"$REPO_NAME\"}}}"
    local mcp_response
    local cli_response
    mcp_response=$(echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)
    cli_response=$(cargo run --release --bin knot -- callers "$entity" 2>/dev/null)

    if echo "$cli_response" | grep "Dispatched call" | grep "\.$bound " | grep -q "$caller"; then
        echo -e "${GREEN}✓ CLI: $caller listed as dispatched caller of $entity (bound to $bound)${NC}"
    else
        echo -e "${RED}✗ CLI: $caller missing from dispatched callers of $entity${NC}"
        echo "Response was: $cli_response"
        exit 1
    fi

    if echo "$mcp_response" | grep -q "Dispatched calls" \
        && echo "$mcp_response" | grep -q "### Target: \`[^\`]*\.$bound\`" \
        && echo "$mcp_response" | grep -q "\*\*\`$caller\`\*\*"; then
        echo -e "${GREEN}✓ MCP: $caller listed as dispatched caller of $entity (bound to $bound)${NC}"
    else
        echo -e "${RED}✗ MCP: $caller missing from dispatched callers of $entity${NC}"
        echo "Response was: $mcp_response"
        exit 1
    fi
}

assert_dispatched_caller 28 "DefaultLookupGateway.listOptions" "optionsViaThisField" \
    "LookupGateway.listOptions" "Implementation reached through its interface"
assert_dispatched_caller 29 "DefaultLookupGateway.listOptions" "optionsViaInheritedField" \
    "LookupGateway.listOptions" "Implementation reached through an inherited interface field"
assert_dispatched_caller 30 "LookupGateway.listOptions" "optionsViaConcreteType" \
    "DefaultLookupGateway.listOptions" "Interface method reached through a concrete-typed call"

echo ""
echo "Test 31: Sibling implementations never leak into dispatched calls..."
CLI_RESPONSE=$(cargo run --release --bin knot -- callers "DefaultLookupGateway.listOptions" 2>/dev/null)
if echo "$CLI_RESPONSE" | grep -q "optionsViaCachedGateway"; then
    echo -e "${RED}✗ CLI: DefaultLookupGateway.listOptions lists optionsViaCachedGateway (bound to a sibling)${NC}"
    echo "Response was: $CLI_RESPONSE"
    exit 1
fi
echo -e "${GREEN}✓ CLI: sibling CachedLookupGateway callers are excluded${NC}"

# Tests 32-37: overload-aware OVERRIDES (issue #30).
# Fixture: testing_files/java/overload_overrides/. Overloads share their FQN,
# so the exact edge set is compared by declaration line: the CLI JSON rows as
# `<queried line>><related file>:<related line>`, the MCP section as the
# `<related file>:<related line>` of each bullet.
assert_override_set() {
    local id=$1
    local entity=$2
    local bucket=$3
    local heading=$4
    local expected=$5
    local description=$6

    echo ""
    echo "Test $id: $description ($entity $heading exactly $expected)..."
    local cli_actual
    cli_actual=$(cargo run --release --bin knot -- callers "$entity" -o json 2>/dev/null \
        | jq -r ".${bucket}[] | \"\(.target_start_line)>\(.file_path | split(\"/\") | last):\(.start_line)\"" \
        | LC_ALL=C sort -u | paste -sd, -)
    if [ "$cli_actual" = "$expected" ]; then
        echo -e "${GREEN}✓ CLI: $entity $heading $cli_actual${NC}"
    else
        echo -e "${RED}✗ CLI: $entity $heading expected '$expected', got '$cli_actual'${NC}"
        exit 1
    fi

    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$entity\",\"repo_name\":\"$REPO_NAME\"}}}"
    local mcp_expected
    local mcp_actual
    mcp_expected=$(echo "$expected" | tr ',' '\n' | sed 's/^[0-9]*>//' | LC_ALL=C sort -u | paste -sd, -)
    mcp_actual=$(echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1 \
        | jq -r '.result.content[0].text' \
        | awk -v h="## $heading" 'index($0, h) == 1 { on = 1; next } /^## / { on = 0 } on && /^- \*\*/' \
        | grep -o '[A-Za-z]*\.java:[0-9]*' | LC_ALL=C sort -u | paste -sd, -)
    if [ "$mcp_actual" = "$mcp_expected" ]; then
        echo -e "${GREEN}✓ MCP: $entity $heading $mcp_actual${NC}"
    else
        echo -e "${RED}✗ MCP: $entity $heading expected '$mcp_expected', got '$mcp_actual'${NC}"
        exit 1
    fi
}

assert_override_set 32 "DefaultSettingsService.applySettings" "overrides" "Overrides" \
    "10>SettingsService.java:14" \
    "Implementation overrides only the overload with matching arity"
assert_override_set 33 "SettingsService.applySettings" "overridden_by" "Overridden by" \
    "10>BaseSettingsService.java:13,14>DefaultSettingsService.java:10,14>TieredSettingsService.java:8" \
    "Each overload is overridden only by implementations with its arity"
assert_override_set 34 "TieredSettingsService.applySettings" "overrides" "Overrides" \
    "8>SettingsService.java:14" \
    "Walks past a supertype declaring only a different-arity overload"
assert_override_set 35 "PlainValueRenderer.renderValue" "overrides" "Overrides" \
    "7>ValueRenderer.java:8" \
    "Same-arity overloads are told apart by parameter type"
assert_override_set 36 "SettingEntryStore.storeEntry" "overrides" "Overrides" \
    "13>EntryStore.java:13,9>EntryStore.java:11" \
    "Type variables match concrete arguments, the most specific overload wins"

echo ""
echo "Test 37: Dispatched calls never cross into a different-arity overload..."
LEAKED=$(cargo run --release --bin knot -- callers "DefaultSettingsService.applySettings" -o json 2>/dev/null \
    | jq '[.dispatched_calls[] | select((.target_file_path | endswith("/SettingsService.java")) and .target_start_line == 10)] | length')
if [ "$LEAKED" != "0" ]; then
    echo -e "${RED}✗ CLI: DefaultSettingsService.applySettings lists $LEAKED dispatched call(s) bound to the one-parameter default overload${NC}"
    exit 1
fi
echo -e "${GREEN}✓ CLI: no dispatched call is bound to the one-parameter default overload${NC}"

# Tests 38-41: the search caller summary is built from the same resolved
# edges as find_callers (issue #31). Fixture: testing_files/java/caller_summary/.
# Every queried method has a same-named homonym elsewhere in the repository,
# so a name-based summary over-counts and leaks the homonym's callers.
search_row_cli() {
    local query=$1
    local fqn=$2
    cargo run --release --bin knot -- search "$query" -r "$REPO_NAME" -m 10 -o json 2>/dev/null \
        | jq -c --arg fqn "$fqn" '[.[] | select(.fqn == $fqn)] | first // empty'
}

search_section_mcp() {
    local id=$1
    local query=$2
    local fqn=$3
    local request="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"$query\",\"max_results\":10,\"repo_name\":\"$REPO_NAME\"}}}"
    echo "$request" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1 \
        | jq -r '.result.content[0].text' \
        | awk -v fqn="**FQN:** \`$fqn\`" '
            /^## `/ { if (hit) printf "%s", buf; buf = ""; hit = 0 }
            { buf = buf $0 "\n"; if (index($0, fqn) == 1) hit = 1 }
            END { if (hit) printf "%s", buf }'
}

# Count from a "**<label>:** N location(s)" line; an absent line means 0.
mcp_summary_count() {
    local section=$1
    local label=$2
    local count
    count=$(echo "$section" | grep -o "\*\*$label:\*\* [0-9]* location" | grep -o '[0-9]*' | head -1)
    echo "${count:-0}"
}

assert_search_summary_matches_callers() {
    local id=$1
    local query=$2
    local fqn=$3
    local expected_calls=$4
    local expected_dispatched=$5
    local foreign_file=$6

    echo ""
    echo "Test $id: Search caller summary of $fqn matches find_callers ($expected_calls direct, $expected_dispatched dispatched)..."

    # Ground truth: find_callers on the exact FQN.
    local callers_json
    callers_json=$(cargo run --release --bin knot -- callers "$fqn" -o json 2>/dev/null)
    local truth_targets truth_calls truth_dispatched
    truth_targets=$(echo "$callers_json" | jq '.resolution.targets | length')
    truth_calls=$(echo "$callers_json" | jq '.calls | length')
    truth_dispatched=$(echo "$callers_json" | jq '.dispatched_calls | length')
    if [ "$truth_targets" != "1" ] || [ "$truth_calls" != "$expected_calls" ] \
        || [ "$truth_dispatched" != "$expected_dispatched" ]; then
        echo -e "${RED}✗ find_callers: expected 1 target / $expected_calls direct / $expected_dispatched dispatched, got $truth_targets / $truth_calls / $truth_dispatched${NC}"
        echo "Response was: $callers_json"
        exit 1
    fi
    echo -e "${GREEN}✓ find_callers: $truth_calls direct, $truth_dispatched dispatched caller(s)${NC}"

    local row
    row=$(search_row_cli "$query" "$fqn")
    if [ -z "$row" ]; then
        echo -e "${RED}✗ CLI: $fqn not found in search results for '$query'${NC}"
        exit 1
    fi
    local cli_calls cli_dispatched
    cli_calls=$(echo "$row" | jq '.caller_count // 0')
    cli_dispatched=$(echo "$row" | jq '.dispatched_caller_count // 0')
    if [ "$cli_calls" != "$truth_calls" ] || [ "$cli_dispatched" != "$truth_dispatched" ]; then
        echo -e "${RED}✗ CLI: search summary says $cli_calls direct / $cli_dispatched dispatched, find_callers says $truth_calls / $truth_dispatched${NC}"
        echo "Row was: $row"
        exit 1
    fi
    if echo "$row" | jq -r '(.caller_samples // []) + (.dispatched_caller_samples // []) | .[]' | grep -q "$foreign_file"; then
        echo -e "${RED}✗ CLI: search summary of $fqn leaks a caller from $foreign_file${NC}"
        echo "Row was: $row"
        exit 1
    fi
    echo -e "${GREEN}✓ CLI: search summary matches find_callers, no caller from $foreign_file${NC}"

    local section
    section=$(search_section_mcp "$id" "$query" "$fqn")
    if [ -z "$section" ]; then
        echo -e "${RED}✗ MCP: $fqn not found in search results for '$query'${NC}"
        exit 1
    fi
    local mcp_calls mcp_dispatched
    mcp_calls=$(mcp_summary_count "$section" "Called by")
    mcp_dispatched=$(mcp_summary_count "$section" "Called via dispatch")
    if [ "$mcp_calls" != "$truth_calls" ] || [ "$mcp_dispatched" != "$truth_dispatched" ]; then
        echo -e "${RED}✗ MCP: search summary says $mcp_calls direct / $mcp_dispatched dispatched, find_callers says $truth_calls / $truth_dispatched${NC}"
        echo "Section was: $section"
        exit 1
    fi
    if echo "$section" | grep -q "$foreign_file"; then
        echo -e "${RED}✗ MCP: search summary of $fqn leaks a caller from $foreign_file${NC}"
        echo "Section was: $section"
        exit 1
    fi
    echo -e "${GREEN}✓ MCP: search summary matches find_callers, no caller from $foreign_file${NC}"
}

CSUMMARY_PKG="com.example.knot.csummary"
assert_search_summary_matches_callers 38 "compute the standard price adjustment value" \
    "$CSUMMARY_PKG.pricing.StandardAdjuster.computeValue" 1 0 "AlternateAdjuster.java"
assert_search_summary_matches_callers 39 "compute the alternate per-currency adjustment values" \
    "$CSUMMARY_PKG.pricing.AlternateAdjuster.computeValue" 2 0 "StandardAdjuster.java"
assert_search_summary_matches_callers 40 "persist a receipt into the long-term archive" \
    "$CSUMMARY_PKG.receipts.ReceiptArchive.persist" 0 1 "DraftCache.java"

# Test 41: the "Calls" list names each callee with its owning type, so the
# private helpers of homonym classes can be told apart.
echo ""
echo "Test 41: Search \"Calls\" list qualifies callees with their owning type..."
ADJUST_ROW=$(search_row_cli "standard adjuster adjust request" "$CSUMMARY_PKG.pricing.StandardAdjuster.adjust")
if [ -z "$ADJUST_ROW" ]; then
    echo -e "${RED}✗ CLI: StandardAdjuster.adjust not found in search results${NC}"
    exit 1
fi
ADJUST_DEPS=$(echo "$ADJUST_ROW" | jq -r '.dependencies | join(",")')
if [ "$ADJUST_DEPS" != "StandardAdjuster.computeValue" ]; then
    echo -e "${RED}✗ CLI: expected dependencies 'StandardAdjuster.computeValue', got '$ADJUST_DEPS'${NC}"
    exit 1
fi
echo -e "${GREEN}✓ CLI: StandardAdjuster.adjust calls StandardAdjuster.computeValue${NC}"

# Step 5: Summarize
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Java E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated Java features:"
echo "  ✓ Java class discovery (UserService)"
echo "  ✓ Java package FQN resolution (com.example.knot.test.*)"
echo "  ✓ Java FQN field-access resolution (ChatMemory.add)"
echo "  ✓ Java method signature search (registerUser(String...))"
echo "  ✓ Java class IMPLEMENTS interface (LoggingHandler → MessageHandler)"
echo "  ✓ Java class IMPLEMENTS interface (UserRepository → Repository)"
echo "  ✓ Java class EXTENDS class (AdminUser → User)"
echo "  ✓ Java interface EXTENDS interface (AuditableRepository → Repository)"
echo "  ✓ Java anonymous class interface implementation"
echo "  ✓ Java interface search (MessageHandler)"
echo "  ✓ Recall ranking (doc-less method definition reaches the top slots)"
echo "  ✓ Calls through interface-typed fields (any field name, this., method refs)"
echo "  ✓ Annotated types keep their FQN and link OVERRIDES (@Service, @Repository)"
echo "  ✓ Interface calls never bind to an implementation containing the interface name"
echo "  ✓ Dispatched calls in both directions, siblings excluded"
echo "  ✓ OVERRIDES match overloads by arity and parameter types (generics included)"
echo "  ✓ Search caller summary matches find_callers (homonyms, dispatched calls)"
echo ""

exit 0
