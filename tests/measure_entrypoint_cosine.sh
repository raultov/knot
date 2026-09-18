#!/usr/bin/env bash
# Cosine-window measurement for entry-point recall (opt-in, LIVE index).
#
# Companion diagnostics to tests/run_rank_recall_live.sh: while that harness
# checks the FINAL ranked position of an expected entry point, this script
# answers the question a regression there raises — was the entry point inside
# the semantic window at all, or did the re-ranker demote it?
#
# For every row it runs a search with the rank trace enabled
# (RUST_LOG=search_hybrid_context::rank=debug) at -m 100, which forces
# candidate_limit(100) = 400: the trace then yields every pool candidate's
# raw cosine. From that we report
#
#   - "Window cutoff": the lowest cosine of the plain Qdrant pass
#     (channel="cosine") — the 400th cosine of the unrestricted window;
#   - "Cosine rank": the target's position inside the FULL pool (cosine +
#     definition + probe + bridge channels merged), ordered by raw cosine;
#   - "Final rank": the target's position in the returned results.
#
# Decision rule:
#   cosine <= cutoff (or "-")  → the target is OUTSIDE the cosine window;
#     no query-time ranking change can reach it. Fix embed_text or the
#     embedding model.
#   cosine > cutoff, final rank poor  → inside the window, re-ranker issue.
#
# When the target is absent from the unrestricted pool, a second
# path-restricted search (-p PROBE_DIR, oversampled x3 cap 600) forces the
# target into the pool through the definition channel, exposing its TRUE
# cosine ("path-probe" in the Source column).
#
# Like run_rank_recall_live.sh this harness needs a LIVE, already-indexed
# knot instance (Qdrant + Neo4j reachable, the listed repositories indexed)
# and is deliberately NOT part of run_all_e2e_fast.sh — it skips with exit 0
# when the preconditions are missing and never fails a Docker-less run.
#
# Usage:
#   export KNOT_NEO4J_PASSWORD=...          # credential for the live Neo4j
#   ./tests/measure_entrypoint_cosine.sh [--out FILE] [--rows targets|baselines|all]
#
# Optional env:
#   KNOT_STORE_BIN=/path/to/knot            # knot CLI to measure with
#                                           # (default: target/release/knot)
#
# Row contract (columns): role|query|repo|expected_name|expected_path_snippet|probe_dir
#   role = target   — residual-recall rows under repair; read the cosine columns.
#   role = baseline — no-regression guardrails; only the final rank matters.

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
readonly NEEDS_PY="cosine-window arithmetic"

OUT_FILE=""
ROWS_FILTER="all"
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT_FILE="$2"; shift 2 ;;
        --rows) ROWS_FILTER="$2"; shift 2 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# Normalize the row filter: the row contract uses singular roles
# (`target` / `baseline`). Accept the documented plural spellings and fail
# loudly on an unknown value instead of silently selecting nothing.
case "$ROWS_FILTER" in
    target|targets) ROWS_FILTER="target" ;;
    baseline|baselines) ROWS_FILTER="baseline" ;;
    all|"") ROWS_FILTER="all" ;;
    *)
        echo "Unknown --rows value: $ROWS_FILTER (expected: targets|baselines|all)" >&2
        exit 2
        ;;
esac

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}knot entry-point cosine-window harness${NC}"
echo -e "${BLUE}========================================${NC}"

# --- preconditions (skip, never fail) ---------------------------------------

skip() { echo -e "${YELLOW}SKIP: $1${NC}"; exit 0; }

if [ -z "${KNOT_NEO4J_PASSWORD:-}" ]; then
    skip "KNOT_NEO4J_PASSWORD is not set — a live indexed knot instance is
required for this harness (see the file header)."
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

# Repos this harness needs; resolved by name via `knot repos`.
NEED_REPOS=("knot" "HikariCP" "csharp-code-map" "chrome-devtools-mcp" "job-watch-ui" "job-watch")
for repo in "${NEED_REPOS[@]}"; do
    if ! "$BIN" repos -f "$repo" -o json 2>/dev/null | grep -q "\"$repo\""; then
        skip "repository '$repo' is not present in the live index."
    fi
done

