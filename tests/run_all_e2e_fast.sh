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

# All 16 suites now use the shared DB. Order matches CI.
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
)

# Pre-flight: ensure shared Neo4j port (17687) is free before starting.
if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':17687 '; then
    echo -e "${YELLOW}Port 17687 still in use, forcing teardown before suite...${NC}"
    cd "$TESTS_DIR"
    docker compose -f docker-compose.e2e.yml down -v --remove-orphans 2>/dev/null || true
    sleep 5
    cd "$PROJECT_ROOT"
fi

# Build binaries once
echo -e "\n${YELLOW}Building knot binaries...${NC}"
cargo build --release --bin knot-indexer --bin knot --bin knot-mcp 2>&1 | grep -E "(Compiling knot|Finished)" || true
echo -e "${GREEN}✓ Build complete${NC}"

# Single shared-DB startup
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
    local timeout=60

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
            if nc -z localhost "$port" 2>/dev/null; then
                echo ""
                echo -e "${GREEN}✓ $service is ready on port $port${NC}"
                return 0
            fi
        fi
        if [ $elapsed -ge $timeout ]; then
            echo ""
            echo -e "${RED}ERROR: $service did not start within ${timeout}s${NC}"
            return 1
        fi
        sleep 2
        elapsed=$((elapsed + 2))
        echo -n "."
    done
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

# Run all 16 suites sequentially against the same live DB.
# --clean is repo-scoped in the indexer, so each suite only wipes its own
# repo's data — earlier suites' data is preserved.
echo -e "\n${BLUE}── Running all 16 suites against shared DB ──${NC}"
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
