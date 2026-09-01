#!/usr/bin/env bash
# E2E Integration Test Script for Repository Scope Selection (issue #19)
#
# Phase 0 BDD suite — written FIRST, must be RED before any production
# change lands. Maps 1:1 to §5 Gherkin scenarios in
# docs/specs/repo_scope_selection_plan.md.
#
# Group A — Sentinel:  all / ALL / * / omitted
# Group B — List:      union / restriction / whitespace / unknown / duplicates
# Group C — find_callers: all + list resolve both callers; single-repo guard
# Group D — explore_file: ambiguity without scope; resolution with scope
# Group E — JSON array form (MCP only)
# Group F — CLI parity:  --repo list / all / single / default
# Group G — Output labeling: every result row carries (repo: ...)
#
# Expected red: groups A / B / C (sentinel+list) / D (sentinel/array cases) /
# E / F (list/all cases) / G FAIL — current code treats "scope_alpha,scope_beta"
# and "all" as one literal repo name → 0 hits. Regression-guard scenarios
# (single repo, omitted repo) PASS — that documents current behavior.
#
# Usage: ./tests/run_repo_scope_e2e.sh
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
FIXTURE_ROOT="$SCRIPT_DIR/testing_files/repo_scope"

# Per-repo source paths (each indexer run uses one of these as KNOT_REPO_PATH)
ALPHA_REPO_DIR="$FIXTURE_ROOT/scope_alpha"
BETA_REPO_DIR="$FIXTURE_ROOT/scope_beta"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_repo_scope_data"

NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_repo_scope_e2e"

ALPHA_REPO_NAME="scope_alpha"
BETA_REPO_NAME="scope_beta"

TIMEOUT_SECONDS=60
HEALTH_CHECK_INTERVAL=2

export KNOT_NEO4J_URI="$NEO4J_URI"
export KNOT_NEO4J_USER="$NEO4J_USER"
export KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD"
export KNOT_QDRANT_URL="$QDRANT_URL"
export KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot Repository Scope E2E (Phase 0 BDD)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

FAILURES=0
PASSES=0

cleanup() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Repo scope E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi
    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi
    echo -e "\n${YELLOW}Cleaning up repo scope E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

pass() {
    local label="$1"
    echo -e "${GREEN}✓ $label${NC}"
    PASSES=$((PASSES + 1))
}

fail() {
    local label="$1"
    echo -e "${RED}✗ $label${NC}"
    FAILURES=$((FAILURES + 1))
}

assert_pass() {
    local label="$1" expected="$2"
    shift 2
    if "$@" | grep -qiE "$expected"; then
        pass "$label"
    else
        fail "$label (no match for /$expected/)"
    fi
}

assert_fail() {
    local label="$1" forbidden="$2"
    shift 2
    if "$@" | grep -qiE "$forbidden"; then
        fail "$label (forbidden match /$forbidden/ found)"
    else
        pass "$label"
    fi
}

# ── Start Docker ─────────────────────────────────────────────────────────────
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/5] Starting Docker containers for repo scope E2E test...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    # Also wipe any stale .knot state inside the fixtures
    rm -rf "$ALPHA_REPO_DIR/.knot" "$BETA_REPO_DIR/.knot" 2>/dev/null || true
    docker compose -f "$COMPOSE_FILE" up -d
else
    echo -e "${YELLOW}[1/5] Skipping Docker start (KNOT_E2E_EXTERNAL_DB set)${NC}"
fi

# ── Wait for services ─────────────────────────────────────────────────────────
if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[2/5] Skipping wait (KNOT_E2E_EXTERNAL_DB set)${NC}"
else
    echo -e "${YELLOW}[2/5] Waiting for services to be ready...${NC}"

    wait_for_port() {
        local port=$1 service=$2 container=$3 elapsed=0
        echo -n "Waiting for $service"
        while true; do
            if [ "$service" = "Neo4j" ]; then
                local status
                status=$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null || echo "starting")
                if [ "$status" = "healthy" ]; then
                    echo ""; echo -e "${GREEN}✓ $service is ready (healthy)${NC}"; return 0
                fi
            else
                if nc -z localhost "$port" 2>/dev/null; then
                    echo ""; echo -e "${GREEN}✓ $service is ready on port $port${NC}"; return 0
                fi
            fi
            if [ $elapsed -ge $TIMEOUT_SECONDS ]; then
                echo ""; echo -e "${RED}ERROR: $service did not start within ${TIMEOUT_SECONDS}s${NC}"; return 1
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

