#!/usr/bin/env bash
# compare_perf_metrics.sh — Compare current benchmark metrics against baseline.
#
# Reads benchmark results from an output directory and compares against
# a baseline JSON file. Fails CI if any metric regresses beyond configured
# tolerance thresholds.
#
# Usage:
#   ./scripts/compare_perf_metrics.sh <metrics_dir> [baseline_file]
#
# Arguments:
#   metrics_dir    Path to directory with aggregated benchmark JSON files
#   baseline_file  Path to .perf_metrics/baseline.json (default: ./.perf_metrics/baseline.json)
#
# Exit codes:
#   0 — All metrics within tolerance
#   1 — One or more metrics exceeded tolerance (regression detected)
#   2 — Usage error or missing files

set -euo pipefail

# ─── Colors ──────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ─── Parse arguments ────────────────────────────────────────────────
METRICS_DIR="${1:-}"
BASELINE_FILE="${2:-.perf_metrics/baseline.json}"

if [ -z "$METRICS_DIR" ]; then
    echo -e "${RED}Usage: $0 <metrics_dir> [baseline_file]${NC}"
    exit 2
fi

if [ ! -d "$METRICS_DIR" ]; then
    echo -e "${RED}ERROR: Metrics directory not found: ${METRICS_DIR}${NC}"
    echo ""
    echo -e "${YELLOW}Run benchmarks first to generate metrics:${NC}"
    echo -e "  ${CYAN}./tests/benchmark_e2e.sh --output-dir ${METRICS_DIR}${NC}"
    exit 2
fi

if [ ! -f "$BASELINE_FILE" ]; then
    echo -e "${YELLOW}WARNING: Baseline file not found: ${BASELINE_FILE}${NC}"
    echo -e "${YELLOW}Skipping comparison (no baseline to compare against).${NC}"
    echo -e "${YELLOW}Run the benchmarks at least once on main to establish a baseline.${NC}"
    exit 0
fi

# ─── Header ─────────────────────────────────────────────────────────
echo -e "${BLUE}${BOLD}========================================${NC}"
echo -e "${BLUE}${BOLD}Performance Regression Check${NC}"
echo -e "${BLUE}${BOLD}========================================${NC}"
echo -e "${CYAN}Metrics dir:  ${METRICS_DIR}${NC}"
echo -e "${CYAN}Baseline:     ${BASELINE_FILE}${NC}"
echo ""

# ─── Extract values from JSON (using python3 or fallback) ───────────
has_python=false
if command -v python3 &>/dev/null; then
    has_python=true
fi

json_get() {
    local file="$1"
    local key="$2"
    if $has_python; then
        python3 -c "import json,sys; d=json.load(open('$file')); print(d$key)" 2>/dev/null || echo "0"
    else
        # Fallback: grep-based extraction
        local val
        val=$(grep -oP '"total_ms_median":\s*\K[0-9]+' "$file" 2>/dev/null | tail -1 || echo "0")
        echo "${val:-0}"
    fi
}

# ─── Read tolerances ─────────────────────────────────────────────────
TOLERANCE_FILE=".perf_metrics/threshold_tolerances.json"
TIME_TOLERANCE_PCT=5.0
MEM_TOLERANCE_PCT=10.0
STAGE_TOLERANCE_PCT=10.0

if [ -f "$TOLERANCE_FILE" ]; then
    if $has_python; then
        TIME_TOLERANCE_PCT=$(python3 -c "import json; d=json.load(open('$TOLERANCE_FILE')); print(d.get('total_time_regression_pct', 5))" 2>/dev/null || echo "5")
        MEM_TOLERANCE_PCT=$(python3 -c "import json; d=json.load(open('$TOLERANCE_FILE')); print(d.get('memory_regression_pct', 10))" 2>/dev/null || echo "10")
        STAGE_TOLERANCE_PCT=$(python3 -c "import json; d=json.load(open('$TOLERANCE_FILE')); print(d.get('stage_regression_pct', 10))" 2>/dev/null || echo "10")
    fi
fi

echo -e "${CYAN}Tolerances:  Time ±${TIME_TOLERANCE_PCT}%  |  Memory ±${MEM_TOLERANCE_PCT}%  |  Stage ±${STAGE_TOLERANCE_PCT}%${NC}"
echo ""

# ─── Compare each suite ─────────────────────────────────────────────
FAILURES=0
WARNINGS=0
SUITES_CHECKED=0

