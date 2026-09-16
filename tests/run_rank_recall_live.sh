#!/usr/bin/env bash
# Rank-recall verification against a LIVE knot index (opt-in harness).
#
# Unlike the other E2E suites this script does NOT spin up its own databases
# or index fixtures: it measures the entry-point recall contract (the
# v1.9.7 "residual rows" regression table) against whatever repositories
# are currently indexed by a live knot index (Qdrant + Neo4j reachable and
# the listed repos already indexed). If that precondition is missing the
# script SKIPS with exit 0 and an explicit message — it never fails a CI
# run, and it is deliberately NOT part of run_all_e2e_fast.sh.
#
# Usage:
#   export KNOT_NEO4J_PASSWORD=...          # credential for the live Neo4j
#   ./tests/run_rank_recall_live.sh
#
# Optional env:
#   KNOT_STORE_BIN=/path/to/knot            # knot CLI to measure with
#                                           # (default: target/release/knot)
#
# Row contract (columns): query|repo|expected_name|expected_path_snippet|
#                         max_position|status
#   status = must  — FAILs when the expected entry point is missing or sits
#                     past max_position (1-based).
#   status = info  — never fails; prints the measured position. Used for
#                     rows that are not reachable query-time without
#                     re-indexing (see CHANGELOG for the evidence).
#
# The two "baseline" rows are the already-fixed cases of the original bug
# report; they exist to catch no-regressions in the ranking contract.

set -u

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
readonly BIN="${KNOT_STORE_BIN:-$PROJECT_ROOT/target/release/knot}"
readonly PY="$(command -v python3 || true)"
readonly NEEDS_PY="ranking arithmetic"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot rank-recall LIVE harness${NC}"
echo -e "${BLUE}========================================${NC}"

# --- preconditions (skip, never fail) ---------------------------------------

skip() { echo -e "${YELLOW}SKIP: $1${NC}"; exit 0; }

if [ -z "${KNOT_NEO4J_PASSWORD:-}" ]; then
    skip "KNOT_NEO4J_PASSWORD is not set — a live indexed knot instance is
required for this harness (it uses the developer's indexed repositories;
see tests/run_rank_recall_live.sh header)."
fi
if [ ! -x "$BIN" ]; then
    skip "knot CLI not found at $BIN (run: cargo build --release)."
fi
if [ -z "$PY" ]; then
    skip "python3 not found — the harness needs it for $NEEDS_PY."
fi

echo -e "${YELLOW}[1/3] Checking live index reachability...${NC}"
if ! "$BIN" repos -o json >/dev/null 2>&1; then
    skip "knot CLI could not reach the live index (Neo4j down? wrong
KNOT_NEO4J_PASSWORD?)."
fi

# Repos this harness needs. Each is resolved by name via `knot repos`;
# if one is absent the associated rows print SKIPPED-ROW, and the harness
# fails only when a `must` row's repo is missing (a broken baseline).
NEED_REPOS=("knot" "HikariCP" "csharp-code-map" "chrome-devtools-mcp" "job-watch-ui" "job-watch")
for repo in "${NEED_REPOS[@]}"; do
    if ! "$BIN" repos -f "$repo" -o json 2>/dev/null | grep -q "\"$repo\""; then
        skip "repository '$repo' is not present in the live index."
    fi
done

echo -e "${YELLOW}[2/3] Measuring rows (one search per row)...${NC}"