# ── Build binaries ───────────────────────────────────────────────────────────
echo -e "${YELLOW}[3/5] Building knot binaries...${NC}"
cd "$PROJECT_ROOT"

if [[ -z "${KNOT_SKIP_BUILD:-}" ]]; then
    cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true
    cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
    cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true
fi

# ── Helpers (copied from run_groovy_e2e.sh L137-198) ─────────────────────────
run_indexer() {
    local repo="$1" repo_dir="$2" clean_flag="${3:-}"
    echo -n "Indexing $repo... "
    env \
        KNOT_REPO_PATH="$repo_dir" \
        KNOT_REPO_NAME="$repo" \
        KNOT_NEO4J_URI="$NEO4J_URI" \
        KNOT_NEO4J_USER="$NEO4J_USER" \
        KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
        KNOT_QDRANT_URL="$QDRANT_URL" \
        KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
        ./target/release/knot-indexer $clean_flag 2>/dev/null
    echo -e "${GREEN}✓${NC}"
}

call_mcp() {
    local request="$1"
    echo "$request" | env \
        KNOT_NEO4J_URI="$NEO4J_URI" \
        KNOT_NEO4J_USER="$NEO4J_USER" \
        KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
        KNOT_QDRANT_URL="$QDRANT_URL" \
        KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
        ./target/release/knot-mcp 2>/dev/null | tail -n 1
}