# Find all aggregated suite files
while IFS= read -r -d '' suite_file; do
    suite_name=$(basename "$suite_file" _aggregated.json)
    SUITES_CHECKED=$((SUITES_CHECKED + 1))

    echo -e "${YELLOW}${BOLD}[Suite: ${suite_name}]${NC}"

    # Get current metrics
    cur_total=$(json_get "$suite_file" "['total_ms_median']")
    cur_mem=$(json_get "$suite_file" "['memory_peak_mb_median']")

    # Get baseline metrics if available
    baseline_total=$(json_get "$BASELINE_FILE" "['suite_results']['${suite_name}']['total_ms']")
    baseline_mem=$(json_get "$BASELINE_FILE" "['suite_results']['${suite_name}']['memory_peak_mb']")

    # Skip if no baseline data for this suite
    if [ "$baseline_total" = "0" ] || [ "$baseline_total" = "null" ]; then
        echo -e "  ${CYAN}No baseline data — skipping${NC}"
        continue
    fi

    # Compare total time
    time_pct_change=$(awk "BEGIN {printf \"%.1f\", ((${cur_total} - ${baseline_total}) / ${baseline_total}) * 100}")
    time_abs_change=$(awk "BEGIN {printf \"%.0f\", ${cur_total} - ${baseline_total}}")

    time_status="${GREEN}✓${NC}"
    time_label="within tolerance"

    if [ "$(awk "BEGIN {print (${time_pct_change} > ${TIME_TOLERANCE_PCT} ? 1 : 0)}")" = "1" ]; then
        time_status="${RED}✗${NC}"
        time_label="REGRESSION"
        FAILURES=$((FAILURES + 1))
    elif [ "$(awk "BEGIN {print (${time_pct_change} < -${TIME_TOLERANCE_PCT} ? 1 : 0)}")" = "1" ]; then
        time_status="${GREEN}✓${NC}"
        time_label="improvement"
    fi

    echo -e "  Total Time:   ${cur_total}ms (baseline: ${baseline_total}ms) ${time_status} ${time_pct_change}% (${time_abs_change}ms) — ${time_label}"

    # Compare memory
    mem_pct_change=$(awk "BEGIN {printf \"%.1f\", ((${cur_mem} - ${baseline_mem}) / ${baseline_mem}) * 100}")

    mem_status="${GREEN}✓${NC}"
    mem_label="within tolerance"

    if [ "$(awk "BEGIN {print (${mem_pct_change} > ${MEM_TOLERANCE_PCT} ? 1 : 0)}")" = "1" ]; then
        mem_status="${RED}✗${NC}"
        mem_label="REGRESSION"
        FAILURES=$((FAILURES + 1))
    elif [ "$(awk "BEGIN {print (${mem_pct_change} > 0 ? 1 : 0)}")" = "1" ]; then
        mem_status="${YELLOW}⚠${NC}"
        mem_label="marginal"
        WARNINGS=$((WARNINGS + 1))
    fi

    echo -e "  Memory:       ${cur_mem}MB (baseline: ${baseline_mem}MB) ${mem_status} ${mem_pct_change}% — ${mem_label}"

    echo ""

done < <(find "$METRICS_DIR" -name '*_aggregated.json' -type f -print0 2>/dev/null || true)

# ─── Summary ────────────────────────────────────────────────────────
echo -e "${BLUE}${BOLD}========================================${NC}"
echo -e "${BLUE}${BOLD}Summary${NC}"
echo -e "${BLUE}${BOLD}========================================${NC}"
echo -e "Suites checked:  ${SUITES_CHECKED}"
echo -e "Warnings:        ${WARNINGS}"
echo -e "Failures:        ${FAILURES}"
echo ""

if [ "$FAILURES" -gt 0 ]; then
    echo -e "${RED}${BOLD}RESULT: ✗ REGRESSION DETECTED${NC}"
    echo -e "${RED}${FAILURES} metric(s) exceeded tolerance thresholds.${NC}"
    echo ""
    echo -e "${YELLOW}Investigation steps:${NC}"
    echo -e "  1. Check git log for recent changes"
    echo -e "  2. Re-run benchmark locally: ${CYAN}./tests/benchmark_e2e.sh --focus <suite>${NC}"
    echo -e "  3. Review per-stage timings in ${METRICS_DIR}/"
    echo -e "  4. Update baseline if regression is intentional:"
    echo -e "     ${CYAN}cp ${METRICS_DIR}/aggregated.json ${BASELINE_FILE}${NC}"
    exit 1
elif [ "$WARNINGS" -gt 0 ]; then
    echo -e "${YELLOW}${BOLD}RESULT: ⚠ MARGINAL${NC}"
    echo -e "${YELLOW}${WARNINGS} metric(s) showed slight increase (within tolerance).${NC}"
    exit 0
else
    echo -e "${GREEN}${BOLD}RESULT: ✓ PASS${NC}"
    echo -e "${GREEN}All metrics within tolerance thresholds.${NC}"
    exit 0
fi
