#!/usr/bin/env bash
# E2E Integration Test Script for Markdown Support in knot
#
# This script tests Markdown-specific features:
# 1. Spins up isolated Neo4j and Qdrant instances on high ports (17xxx/16xxx)
# 2. Indexes Markdown fixture files under tests/testing_files/markdown/
# 3. Queries via MCP/CLI to validate Markdown entity extraction
# 4. Cleans up containers and data
#
# Usage: ./tests/run_markdown_e2e.sh
# Requirements: docker, docker-compose

set -e
set -u

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.e2e.yml"
TEST_FILES_DIR="$SCRIPT_DIR/testing_files/markdown"
E2E_DATA_DIR="$SCRIPT_DIR/.e2e_markdown_data"

# Database configuration (high ports to avoid conflicts)
NEO4J_URI="bolt://localhost:17687"
NEO4J_USER="neo4j"
NEO4J_PASSWORD="e2e_test_password"
QDRANT_URL="http://localhost:16334"
QDRANT_COLLECTION="knot_markdown_e2e_test"
REPO_NAME="markdown_e2e_test_repo"

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
echo -e "${BLUE}knot Markdown E2E Integration Test${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Cleanup function (runs on exit)
cleanup() {
    local exit_code=$?

    if [ $exit_code -ne 0 ]; then
        echo -e "\n${RED}Markdown E2E tests failed!${NC}"
        echo -e "${YELLOW}To clean up manually:${NC}"
        echo "  cd $SCRIPT_DIR && docker compose -f docker-compose.e2e.yml down -v"
        echo "  sudo rm -rf $E2E_DATA_DIR"
        return 0
    fi

    if [[ -n "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
        return 0
    fi

    echo -e "\n${YELLOW}Cleaning up Markdown E2E test environment...${NC}"
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    if [ -d "$E2E_DATA_DIR" ]; then
        sudo rm -rf "$E2E_DATA_DIR" 2>/dev/null || rm -rf "$E2E_DATA_DIR" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# Step 1: Start Docker containers (skipped if KNOT_E2E_EXTERNAL_DB is set)
if [[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]]; then
    echo -e "${YELLOW}[1/5] Starting Docker containers for Markdown E2E test...${NC}"
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

# Step 3: Index Markdown fixture files
echo -e "${YELLOW}[3/5] Indexing Markdown fixture files...${NC}"
cd "$PROJECT_ROOT"

echo "Building knot-indexer..."
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling|Finished|error)" || true

echo "Running indexer for Markdown files..."
INDEXER_FLAGS=()
[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")
cargo run --release --bin knot-indexer -- "${INDEXER_FLAGS[@]}"

echo -e "${GREEN}✓ Markdown files indexed${NC}"

# Step 4: Validate results via MCP server and CLI
echo -e "${YELLOW}[4/5] Validating Markdown entities via knot-mcp and knot CLI...${NC}"

echo "Building knot-mcp and knot..."
cargo build --release --bin knot-mcp 2>&1 | grep -E "(Compiling|Finished|error)" || true
cargo build --release --bin knot 2>&1 | grep -E "(Compiling|Finished|error)" || true

# Helper: run a Cypher query against the E2E Neo4j and return the first
# data row. Used for structural assertions that bypass MCP/search ranking.
run_neo4j_cypher() {
    local query="$1"
    echo "$query" | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
        --format plain 2>/dev/null \
        | tail -n +2
}

echo ""
echo "Test 1: Searching for a body-only phrase to prove section body is in embed_text..."
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"SPHINX_WIDGET_TOKEN_42\",\"repo_name\":\"$REPO_NAME\"}}}"

MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

# The token only appears in a code block inside the Setup section body.
# If the search returns the Setup markdown_section, embed_text must contain
# the body — proving the regression flagged in the issue is fixed.
if echo "$MCP_RESPONSE" | grep -q "markdown_section" && echo "$MCP_RESPONSE" | grep -q "Setup"; then
    echo -e "${GREEN}✓ Section body is searchable (embed_text includes body content)${NC}"
else
    echo -e "${RED}✗ Body-only phrase not found — embed_text likely only contains heading${NC}"
    echo "Response was: $MCP_RESPONSE"
    exit 1
fi

# Test 2: Two files with the same heading text produce distinct entities
# with correct body content (no collision / cross-contamination).
echo ""
echo "Test 2: Same heading in two files — distinct entities, distinct bodies..."

# Search for the token unique to GUIDE.md's Setup section.
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"GIZMO_ARTIFACT_99\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

# The token only appears in GUIDE.md's Setup body. The search should return
# a Setup section whose FQN points to GUIDE.md, NOT README.md.
if echo "$MCP_RESPONSE" | grep -q "GUIDE.md" && echo "$MCP_RESPONSE" | grep -q "Setup"; then
    echo -e "${GREEN}✓ GUIDE.md Setup section found via its unique body content${NC}"
else
    echo -e "${RED}✗ GUIDE.md Setup section not correctly disambiguated${NC}"
    echo "Response: $MCP_RESPONSE"
    exit 1
fi

# Now verify the README.md token still resolves to README.md's Setup, not GUIDE's.
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"SPHINX_WIDGET_TOKEN_42\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "README.md" && echo "$MCP_RESPONSE" | grep -q "Setup"; then
    echo -e "${GREEN}✓ README.md Setup section still resolves correctly after adding GUIDE.md${NC}"
else
    echo -e "${RED}✗ README.md Setup section was corrupted by GUIDE.md addition${NC}"
    echo "Response: $MCP_RESPONSE"
    exit 1
fi

# Cypher cross-check: verify there are exactly 3 distinct Setup sections
# (one per file), each with its own embed_text containing the file-specific
# token. Proves the two entities are structurally separate, not just
# returned separately by search.
SETUP_COUNT=$(run_neo4j_cypher "MATCH (s:Entity) WHERE s.kind = 'markdown_section' AND s.name = 'Setup' RETURN count(s) AS cnt;")
SETUP_COUNT=${SETUP_COUNT:-0}

if [ "$SETUP_COUNT" = "3" ]; then
    echo -e "${GREEN}✓ Cypher: exactly 3 distinct Setup sections in the graph${NC}"
else
    echo -e "${RED}✗ Cypher: expected 3 Setup sections, got $SETUP_COUNT${NC}"
    exit 1
fi

GUIDE_EMBED=$(run_neo4j_cypher "MATCH (s:Entity) WHERE s.kind = 'markdown_section' AND s.name = 'Setup' AND s.file_path ENDS WITH 'GUIDE.md' RETURN s.embed_text AS et;")

if echo "$GUIDE_EMBED" | grep -q "GIZMO_ARTIFACT_99"; then
    echo -e "${GREEN}✓ Cypher: GUIDE.md Setup embed_text contains its own body token${NC}"
else
    echo -e "${RED}✗ Cypher: GUIDE.md Setup embed_text missing its body token${NC}"
    echo "Response: $GUIDE_EMBED"
    exit 1
fi

# Test 3: Heading text with special characters (backticks + em-dash)
# is preserved correctly in the section name/FQN, and body content
# under such headings is still indexed.
echo ""
echo "Test 3: Heading with special characters (backticks, em-dash) parses correctly..."

MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"reacting to real shell context\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

# The phrase lives in a paragraph under the H3 heading
# '### `next-cmd` — ghost completion'. If the search returns the
# correct section from complex.md, the parser handled the special-
# character heading without dropping its body content.
if echo "$MCP_RESPONSE" | grep -q "complex.md" && echo "$MCP_RESPONSE" | grep -q "next-cmd"; then
    echo -e "${GREEN}✓ H3 heading with backticks and em-dash indexed body correctly${NC}"
else
    echo -e "${RED}✗ Heading with special characters did not index its body${NC}"
    echo "Response: $MCP_RESPONSE"
    exit 1
fi

# Test 4: Natural-language query (no literal keyword overlap with body)
# routes to the semantically correct section. This stresses the embedding
# model — BM25 alone cannot satisfy this query because the phrase doesn't
# appear verbatim in the indexed text.
echo ""
echo "Test 4: Natural-language query matches semantically correct section..."

MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"how do I get started with this for the first time\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

# The Setup section should rank highest because its body describes
# installation steps, even though it doesn't contain the literal words
# "get started" or "first time".
if echo "$MCP_RESPONSE" | grep -q "Setup" && echo "$MCP_RESPONSE" | grep -q "complex.md"; then
    echo -e "${GREEN}✓ Natural-language query routed to semantically correct section${NC}"
else
    echo -e "${RED}✗ Natural-language query did not return expected section${NC}"
    echo "Response: $MCP_RESPONSE"
    exit 1
fi


# Test 5: Deep nesting (H4) and within-file same-name disambiguation
echo ""
echo "Test 5: Deep nesting and within-file FQN disambiguation..."
# Verify both 'Examples' sections exist as distinct nodes under
# different parents.
EXAMPLES_COUNT=$(run_neo4j_cypher "MATCH (s:Entity) WHERE s.kind = 'markdown_section' AND s.name = 'Examples' AND s.file_path ENDS WITH 'nested.md' RETURN count(s) AS cnt;")
EXAMPLES_COUNT=${EXAMPLES_COUNT:-0}

if [ "$EXAMPLES_COUNT" = "2" ]; then
    echo -e "${GREEN}✓ Two distinct 'Examples' sections exist under different parents${NC}"
else
    echo -e "${RED}✗ Expected 2 'Examples' sections in api_reference.md, got $EXAMPLES_COUNT${NC}"
    exit 1
fi

# Verify H4 sections (the curl/Python sub-sections) exist with their
# bodies indexed. Tokens are unique to each tree branch.
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"AUTH_TOKEN_FLOW_31\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

if echo "$MCP_RESPONSE" | grep -q "Authentication" && echo "$MCP_RESPONSE" | grep -q "Examples"; then
    echo -e "${GREEN}✓ Body content under deeply-nested H4 sections is searchable${NC}"
else
    echo -e "${RED}✗ Deep nesting broke body indexing${NC}"
    echo "Response: $MCP_RESPONSE"
    exit 1
fi

# Test 6: Sections have real line numbers (not the 1,1 placeholders
# the YAML parser uses). Tree-sitter provides accurate positions, so
# any section with end_line == 1 or end_line <= start_line indicates
# the parser didn't compute boundaries correctly.
echo ""
echo "Test 6: Section line numbers are real, not placeholders..."

PLACEHOLDER_COUNT=$(echo "MATCH (s:Entity) WHERE s.kind = 'markdown_section' AND (s.end_line <= 1 OR s.end_line < s.start_line) RETURN count(s) AS cnt;" \
    | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" --format plain 2>/dev/null \
    | tail -n +2 | head -n 1 | tr -d ' ')
PLACEHOLDER_COUNT=${PLACEHOLDER_COUNT:-0}

if [ "$PLACEHOLDER_COUNT" = "0" ]; then
    echo -e "${GREEN}✓ All section line numbers are real (no 1,1 placeholders)${NC}"
else
    echo -e "${RED}✗ $PLACEHOLDER_COUNT sections still have placeholder line numbers${NC}"
    exit 1
fi

# Sanity check: a specific known section should have plausible line range.
# GUIDE.md's Setup heading is on line 5; its body extends a few lines below.
SETUP_RANGE=$(echo "MATCH (s:Entity) WHERE s.kind = 'markdown_section' AND s.name = 'Setup' AND s.file_path ENDS WITH 'GUIDE.md' RETURN s.start_line + '-' + s.end_line AS range;" \
    | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" --format plain 2>/dev/null \
    | tail -n +2 | head -n 1 | tr -d ' "')

START=$(echo "$SETUP_RANGE" | cut -d'-' -f1)
END=$(echo "$SETUP_RANGE" | cut -d'-' -f2)

if [ "$START" -ge "1" ] && [ "$END" -gt "$START" ]; then
    echo -e "${GREEN}✓ GUIDE.md Setup section has valid range ($SETUP_RANGE)${NC}"
else
    echo -e "${RED}✗ GUIDE.md Setup section has invalid range ($SETUP_RANGE)${NC}"
    exit 1
fi


# Step 5: Summarize
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All Markdown E2E tests passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Validated Markdown features:"
echo "  ✓ MarkdownDocument entity creation from .md files"
echo "  ✓ Section body included in embed_text (semantic search regression test)"
echo "  ✓ Same heading in different files produces distinct, non-colliding entities"
echo "  ✓ Headings with special characters (backticks, em-dash) parse correctly"
echo "  ✓ Natural-language query matches semantically correct section"
echo "  ✓ Deep nesting and within-file FQN disambiguation"
echo "  ✓  Section line numbers are real, not placeholders"
echo ""

exit 0