# role|query|repo|expected_name|expected_path_snippet|probe_dir
readonly ROWS=(
  # --- entry-point targets (the residual-recall rows under repair) ---
  "target|find relevant code by meaning across the repository|knot|run_search_hybrid_context|src/cli_tools/search_hybrid_context/mod.rs|src/cli_tools/search_hybrid_context"
  "target|acquire a client for talking to the database|HikariCP|getConnection|pool/HikariPool.java|src/main/java/com/zaxxer/hikari/pool"
  "target|get the callers of a symbol|csharp-code-map|GetCallersAsync|src/|src"
  "target|find who invokes a given symbol|csharp-code-map|GetCallersAsync|src/CodeMap.Query/QueryEngine.cs|src/CodeMap.Query"
  "target|log a user in and issue a session token|job-watch-ui|login|src/auth/AuthContext.tsx|src/auth"
  # --- baselines: already-fixed rows (no-regression guardrails) ---
  # NOTE: the spring-ai "create a client to chat with a language model" #1
  # guardrail (25.7k entities) was measured in the v1.9.8 baseline but is
  # excluded from the dev-cycle tables — it is covered by the mandatory full
  # re-index the embedding-model change requires (see CHANGELOG).
  "baseline|borrow a connection from the pool|HikariCP|getConnection|pool/HikariPool.java|src/main/java/com/zaxxer/hikari/pool"
  "baseline|authenticate user with email and password|job-watch|login|src/api/auth.rs|src/api"
  "baseline|take screenshot|chrome-devtools-mcp|screenshot|src/tools|src/tools"
  "baseline|capture the current view as an image|chrome-devtools-mcp|screenshot|src/tools/screenshot.ts|src/tools"
)

readonly TMPDIR_EC="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_EC"' EXIT INT TERM

cat >"$TMPDIR_EC/parse.py" <<'PYEOF'
import json, re, sys

ANSI = re.compile(r'\x1b\[[0-9;]*m')


def parse_trace(path):
    """[(cosine, name, channel)] from one rank-trace log, cosine-descending."""
    rows = []
    for line in open(path, errors="ignore"):
        line = ANSI.sub("", line)
        if "ranked" not in line:
            continue
        c = re.search(r'cosine=([\d.]+)', line)
        n = re.search(r'name="([^"]*)"', line)
        ch = re.search(r'channel="([^"]*)"', line)
        if c and n:
            rows.append((float(c.group(1)), n.group(1), ch.group(1) if ch else "?"))
    rows.sort(key=lambda r: -r[0])
    return rows


def cutoff_of(rows):
    """Lowest cosine of the plain Qdrant pass (channel='cosine')."""
    cos_only = [c for c, _, ch in rows if ch == "cosine"]
    return min(cos_only) if cos_only else (rows[-1][0] if rows else 0.0)


def best_hit(rows, name):
    """Highest-cosine hit named `name`: (rank, cosine)."""
    hits = [(i + 1, c) for i, (c, n, _) in enumerate(rows) if n == name]
    return hits[0] if hits else None


def final_rank(out_path, name, snippet):
    """1-based position of `name` (in `snippet`) in the returned results."""
    try:
        data = json.load(open(out_path))
    except Exception:
        return "ERR"

    def walk(x):
        if isinstance(x, dict):
            if "name" in x:
                yield x
            for v in x.values():
                yield from walk(v)
        elif isinstance(x, list):
            for v in x:
                yield from walk(v)

    for i, e in enumerate(walk(data), 1):
        if e.get("name") == name and snippet in (e.get("file_path") or ""):
            return str(i)
    return "ABSENT"


def fmt(cosine, cutoff, rank, pool, final, source):
    return f"{cosine}|{cutoff}|{rank}|{pool}|{final}|{source}"


trace, out_json, name, snippet = sys.argv[1:5]
have_probe = len(sys.argv) >= 7
probe_trace, probe_out = (sys.argv[5], sys.argv[6]) if have_probe else ("", "")

rows = parse_trace(trace)
cutoff = cutoff_of(rows)
hit = best_hit(rows, name)
if hit:
    rank, cosine = hit
    final = final_rank(out_json, name, snippet)
    print(fmt(f"{cosine:.4f}", f"{cutoff:.4f}", rank, len(rows), final, "direct"))