call_cli() {
    local cwd="$1"
    shift
    ( cd "$cwd" && env \
        KNOT_NEO4J_URI="$NEO4J_URI" \
        KNOT_NEO4J_USER="$NEO4J_USER" \
        KNOT_NEO4J_PASSWORD="$NEO4J_PASSWORD" \
        KNOT_QDRANT_URL="$QDRANT_URL" \
        KNOT_QDRANT_COLLECTION="$QDRANT_COLLECTION" \
        "$PROJECT_ROOT/target/release/knot" "$@" ) 2>/dev/null
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

# ── Index both repos into the SAME collection ────────────────────────────────
echo -e "${YELLOW}[4/5] Indexing scope_alpha and scope_beta into shared collection...${NC}"

INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
run_indexer "$ALPHA_REPO_NAME" "$ALPHA_REPO_DIR" "${INDEXER_FLAGS[*]}"

# Reset INDEXER_FLAGS for second index (no --clean on subsequent repo)
run_indexer "$BETA_REPO_NAME" "$BETA_REPO_DIR" ""

# Single sleep after the second indexing for graph eventual consistency
sleep 5

# ── Assertions (groups A through G) ──────────────────────────────────────────
echo -e "${YELLOW}[5/5] Running assertions...${NC}"

# Assertion block must not abort on first failure — the FAILURES counter
# drives the exit code. The CLI may legitimately return non-zero (e.g.
# argument errors when --repo is passed an unrecognized literal), and we
# want each assertion to be evaluated independently.
set +e

echo ""
echo -e "${BLUE}── Group A: Sentinel (all / ALL / * / omitted) ──${NC}"

# Discriminator: BetaSearchService is unique to scope_beta. Each sentinel
# scenario expects BetaSearchService to appear when the scope widens to
# include scope_beta; with the current code it doesn't (the literal token
# is treated as a repo name → 0 hits), so these scenarios go RED.

# A1: search "BetaSearchService" with repo_name "all" → must appear
REQUEST_ALL='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"all"}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_ALL"; then
    pass "A1. MCP search with repo_name 'all' finds BetaSearchService (scope_beta)"
else
    fail "A1. MCP search with repo_name 'all' (expected BetaSearchService from scope_beta)"
fi

# A2: case-insensitive "ALL"
REQUEST_ALL_UPPER='{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"ALL"}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_ALL_UPPER"; then
    pass "A2. MCP search with repo_name 'ALL' (case-insensitive) finds BetaSearchService"
else
    fail "A2. MCP search with repo_name 'ALL' (expected BetaSearchService)"
fi

# A3: star sentinel "*"
REQUEST_STAR='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"*"}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_STAR"; then
    pass "A3. MCP search with repo_name '*' (star sentinel) finds BetaSearchService"
else
    fail "A3. MCP search with repo_name '*' (expected BetaSearchService)"
fi

# A4: star wins over list ("scope_alpha,*")
REQUEST_STAR_LIST='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"scope_alpha,*"}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_STAR_LIST"; then
    pass "A4. MCP search with 'scope_alpha,*' (star wins) finds BetaSearchService"
else
    fail "A4. MCP search with 'scope_alpha,*' (star wins, expected BetaSearchService)"
fi

# A5: omitted repo_name searches across all repos (regression guard — passes today)
REQUEST_OMITTED='{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService"}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_OMITTED"; then
    pass "A5. MCP search without repo_name finds BetaSearchService (regression guard)"
else
    fail "A5. MCP search without repo_name (regression guard, expected BetaSearchService)"
fi

echo ""
echo -e "${BLUE}── Group B: Comma-separated list ──${NC}"

# B1: comma list unions the listed repos — search for BetaSearchService
REQUEST_LIST='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"scope_alpha,scope_beta"}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_LIST"; then
    pass "B1. MCP search with 'scope_alpha,scope_beta' list finds BetaSearchService"
else
    fail "B1. MCP search with 'scope_alpha,scope_beta' list (expected BetaSearchService)"
fi

# B2: single repo "scope_alpha" — must NOT include BetaSearchService (regression guard)
REQUEST_ALPHA_ONLY='{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"scope_alpha"}}}'
RESP_ALPHA_ONLY=$(call_mcp "$REQUEST_ALPHA_ONLY")
if echo "$RESP_ALPHA_ONLY" | grep -qiE "BetaSearchService"; then
    fail "B2. MCP search 'BetaSearchService' with 'scope_alpha' must NOT include BetaSearchService (regression guard)"
else
    pass "B2. MCP search 'BetaSearchService' with 'scope_alpha' excludes scope_beta (regression guard)"
fi

# B3: whitespace around tokens tolerated (" scope_alpha , scope_beta ")
REQUEST_WS='{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":" scope_alpha , scope_beta "}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_WS"; then
    pass "B3. MCP search with whitespace-padded list finds BetaSearchService"
else
    fail "B3. MCP search with whitespace-padded list (expected BetaSearchService)"
fi

# B4: unknown repo in list degrades gracefully (BetaSearchService must be excluded)
REQUEST_UNKNOWN='{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"scope_alpha,scope_gamma"}}}'
RESP_UNKNOWN=$(call_mcp "$REQUEST_UNKNOWN")
if echo "$RESP_UNKNOWN" | grep -qiE "BetaSearchService"; then
    fail "B4. MCP search with unknown repo must NOT include BetaSearchService"
else
    pass "B4. MCP search with unknown repo excludes scope_beta"
fi

# B5: duplicate tokens collapse ("scope_beta,scope_beta") — finds BetaSearchService
REQUEST_DUP='{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":"scope_beta,scope_beta"}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_DUP"; then
    pass "B5. MCP search with duplicate tokens finds BetaSearchService"
else
    fail "B5. MCP search with duplicate tokens (expected BetaSearchService)"
fi

echo ""
echo -e "${BLUE}── Group C: find_callers across scopes ──${NC}"

# C1: find_callers "SharedUtil.work" with repo_name "all" → both callers
REQUEST_C_ALL='{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"SharedUtil.work","repo_name":"all"}}}'
RESP_C_ALL=$(call_mcp "$REQUEST_C_ALL")
if echo "$RESP_C_ALL" | grep -q "alphaCaller"; then
    pass "C1. find_callers SharedUtil.work 'all' includes alphaCaller"
else
    fail "C1. find_callers SharedUtil.work 'all' (expected alphaCaller)"
fi
if echo "$RESP_C_ALL" | grep -q "betaCaller"; then
    pass "C1. find_callers SharedUtil.work 'all' includes betaCaller"
else
    fail "C1. find_callers SharedUtil.work 'all' (expected betaCaller)"
fi

# C2: find_callers "SharedUtil.work" with two-repo list
REQUEST_C_LIST='{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"SharedUtil.work","repo_name":"scope_alpha,scope_beta"}}}'
RESP_C_LIST=$(call_mcp "$REQUEST_C_LIST")
if echo "$RESP_C_LIST" | grep -q "alphaCaller" && echo "$RESP_C_LIST" | grep -q "betaCaller"; then
    pass "C2. find_callers SharedUtil.work with list includes both callers"
else
    fail "C2. find_callers SharedUtil.work with list (expected both callers)"
fi

# C3: single-repo guard — find_callers with scope_alpha shows alphaCaller only
REQUEST_C_ALPHA='{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"find_callers","arguments":{"entity_name":"SharedUtil.work","repo_name":"scope_alpha"}}}'
RESP_C_ALPHA=$(call_mcp "$REQUEST_C_ALPHA")
if echo "$RESP_C_ALPHA" | grep -q "alphaCaller"; then
    pass "C3. find_callers SharedUtil.work 'scope_alpha' includes alphaCaller (regression guard)"
else
    fail "C3. find_callers SharedUtil.work 'scope_alpha' (regression guard, expected alphaCaller)"
fi
if echo "$RESP_C_ALPHA" | grep -q "betaCaller"; then
    fail "C3. find_callers SharedUtil.work 'scope_alpha' must NOT include betaCaller (regression guard)"
else
    pass "C3. find_callers SharedUtil.work 'scope_alpha' excludes betaCaller (regression guard)"
fi

echo ""
echo -e "${BLUE}── Group D: explore_file ambiguity & resolution ──${NC}"

# D1: explore_file "src/index.ts" with no repo_name → ambiguous_path_candidates has 2 entries
REQUEST_D1='{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"src/index.ts"}}}'
RESP_D1=$(call_mcp "$REQUEST_D1")
if echo "$RESP_D1" | grep -q "ambiguous_path_candidates"; then
    pass "D1. explore_file 'src/index.ts' without scope returns ambiguous_path_candidates"
else
    fail "D1. explore_file 'src/index.ts' without scope (expected ambiguous_path_candidates)"
fi
# Count occurrences of repo_name field inside the candidates array (each entry has one)
CAND_COUNT=$(echo "$RESP_D1" | grep -oE '"repo_name"' | wc -l)
if [ "$CAND_COUNT" -ge 2 ]; then
    pass "D1. ambiguous_path_candidates contains >= 2 entries"
else
    fail "D1. ambiguous_path_candidates contains only $CAND_COUNT entries (expected >= 2)"
fi

# D2: explore_file "src/index.ts" with repo_name "scope_beta" → resolves without ambiguity
REQUEST_D2='{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"explore_file","arguments":{"file_path":"src/index.ts","repo_name":"scope_beta"}}}'
RESP_D2=$(call_mcp "$REQUEST_D2")
if echo "$RESP_D2" | grep -q "BetaSearchService"; then
    pass "D2. explore_file 'src/index.ts' with 'scope_beta' resolves to beta fixture"
else
    fail "D2. explore_file 'src/index.ts' with 'scope_beta' (expected BetaSearchService)"
fi
if echo "$RESP_D2" | grep -q "ambiguous_path_candidates"; then
    fail "D2. explore_file with scope should NOT be ambiguous"
else
    pass "D2. explore_file with scope resolves without ambiguity"
fi

echo ""
echo -e "${BLUE}── Group E: JSON array form ──${NC}"

# E1: repo_name as JSON array
REQUEST_E1='{"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"BetaSearchService","repo_name":["scope_alpha","scope_beta"]}}}'
if retry_match "BetaSearchService" call_mcp "$REQUEST_E1"; then
    pass "E1. MCP search with repo_name JSON array finds BetaSearchService"
else
    fail "E1. MCP search with repo_name JSON array (expected BetaSearchService)"
fi

echo ""
echo -e "${BLUE}── Group F: CLI parity ──${NC}"

# F1: knot search "BetaSearchService" --repo "scope_alpha,scope_beta"
CLI_F1=$(call_cli "$PROJECT_ROOT" search "BetaSearchService" --repo "scope_alpha,scope_beta")
if echo "$CLI_F1" | grep -qiE "BetaSearchService"; then
    pass "F1. CLI search --repo list finds BetaSearchService"
else
    fail "F1. CLI search --repo list (expected BetaSearchService)"
fi

# F2: knot search "BetaSearchService" --repo all
CLI_F2=$(call_cli "$PROJECT_ROOT" search "BetaSearchService" --repo all)
if echo "$CLI_F2" | grep -qiE "BetaSearchService"; then
    pass "F2. CLI search --repo all finds BetaSearchService (scope_beta)"
else
    fail "F2. CLI search --repo all (expected BetaSearchService)"
fi

# F3: knot search "SharedUtil" --repo scope_beta (regression guard)
CLI_F3=$(call_cli "$PROJECT_ROOT" search "SharedUtil" --repo scope_beta)
if echo "$CLI_F3" | grep -qiE "SharedUtil"; then
    pass "F3. CLI search --repo scope_beta (regression guard, single-repo)"
else
    fail "F3. CLI search --repo scope_beta (regression guard, expected SharedUtil)"
fi

# F4: knot search "SharedUtil" from scope_alpha's dir — auto-detect default
CLI_F4=$(call_cli "$ALPHA_REPO_DIR" search "SharedUtil")
if echo "$CLI_F4" | grep -qiE "SharedUtil"; then
    pass "F4. CLI search without --repo (auto-detected default, regression guard)"
else
    fail "F4. CLI search without --repo (regression guard, expected SharedUtil from scope_alpha)"
fi

echo ""
echo -e "${BLUE}── Group G: Output labeling ──${NC}"

# G1: every result row in MCP multi-repo search carries (repo: scope_alpha|scope_beta)
REQUEST_G1='{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"search_hybrid_context","arguments":{"query":"SharedUtil","repo_name":"all","max_results":10}}}'
RESP_G1=$(call_mcp "$REQUEST_G1")
REPO_TAGS=$(echo "$RESP_G1" | grep -oE '\(repo: [^)]+\)' | wc -l)
if [ "$REPO_TAGS" -ge 2 ]; then
    pass "G1. Multi-repo MCP results carry (repo: ...) annotation ($REPO_TAGS tags)"
else
    fail "G1. Multi-repo MCP results missing (repo: ...) annotation (found $REPO_TAGS)"
fi
if echo "$RESP_G1" | grep -q "(repo: scope_alpha)" && echo "$RESP_G1" | grep -q "(repo: scope_beta)"; then
    pass "G1. (repo: ...) annotation names both scope_alpha and scope_beta"
else
    fail "G1. (repo: ...) annotation does not name both repos"
fi

# G2: CLI multi-repo search also labels every row
CLI_G2=$(call_cli "$PROJECT_ROOT" search "SharedUtil" --repo all)
REPO_TAGS_CLI=$(echo "$CLI_G2" | grep -oE '\(repo: [^)]+\)' | wc -l)
if [ "$REPO_TAGS_CLI" -ge 2 ]; then
    pass "G2. Multi-repo CLI results carry (repo: ...) annotation ($REPO_TAGS_CLI tags)"
else
    fail "G2. Multi-repo CLI results missing (repo: ...) annotation (found $REPO_TAGS_CLI)"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Repo Scope E2E Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}Passed: $PASSES${NC}"
echo -e "${RED}Failed: $FAILURES${NC}"
echo ""

if [ "$FAILURES" -eq 0 ]; then
    echo -e "${GREEN}All repo scope E2E tests passed! ✓${NC}"
    exit 0
else
    echo -e "${RED}$FAILURES repo scope E2E test(s) failed — see red flags above. ✗${NC}"
    echo -e "${YELLOW}Phase 0 red criterion: groups A/B/E/F/G and parts of C/D should FAIL${NC}"
    echo -e "${YELLOW}until the production RepoScope model + DB-layer IN filter lands.${NC}"
    exit 1
fi