# query|repo|expected_name|expected_path_snippet|max_position|status
readonly ROWS=(
  # --- already-fixed baseline rows (no-regression guardrails) ---
  "borrow a connection from the pool|HikariCP|getConnection|pool/HikariPool.java|1|must"
  "authenticate user with email and password|job-watch|login|src/api/auth.rs|1|must"
  # --- v1.9.7 residual rows under test ---
  # Note for the tool rows: the parser emits one `constant screenshot` per
  # descriptor; several legitimate rows share the name. The contract is
  # "a screenshot tool definition ranks #1 (prose below)":
  #   - "take screenshot": any tool row in src/tools (matches slim + main).
  #   - "capture the current view": the main tool file.
  "capture the current view as an image|chrome-devtools-mcp|screenshot|src/tools/screenshot.ts|1|must"
  "take screenshot|chrome-devtools-mcp|screenshot|src/tools|1|must"
  "acquire a client for talking to the database|HikariCP|getConnection|pool/HikariPool.java|1|info"
  "get the callers of a symbol|csharp-code-map|GetCallersAsync|src/|8|must"
  "log a user in and issue a session token|job-watch-ui|login|src/auth/AuthContext.tsx|8|info"
  # --- documented unreachable query-time (position INFO-only) ---
  "find relevant code by meaning across the repository|knot|run_search_hybrid_context|src/cli_tools/search_hybrid_context/mod.rs|0|info"
  "find who invokes a given symbol|csharp-code-map|GetCallersAsync|src/CodeMap.Query/QueryEngine.cs|0|info"
)

readonly TMPDIR_RR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_RR"' EXIT INT TERM

measure_row() {
    # $1 query, $2 repo, out: results json into $TMPDIR_RR/out.json
    "$BIN" search "$1" -r "$2" -m 8 -o json >"$TMPDIR_RR/out.json" 2>"$TMPDIR_RR/err.log"
}

fail=0
printf "%-58s %-22s %-8s %-8s %-6s\n" "QUERY" "REPO" "EXPECT" "MEASURED" "VERDICT"
printf "%s\n" "----------------------------------------------------------------------------------------------"

for row in "${ROWS[@]}"; do
    IFS='|' read -r query repo want_name want_path max_pos status <<< "$row"
    measured="ABSENT"
    verdict="SKIP"
    if measure_row "$query" "$repo"; then
        position="$("$PY" - "$TMPDIR_RR/out.json" "$want_name" "$want_path" <<'EOF'
import json, sys
try:
    rows = json.load(open(sys.argv[1]))
except Exception:
    print("ERR"); raise SystemExit
if not isinstance(rows, list):
    print("ERR"); raise SystemExit
for i, e in enumerate(rows, 1):
    name = (e.get("name") or "")
    path = (e.get("file_path") or "")
    if sys.argv[2] == name and sys.argv[3] in path:
        print(i)
        raise SystemExit
print("ABSENT")
EOF
)"
        if [ "$position" != "ERR" ]; then
            measured="$position"
        fi
        if [ "$position" = "ERR" ]; then
            verdict="ERROR"
            [ "$status" = "must" ] && fail=1
        elif [ "$position" = "ABSENT" ]; then
            if [ "$status" = "must" ]; then
                verdict="FAIL"
                fail=1
            else
                verdict="INFO"
            fi
        else
            if [ "$max_pos" != "0" ] && [ "$position" -le "$max_pos" ]; then
                verdict="PASS"
            elif [ "$status" = "info" ]; then
                verdict="INFO"
            else
                verdict="FAIL"
                fail=1
            fi
        fi
    else
        verdict="ERROR"
        [ "$status" = "must" ] && fail=1
    fi
    printf "%-58s %-22s %-8s %-8s %s%s%s\n" \
        "${query:0:58}" "$repo" "#$max_pos: $want_name" "$measured" \
        "$([ "$verdict" = "FAIL" ] && echo -e "$RED$verdict$NC" || echo -e "$([ "$verdict" = "PASS" ] && echo -e "$GREEN$verdict$NC" || echo -e "${YELLOW}${verdict}${NC}")")"
done

echo -e "${YELLOW}[3/3] Summary${NC}"
if [ "$fail" -ne 0 ]; then
    echo -e "${RED}Rank-recall harness FAILED — see FAIL rows above.${NC}"
    exit 1
fi
echo -e "${GREEN}Rank-recall harness PASSED (INFO rows report measured positions only).${NC}"