else:
    # The cosine-rank column that matters for the decision rule is always
    # the UNRESTRICTED cutoff: the target's probe-sampled cosine must be
    # compared against the plain -m 100 window, never against the probe
    # run's own (path-filtered) window.
    final = final_rank(out_json, name, snippet)
    if have_probe:
        prow = parse_trace(probe_trace)
        phit = best_hit(prow, name)
        if phit:
            _, pcos = phit
            print(fmt(f"{pcos:.4f}*", f"{cutoff:.4f}", "absent", len(prow), f"{final}*", "path-probe"))
        else:
            print(fmt("-", f"{cutoff:.4f}", "absent", len(rows), "ABSENT", "outside-pool"))
    else:
        print(fmt("-", f"{cutoff:.4f}", "absent", len(rows), final, "outside-pool"))
PYEOF

echo -e "${YELLOW}[2/3] Measuring rows (up to two searches per row)...${NC}"

header="| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |"
rule="|---|---|---|---|---|---|---|---|"

TMP_OUT=""
if [ -n "$OUT_FILE" ]; then
    TMP_OUT="$(mktemp)"
    printf '%s\n%s\n' "$header" "$rule" >"$TMP_OUT"
fi

print_row() {
    echo "$1"
    if [ -n "$TMP_OUT" ]; then printf '%s\n' "$1" >>"$TMP_OUT"; fi
}

fail=0
valid_rows=0
total_measured=0

for row in "${ROWS[@]}"; do
    IFS='|' read -r role query repo want_name want_path probe_dir <<< "$row"
    if [ "$ROWS_FILTER" != "all" ] && [ "$role" != "$ROWS_FILTER" ]; then continue; fi

    total_measured=$((total_measured + 1))
    # Reset per iteration: a failed search must not poison every later row.
    rc=0
    RUST_LOG=search_hybrid_context::rank=debug "$BIN" search "$query" -r "$repo" -m 100 -o json \
        >"$TMPDIR_EC/out.json" 2>"$TMPDIR_EC/trace.log" || rc=$?
    if [ "$rc" -ne 0 ]; then
        print_row "| $query | $repo | $want_name | ERR | - | - | - | search failed |"
        [ "$role" = "baseline" ] && fail=1
        continue
    fi

    parsed="$("$PY" "$TMPDIR_EC/parse.py" \
        "$TMPDIR_EC/trace.log" "$TMPDIR_EC/out.json" "$want_name" "$want_path" 2>/dev/null)"

    # Outside the unrestricted pool → retry path-restricted for the true cosine.
    if echo "$parsed" | grep -q "outside-pool"; then
        RUST_LOG=search_hybrid_context::rank=debug "$BIN" search "$query" -r "$repo" -m 100 \
            -p "$probe_dir" -o json \
            >"$TMPDIR_EC/probe_out.json" 2>"$TMPDIR_EC/probe_trace.log" || true
        prep="$("$PY" "$TMPDIR_EC/parse.py" \
            "$TMPDIR_EC/trace.log" "$TMPDIR_EC/out.json" "$want_name" "$want_path" \
            "$TMPDIR_EC/probe_trace.log" "$TMPDIR_EC/probe_out.json" 2>/dev/null)"
        [ -n "$prep" ] && parsed="$prep"
    fi

    if [ -z "$parsed" ]; then
        print_row "| $query | $repo | $want_name | ERR | - | - | - | parse error |"
        [ "$role" = "baseline" ] && fail=1
        continue
    fi

    IFS='|' read -r tcos tcutoff trank tpool tfinal tsource <<< "$parsed"
    if [ "$tcos" != "ERR" ] && [ "$tsource" != "search failed" ] && [ "$tsource" != "parse error" ]; then
        valid_rows=$((valid_rows + 1))
    fi

    if [ "$role" = "baseline" ]; then
        case "$tfinal" in
            "" | ABSENT | ERR)
                tfinal="**$tfinal**"
                fail=1
                ;;
        esac
    fi
    print_row "| $query | $repo | $want_name | $tcos | $tcutoff | $trank | $tfinal | $tsource |"
done

echo -e "${YELLOW}[3/3] Done${NC}"
echo "Decision rule: cosine <= cutoff (or '-') → outside the cosine window;"
echo "no query-time ranking change can reach it — fix embed_text or the model."

if [ -n "$OUT_FILE" ]; then
    if [ "$valid_rows" -gt 0 ]; then
        mv "$TMP_OUT" "$OUT_FILE"
    else
        rm -f "$TMP_OUT"
        echo -e "${RED}ERROR: All measured rows failed. Output file $OUT_FILE was not overwritten.${NC}" >&2
        exit 1
    fi
fi

if [ "$total_measured" -gt 0 ] && [ "$valid_rows" -eq 0 ]; then
    exit 1
fi

exit $fail
