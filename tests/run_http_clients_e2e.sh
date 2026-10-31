#!/usr/bin/env bash
# E2E Integration Test Script for HTTP client → endpoint linking (issue #46)
#
# Three repositories with no build dependency between them
# (testing_files/http_clients/): `shop-client` calls `items-server` and
# `billing-server` through declarative HTTP clients (OpenFeign, MicroProfile
# REST Client, Spring HTTP interface). The client is indexed FIRST, so the
# HTTP_CALLS edges must appear when the servers are indexed later.
#
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes shop-client, then items-server, then billing-server
# 3. Verifies `find_callers` on each handler lists its HTTP callers (CLI + MCP)
# 4. Verifies ambiguous and unrelated endpoints get no HTTP caller
# 5. Re-indexes the client (no changes) and verifies the links are stable
# 6. Re-indexes the three repositories concurrently and verifies the links converge
#
# Usage: ./tests/run_http_clients_e2e.sh
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
FIXTURES_DIR="$SCRIPT_DIR/testing_files/http_clients"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_http_clients_data"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_http_clients_e2e_test"

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot HTTP client linking E2E Test (#46)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}HTTP client E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up HTTP client E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/6] Starting Docker containers...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/6] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set; expecting shared DB)${NC}"
fi

if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[2/6] Skipping wait (KNOT_E2E_EXTERNAL_DB set; orchestrator manages readiness)${NC}"
else
    echo -e "${YELLOW}[2/6] Waiting for services to be ready...${NC}"

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

cd "$PROJECT_ROOT"
echo "Building knot-indexer, knot and knot-mcp..."
if [[ -z "${KNOT_SKIP_BUILD:-}" ]]; then
    cargo build --release --bin knot-indexer --bin knot --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
fi

index_repo() {
    local repo=$1
    shift
    echo "Indexing $repo..."
    KNOT_REPO_PATH="$FIXTURES_DIR/$repo" KNOT_REPO_NAME="$repo" \
        ./target/release/knot-indexer "$@" > /dev/null 2>&1
    echo -e "${GREEN}✓ $repo indexed${NC}"
}

# The client goes first: the links must appear once the servers exist.
echo -e "${YELLOW}[3/6] Indexing shop-client, items-server, billing-server...${NC}"
index_repo shop-client --clean
index_repo items-server --clean
index_repo billing-server --clean

# `<handler FQN>` → its HTTP callers, as `<client file>:<client call name>`.
http_callers_cli() {
    ./target/release/knot callers "$1" -r all -o json 2>/dev/null \
        | jq -r '(.http_calls // [])[] | "\(.file_path | split("/") | last):\(.name)"' \
        | LC_ALL=C sort -u | paste -sd, -
}

http_callers_mcp() {
    local request="{\"jsonrpc\":\"2.0\",\"id\":$2,\"method\":\"tools/call\",\"params\":{\"name\":\"find_callers\",\"arguments\":{\"entity_name\":\"$1\",\"repo_name\":\"all\"}}}"
    echo "$request" | KNOT_REPO_PATH="$FIXTURES_DIR/shop-client" ./target/release/knot-mcp 2>/dev/null | tail -n 1 \
        | jq -r '.result.content[0].text' \
        | awk '/^## /{ on = ($0 ~ /^## HTTP callers/) } on && /^- \*\*`/' \
        | sed -E 's/^- \*\*`([^`]*)`.* at `([^`:]*\/)?([^`/:]*):[0-9]+`.*/\3:\1/' \
        | LC_ALL=C sort -u | paste -sd, -
}

assert_http_callers() {
    local id=$1
    local fqn=$2
    local expected=$3
    local description=$4

    echo ""
    echo "Test $id: $description ($fqn HTTP callers exactly [$expected])..."
    local cli_actual
    cli_actual=$(http_callers_cli "$fqn")
    if [ "$cli_actual" != "$expected" ]; then
        echo -e "${RED}✗ CLI: expected [$expected], got [$cli_actual]${NC}"
        ./target/release/knot callers "$fqn" -r all -o markdown 2>/dev/null || true
        exit 1
    fi
    echo -e "${GREEN}✓ CLI: [$cli_actual]${NC}"
    local mcp_actual
    mcp_actual=$(http_callers_mcp "$fqn" "$id")
    if [ "$mcp_actual" != "$expected" ]; then
        echo -e "${RED}✗ MCP: expected [$expected], got [$mcp_actual]${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ MCP: [$mcp_actual]${NC}"
}

run_link_assertions() {
    assert_http_callers "${1}01" "com.example.items.ItemsController.read" \
        "ItemsClient.java:GET /api/items/{id}" \
        "Feign client found by service name, context path read from application.yml"
    assert_http_callers "${1}02" "com.example.items.ItemsController.create" \
        "ItemsRestClient.java:POST /api/items" \
        "MicroProfile client whose base URL comes from the configuration"
    assert_http_callers "${1}03" "com.example.billing.BillingController.createInvoice" \
        "BillingClient.java:POST /billing/invoices" \
        "Feign url placeholder resolved; context path read from application.properties"
    assert_http_callers "${1}04" "com.example.items.HealthController.health" "" \
        "An endpoint served by two services gets no link"
    assert_http_callers "${1}05" "com.example.billing.BillingController.health" "" \
        "The other side of the ambiguous call gets no link either"
    assert_http_callers "${1}06" "com.example.items.ItemsController.export" "" \
        "A more specific sibling endpoint never steals the call"
    assert_http_callers "${1}07" "com.example.items.ItemsController.stock" \
        "StockGateway.java:GET /api/items/{id}/stock" \
        "RestTemplate URL built on an @Value field; the test file's call is not linked"
    assert_http_callers "${1}08" "com.example.items.ItemsController.delete" \
        "StockGateway.java:DELETE /api/items/{id}" \
        "RestTemplate.delete with a variable segment"
    assert_http_callers "${1}09" "com.example.items.ItemsController.update" \
        "InventoryClient.kt:PUT /api/items/{id}" \
        "Kotlin RestClient whose unknown base URL is treated as the host"
    assert_http_callers "${1}10" "com.example.billing.BillingController.readInvoice" \
        "BillingWebClient.java:GET /billing/invoices/{number}" \
        "WebClient base URL read from the builder of the same file"
    assert_http_callers "${1}11" "com.example.billing.BillingController.refund" \
        "BillingWebClient.java:POST /billing/refunds" \
        "WebClient URI built by a uriBuilder lambda"
}

echo -e "${YELLOW}[4/6] Validating HTTP callers...${NC}"
run_link_assertions 1

echo ""
echo "Test 401: the client method keeps its own callers; explore_file lists the client calls..."
CHECKOUT=$(./target/release/knot callers "com.example.shop.ItemsClient.fetch" -r all -o json 2>/dev/null \
    | jq -r '[.calls[]?.name] | join(",")')
if [ "$CHECKOUT" != "checkout" ]; then
    echo -e "${RED}✗ CLI: ItemsClient.fetch callers expected [checkout], got [$CHECKOUT]${NC}"
    exit 1
fi
EXPLORE=$(./target/release/knot explore "src/main/java/com/example/shop/ItemsClient.java" -r shop-client -o markdown 2>/dev/null)
if ! echo "$EXPLORE" | grep -q "## HTTP Client Calls" || ! echo "$EXPLORE" | grep -qF 'GET /api/items/{id}'; then
    echo -e "${RED}✗ CLI: explore_file misses the HTTP Client Calls section:${NC}"
    echo "$EXPLORE"
    exit 1
fi
echo -e "${GREEN}✓ CLI: checkout → ItemsClient.fetch; ItemsClient.java lists GET /api/items/{id}${NC}"

echo ""
echo "Test 402: imperative calls are listed in their file; Map.put / Map.remove are not HTTP calls..."
STOCK_CALLS=$(./target/release/knot explore "src/main/java/com/example/shop/StockGateway.java" -r shop-client -o json 2>/dev/null \
    | jq -r '[.. | objects | select(.kind? == "http_client_call") | .name] | sort | join(",")')
if [ "$STOCK_CALLS" != "DELETE /api/items/{id},GET /api/items/{id}/stock" ]; then
    echo -e "${RED}✗ CLI: StockGateway.java client calls expected [DELETE /api/items/{id},GET /api/items/{id}/stock], got [$STOCK_CALLS]${NC}"
    ./target/release/knot explore "src/main/java/com/example/shop/StockGateway.java" -r shop-client -o markdown 2>/dev/null || true
    exit 1
fi
echo -e "${GREEN}✓ CLI: [$STOCK_CALLS]${NC}"

echo -e "${YELLOW}[5/6] Re-indexing shop-client (no changes): links must be stable...${NC}"
index_repo shop-client
run_link_assertions 2

# Every run recomputes the links of the whole graph: concurrent runs (cluster
# nodes sharing Neo4j) must serialize, so the last one sees every repository.
echo -e "${YELLOW}[6/6] Re-indexing the three repositories concurrently (--clean): links must converge...${NC}"
for round in 1 2; do
    index_repo shop-client --clean &
    client_pid=$!
    index_repo items-server --clean &
    items_pid=$!
    index_repo billing-server --clean &
    billing_pid=$!
    wait "$client_pid" "$items_pid" "$billing_pid"
    echo "Round $round done"
done
run_link_assertions 3

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All HTTP client E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated HTTP client linking features:"
echo "  ✓ OpenFeign, MicroProfile REST Client and Spring HTTP interfaces"
echo "  ✓ RestTemplate, WebClient and RestClient calls (Java and Kotlin), test code excluded"
echo "  ✓ Base URLs and placeholders resolved from application.yml / .properties"
echo "  ✓ Context paths read from the server configuration (optional in the client path)"
echo "  ✓ Service-name narrowing; ambiguous calls are not linked"
echo "  ✓ Links independent of indexing order and stable on re-index"
echo "  ✓ Concurrent indexer runs converge to the same links"
echo ""

exit 0
