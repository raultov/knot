#!/usr/bin/env bash
# Fast E2E Test Suite — single shared DB lifecycle with full data coexistence.
#
# Implements both phases of plan_fast_e2e.md:
#   Phase 1: One docker-compose up/down for all 12 suites (saves ~12-15 min).
#   Phase 2: All suites use the shared DB; --clean is repo-scoped in the
#            indexer, so by the end of a successful run the DB contains live
#            data from every language simultaneously.
#
# Usage: ./tests/run_all_e2e_fast.sh

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TESTS_DIR="$PROJECT_ROOT/tests"
COMPOSE_FILE="$TESTS_DIR/docker-compose.e2e.yml"

# Share the ONNX fastembed model cache across all E2E suites
# so only the first suite downloads the model.
export KNOT_FASTEMBED_CACHE_DIR="${KNOT_FASTEMBED_CACHE_DIR:-$HOME/.cache/knot/fastembed_cache}"

# Signal child suites: shared DB is managed by this orchestrator; do NOT
# spin up or tear down docker-compose.e2e.yml.
export KNOT_E2E_EXTERNAL_DB=1

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot E2E Test Suite (Fast — Shared DB)${NC}"
echo -e "${BLUE}========================================${NC}"

FAILED_TESTS=()
PASSED_TESTS=()

# All 19 suites now use the shared DB. Order matches CI.
SUITES=(
    "run_typescript_e2e.sh"
    "run_java_e2e.sh"
    "run_javascript_e2e.sh"
    "run_web_e2e.sh"
    "run_kotlin_e2e.sh"
    "run_rust_e2e.sh"
    "run_rust_reference_resolution_e2e.sh"
    "run_python_e2e.sh"
    "run_rust_test_module_e2e.sh"
    "run_build_systems_e2e.sh"
    "run_config_e2e.sh"
    "run_k8s_helm_e2e.sh"
    "run_groovy_e2e.sh"
    "run_cross_repo_dep_e2e.sh"
    "run_cross_lang_ref_e2e.sh"
    "run_cpp_e2e.sh"
    "run_markdown_e2e.sh"
    "run_contains_autolink_index_e2e.sh"
    "run_varnish_e2e.sh"
)

# Pre-flight: free the e2e ports of any foreign container. Other e2e
# projects on this host (e.g. knot-server/tests/docker-compose.e2e.yml)
# publish the same high ports under their own container names; if any of
# those is still running, our containers will silently die on port-bind and
# `nc -z` will then talk to the wrong Qdrant/Neo4j.
OUR_E2E_CONTAINERS=(knot_qdrant_e2e knot_neo4j_e2e)
E2E_PORTS=(16333 16334 17474 17687)
stop_foreign_holders() {
    local stopped_any=0
    for port in "${E2E_PORTS[@]}"; do
        # `docker ps --filter publish=<port>` matches by host-port mapping.
        local holders
        holders=$(docker ps --filter "publish=$port" --format '{{.Names}}' 2>/dev/null || true)
        for name in $holders; do
            local skip=0
            for ours in "${OUR_E2E_CONTAINERS[@]}"; do
                if [ "$name" = "$ours" ]; then skip=1; break; fi
            done
            if [ "$skip" -eq 1 ]; then continue; fi
            echo -e "${YELLOW}Stopping foreign container '$name' holding e2e port $port...${NC}"
            docker stop "$name" >/dev/null 2>&1 || true
            stopped_any=1
        done
    done
    if [ "$stopped_any" -eq 1 ]; then sleep 3; fi
}
stop_foreign_holders

# If our own e2e containers are still around from a previous run, tear them down.
cd "$TESTS_DIR"
docker compose -f docker-compose.e2e.yml down -v --remove-orphans 2>/dev/null || true
cd "$PROJECT_ROOT"

# Build binaries once (unless caller provides pre-built binaries via KNOT_SKIP_BUILD=1)
if [ "${KNOT_SKIP_BUILD:-0}" = "1" ]; then
    echo -e "${YELLOW}KNOT_SKIP_BUILD=1 set — skipping cargo build; expecting pre-built binaries in target/release/${NC}"
else
    echo -e "\n${YELLOW}Building knot binaries...${NC}"
    cargo build --release --bin knot-indexer --bin knot --bin knot-mcp 2>&1 | grep -E "(Compiling knot|Finished)" || true
    echo -e "${GREEN}✓ Build complete${NC}"
fi

