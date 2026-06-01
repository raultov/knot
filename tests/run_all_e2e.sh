#!/usr/bin/env bash
set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# Share the ONNX fastembed model cache across all E2E suites
# so only the first suite downloads the model.
export KNOT_FASTEMBED_CACHE_DIR="${KNOT_FASTEMBED_CACHE_DIR:-$HOME/.cache/knot/fastembed_cache}"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot E2E Test Suite (All Languages)${NC}"
echo -e "${BLUE}========================================${NC}"

FAILED_TESTS=()
PASSED_TESTS=()

run_test() {
    local test_name="$1"
    local test_script="$2"

    # Pre-flight: ensure shared Neo4j port (17687) is free before starting.
    # If a previous suite left a container holding the port, force teardown.
    if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':17687 '; then
        echo -e "${YELLOW}Port 17687 still in use, forcing teardown before next suite...${NC}"
        cd "$PROJECT_ROOT/tests"
        docker compose -f docker-compose.e2e.yml down -v --remove-orphans 2>/dev/null || true
        sleep 5
        cd "$PROJECT_ROOT"
    fi

    echo -e "\n${YELLOW}[Running: $test_name]${NC}"
    echo -e "${YELLOW}========================================${NC}"

    if "$PROJECT_ROOT/tests/$test_script"; then
        echo -e "${GREEN}✓ $test_name PASSED${NC}"
        PASSED_TESTS+=("$test_name")
    else
        echo -e "${RED}✗ $test_name FAILED${NC}"
        FAILED_TESTS+=("$test_name")
    fi

    # Cleanup Docker between test suites
    echo -e "\n${YELLOW}Cleaning up Docker...${NC}"
    cd "$PROJECT_ROOT/tests"
    docker compose -f docker-compose.e2e.yml down -v 2>/dev/null || true
    docker compose -f .e2e_cpp_data/docker-compose.yml down -v 2>/dev/null || true
    docker compose -f .e2e_crosslang_data/docker-compose.yml down -v 2>/dev/null || true
    sudo rm -rf .e2e_* 2>/dev/null || rm -rf .e2e_* 2>/dev/null || true
    # Give Linux time to release TCP ports (TIME_WAIT) before next suite.
    sleep 6
    cd "$PROJECT_ROOT"
}

# Build binaries first
echo -e "\n${YELLOW}Building knot binaries...${NC}"
cargo build --release --bin knot-indexer --bin knot --bin knot-mcp 2>&1 | grep -E "(Compiling knot|Finished)" || true
echo -e "${GREEN}✓ Build complete${NC}"

# Run all E2E test suites in the same order as CI
run_test "JS/TS/Java E2E" "run_e2e.sh"
run_test "Kotlin E2E" "run_kotlin_e2e.sh"
run_test "Rust E2E" "run_rust_e2e.sh"
run_test "Python E2E" "run_python_e2e.sh"
run_test "Build Systems E2E" "run_build_systems_e2e.sh"
run_test "Config Files E2E" "run_config_e2e.sh"
run_test "K8s/Helm E2E" "run_k8s_helm_e2e.sh"
run_test "Groovy E2E" "run_groovy_e2e.sh"
run_test "Cross-Language Ref E2E" "run_cross_lang_ref_e2e.sh"
run_test "C/C++ E2E" "run_cpp_e2e.sh"
run_test "Cross-Repo Deps E2E" "run_cross_repo_dep_e2e.sh"

# Final cleanup
echo -e "\n${YELLOW}Final cleanup...${NC}"
cd "$PROJECT_ROOT/tests"
docker compose -f docker-compose.e2e.yml down -v 2>/dev/null || true
docker compose -f .e2e_cpp_data/docker-compose.yml down -v 2>/dev/null || true
docker compose -f .e2e_crosslang_data/docker-compose.yml down -v 2>/dev/null || true
rm -rf .e2e_* 2>/dev/null || true
cd "$PROJECT_ROOT"

# Summary
echo -e "\n${BLUE}========================================${NC}"
echo -e "${BLUE}E2E Test Suite Summary${NC}"
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