# Single shared-DB startup
echo -e "${YELLOW}Cleaning up stale .knot and .e2e_data directories...${NC}"
docker run --rm -v "$TESTS_DIR:/tests" alpine rm -rf /tests/.e2e_data 2>/dev/null || true
find "$PROJECT_ROOT/tests" -type d -name ".knot" -exec rm -rf {} + 2>/dev/null || true

echo -e "\n${YELLOW}Starting shared Neo4j + Qdrant (once for all suites)...${NC}"
cd "$TESTS_DIR"
docker compose -f docker-compose.e2e.yml down -v 2>/dev/null || true
docker compose -f docker-compose.e2e.yml up -d

# Wait for shared services to be ready
wait_for_port() {
    local port=$1
    local service=$2
    local container=$3
    local elapsed=0
    local timeout=120

    echo -n "Waiting for $service"
    while [ $elapsed -lt $timeout ]; do
        if [ "$service" = "Neo4j" ]; then
            local status
            status=$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null || echo "starting")
            if [ "$status" = "healthy" ]; then
                echo ""
                echo -e "${GREEN}✓ $service is ready (healthy)${NC}"
                return 0
            fi
        else
            # For Qdrant (and any non-Neo4j service), probe the actual port
            # instead of the container's Health.Status. Qdrant 1.16+ removed
            # the /health endpoint (returns 404), so the compose healthcheck
            # never reports healthy and the container stays in "starting".
            if nc -z localhost "$port" 2>/dev/null; then
                echo ""
                echo -e "${GREEN}✓ $service is ready on port $port${NC}"
                return 0
            fi
        fi
        sleep 2
        elapsed=$((elapsed + 2))
        echo -n "."
    done
    echo ""
    echo -e "${RED}ERROR: $service did not start within ${timeout}s${NC}"
    return 1
}

wait_for_port 17687 "Neo4j" "knot_neo4j_e2e" || { echo -e "${RED}Shared DB failed to start${NC}"; exit 1; }
wait_for_port 16334 "Qdrant" "knot_qdrant_e2e" || { echo -e "${RED}Shared DB failed to start${NC}"; exit 1; }
sleep 5
cd "$PROJECT_ROOT"

echo -e "${GREEN}✓ Shared DB ready${NC}"

run_suite() {
    local test_script="$1"

    echo -e "\n${YELLOW}[Running: $test_script]${NC}"
    echo -e "${YELLOW}========================================${NC}"

    if "$TESTS_DIR/$test_script"; then
        echo -e "${GREEN}✓ $test_script PASSED${NC}"
        PASSED_TESTS+=("$test_script")
    else
        echo -e "${RED}✗ $test_script FAILED${NC}"
        FAILED_TESTS+=("$test_script")
    fi
}

# Run all 19 suites sequentially against the same live DB.
# --clean is repo-scoped in the indexer, so each suite only wipes its own
# repo's data — earlier suites' data is preserved.
echo -e "\n${BLUE}── Running all 19 suites against shared DB ──${NC}"
for suite in "${SUITES[@]}"; do
    run_suite "$suite"
done

# Final teardown of shared DB
echo -e "\n${YELLOW}Stopping shared Neo4j + Qdrant...${NC}"
cd "$TESTS_DIR"
docker compose -f docker-compose.e2e.yml down -v 2>/dev/null || true
cd "$PROJECT_ROOT"
echo -e "${GREEN}✓ Shared DB stopped${NC}"

# Summary
echo -e "\n${BLUE}========================================${NC}"
echo -e "${BLUE}E2E Test Suite Summary (Fast)${NC}"
echo -e "${BLUE}========================================${NC}"

if [ ${#PASSED_TESTS[@]} -gt 0 ]; then
    echo -e "${GREEN}Passed (${#PASSED_TESTS[@]}):${NC}"
    for test in "${PASSED_TESTS[@]}"; do
        echo -e "  ${GREEN}✓${NC} $test"
    done
fi

if [ ${#FAILED_TESTS[@]} -gt 0 ]; then
    echo -e "\n${RED}Failed (${#FAILED_TESTS[@]}):${NC}"
    for test in "${FAILED_TESTS[@]}"; do
        echo -e "  ${RED}✗${NC} $test"
    done
    echo -e "\n${RED}Some E2E tests failed. Please fix them before committing.${NC}"
    exit 1
else
    echo -e "\n${GREEN}All E2E tests passed!${NC}"
    exit 0
fi
