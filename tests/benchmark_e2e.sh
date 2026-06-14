#!/usr/bin/env bash
# benchmark_e2e.sh — Full pipeline performance benchmarking with metrics capture.
#
# Runs the knot-indexer on real test fixtures and captures timing,
# memory, and throughput metrics in JSON format. Designed to be run
# after all correctness E2E tests pass.
#
# Usage:
#   ./tests/benchmark_e2e.sh [--focus <suite>] [--output-dir <path>]
#
# Options:
#   --focus <suite>     Only benchmark a specific language suite
#                       (rust_e2e, java_e2e, kotlin_e2e, python_e2e)
#   --output-dir <path> Directory for metrics output
#                       (default: /tmp/perf_results)
#
# Environment variables:
#   KNOT_NEO4J_URI       Neo4j URI (default: bolt://localhost:17687)
#   KNOT_NEO4J_USER      Neo4j user (default: neo4j)
#   KNOT_NEO4J_PASSWORD  Neo4j password (required via env or .env)
#   KNOT_QDRANT_URL      Qdrant URL (default: http://localhost:16334)
#   PERF_ITERATIONS      Number of benchmark repetitions (default: 3)
#
# Requirements: docker, docker-compose, cargo build --release

set -euo pipefail

# ─── Colors ──────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ─── Defaults ────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.e2e.yml"
FOCUS_SUITE="all"
OUTPUT_DIR="/tmp/perf_results"
PERF_ITERATIONS="${PERF_ITERATIONS:-3}"
TIMEOUT_SECONDS=120
HEALTH_CHECK_INTERVAL=2

# Parse CLI args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --focus)
            FOCUS_SUITE="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

# ─── Timestamped output paths ────────────────────────────────────────
RUN_DATE="$(date +%Y-%m-%d)"
RUN_ID="run_$(date +%H%M%S)_$$"
METRICS_DIR="${OUTPUT_DIR}/${RUN_DATE}/${RUN_ID}"
mkdir -p "$METRICS_DIR"

# ─── Report header ───────────────────────────────────────────────────
echo -e "${BLUE}${BOLD}========================================${NC}"
echo -e "${BLUE}${BOLD}knot Performance Benchmark Runner${NC}"
echo -e "${BLUE}${BOLD}========================================${NC}"
echo -e "${CYAN}Focus:       ${FOCUS_SUITE}${NC}"
echo -e "${CYAN}Iterations:  ${PERF_ITERATIONS}${NC}"
echo -e "${CYAN}Output dir:  ${METRICS_DIR}${NC}"
echo ""

# ─── Detect hardware ─────────────────────────────────────────────────
CPU_CORES="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo "unknown")"
RAM_GB="$(( $(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo 0) / 1024 / 1024 ))"
OS_INFO="$(uname -s 2>/dev/null || echo "unknown")"
COMMIT_HASH="$(cd "$PROJECT_ROOT" && git rev-parse --short HEAD 2>/dev/null || echo "unknown")"

echo -e "${CYAN}Hardware:    ${CPU_CORES} cores, ${RAM_GB} GB RAM${NC}"
echo -e "${CYAN}OS:          ${OS_INFO}${NC}"
echo -e "${CYAN}Commit:      ${COMMIT_HASH}${NC}"
echo ""

# ─── Ensure dependencies are installed ─────────────────────────────────
if [ ! -x "/usr/bin/time" ]; then
    echo -e "${RED}Error: /usr/bin/time is required but not found.${NC}"
    echo -e "Please install it (e.g., 'sudo apt-get install time' or 'brew install gnu-time')."
    exit 1
fi

# ─── Ensure binaries are built ───────────────────────────────────────
echo -e "${YELLOW}Building knot-indexer (release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --bin knot-indexer 2>&1 | grep -E "(Compiling knot|Finished)" || true
echo ""

# ─── Helper: wait for port ──────────────────────────────────────────
wait_for_port() {
    local port="$1"
    local service="$2"
    local container="$3"
    local elapsed=0

    echo -n "  Waiting for $service"
    while true; do
        if [ "$service" = "Neo4j" ]; then
            local status
            status=$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null || echo "starting")
            if [ "$status" = "healthy" ]; then
                echo -e " ${GREEN}ready${NC}"
                return 0
            fi
        else
            if nc -z localhost "$port" 2>/dev/null; then
                echo -e " ${GREEN}ready${NC}"
                return 0
            fi
        fi

        if [ $elapsed -ge $TIMEOUT_SECONDS ]; then
            echo -e " ${RED}TIMEOUT${NC}"
            return 1
        fi
        sleep $HEALTH_CHECK_INTERVAL
        elapsed=$((elapsed + HEALTH_CHECK_INTERVAL))
        echo -n "."
    done
}

# ─── Helper: start databases ────────────────────────────────────────
start_databases() {
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    rm -rf "$SCRIPT_DIR"/.e2e_* 2>/dev/null || true
    sleep 2
    docker compose -f "$COMPOSE_FILE" up -d
    wait_for_port 17687 "Neo4j" "knot_neo4j_e2e"
    wait_for_port 16334 "Qdrant" "knot_qdrant_e2e"
    sleep 3
}

# ─── Helper: stop databases ────────────────────────────────────────
stop_databases() {
    cd "$SCRIPT_DIR"
    docker compose -f "$COMPOSE_FILE" down -v 2>/dev/null || true
    rm -rf "$SCRIPT_DIR"/.e2e_* 2>/dev/null || true
}

# ─── Helper: run single iteration and capture metrics ────────────────
# Captures wall-clock time, peak memory, and parses log output for
# per-stage timing breakdown.
run_benchmark_iteration() {
    local suite_name="$1"
    local repo_dir="$2"
    local repo_name="$3"
    local iter="$4"

    local iter_metrics="${METRICS_DIR}/${suite_name}_iter_${iter}.json"
    local log_file="${METRICS_DIR}/${suite_name}_iter_${iter}.log"

    echo -e "  ${CYAN}Iteration ${iter}/${PERF_ITERATIONS}${NC}"

    # Time the indexer run. Use /usr/bin/time for peak RSS.
    local time_output
    time_output=$(export KNOT_REPO_PATH="$repo_dir"
        export KNOT_REPO_NAME="$repo_name"
        export KNOT_NEO4J_URI="bolt://localhost:17687"
        export KNOT_NEO4J_USER="neo4j"
        export KNOT_NEO4J_PASSWORD="e2e_test_password"
        export KNOT_QDRANT_URL="http://localhost:16334"
        export KNOT_QDRANT_COLLECTION="knot_perf_${suite_name}"
        RUST_LOG=info /usr/bin/time -f "TIME_ELAPSED=%e\nMEM_MAX_RSS=%M" \
            "${PROJECT_ROOT}/target/release/knot-indexer" \
            2>&1 | tee "$log_file")

    # Parse wall-clock time (in seconds, fractional)
    local wall_secs
    wall_secs=$(echo "$time_output" | grep -oP 'TIME_ELAPSED=\K[0-9.]+' | tail -1)

    # Parse peak RSS (in KB) from /usr/bin/time
    local mem_kb
    mem_kb=$(echo "$time_output" | grep -oP 'MEM_MAX_RSS=\K[0-9]+' | tail -1)

    # Parse entity count from logs
    local entities_total
    entities_total=$(echo "$time_output" | grep -oP 'Total entities parsed:\s*\K[0-9]+' | tail -1 || echo "0")

    # Parse stage timings from log markers (adjust patterns as needed)
    # Stage 2 (parse) - look for timing in logs
    local parse_ms="0"
    local embed_ms="0"
    local ingest_ms="0"
    local resolve_ms="0"
    local neo4j_queries="0"

    # Attempt to extract per-stage timings from structured log output
    parse_ms=$(echo "$time_output" | grep -oP 'Parsing completed in \K[0-9]+(?=ms)' | tail -1 || echo "0")
    embed_ms=$(echo "$time_output" | grep -oP 'Embedding completed in \K[0-9]+(?=ms)' | tail -1 || echo "0")
    ingest_ms=$(echo "$time_output" | grep -oP 'Ingestion completed in \K[0-9]+(?=ms)' | tail -1 || echo "0")
    resolve_ms=$(echo "$time_output" | grep -oP 'Relationship resolution completed in \K[0-9]+(?=ms)' | tail -1 || echo "0")
    neo4j_queries=$(echo "$time_output" | grep -oP 'Neo4j queries:\s*\K[0-9]+' | tail -1 || echo "0")

    # Convert wall time to ms
    local wall_ms
    wall_ms=$(awk "BEGIN {printf \"%.0f\", ${wall_secs:-0} * 1000}")

    # Convert mem to MB
    local mem_mb
    mem_mb=$(awk "BEGIN {printf \"%.0f\", ${mem_kb:-0} / 1024}")

    # Compute entities per second
    local eps
    eps=$(awk "BEGIN {printf \"%.0f\", ${entities_total:-0} / ((${wall_secs:-1}) + 0.001)}")

    cat > "$iter_metrics" <<JSON
{
  "suite": "${suite_name}",
  "iteration": ${iter},
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "commit": "${COMMIT_HASH}",
  "total_ms": ${wall_ms},
  "stage_timings": {
    "parse": { "ms": ${parse_ms}, "entities_total": ${entities_total} },
    "embed": { "ms": ${embed_ms} },
    "ingest": { "ms": ${ingest_ms}, "neo4j_queries": ${neo4j_queries} },
    "resolve": { "ms": ${resolve_ms} }
  },
  "memory_peak_mb": ${mem_mb},
  "entities_total": ${entities_total},
  "entities_per_sec": ${eps}
}
JSON

    echo -e "    ${GREEN}Total: ${wall_ms}ms | Mem: ${mem_mb}MB | Entities: ${entities_total}${NC}"
}

# ─── Helper: aggregate iterations ───────────────────────────────────
aggregate_iterations() {
    local suite_name="$1"
    local output_file="${METRICS_DIR}/${suite_name}_aggregated.json"

    # Collect values
    local total_ms_list=()
    local mem_list=()
    local entities_list=()

    for i in $(seq 1 $PERF_ITERATIONS); do
        local iter_file="${METRICS_DIR}/${suite_name}_iter_${i}.json"
        if [ -f "$iter_file" ]; then
            local t m e
            t=$(python3 -c "import json; d=json.load(open('$iter_file')); print(d['total_ms'])" 2>/dev/null || echo "0")
            m=$(python3 -c "import json; d=json.load(open('$iter_file')); print(d['memory_peak_mb'])" 2>/dev/null || echo "0")
            e=$(python3 -c "import json; d=json.load(open('$iter_file')); print(d['entities_total'])" 2>/dev/null || echo "0")
            total_ms_list+=("$t")
            mem_list+=("$m")
            entities_list+=("$e")
        fi
    done

    # Compute median (basic: sort + pick middle)
    local median_time median_mem median_entities
    median_time=$(printf '%s\n' "${total_ms_list[@]}" | sort -n | awk '{a[NR]=$0} END{print a[int((NR+1)/2)]}')
    median_mem=$(printf '%s\n' "${mem_list[@]}" | sort -n | awk '{a[NR]=$0} END{print a[int((NR+1)/2)]}')
    median_entities=$(printf '%s\n' "${entities_list[@]}" | sort -n | awk '{a[NR]=$0} END{print a[int((NR+1)/2)]}')

    cat > "$output_file" <<JSON
{
  "suite": "${suite_name}",
  "iterations": ${PERF_ITERATIONS},
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "commit": "${COMMIT_HASH}",
  "hardware": {
    "cpu_cores": "${CPU_CORES}",
    "ram_gb": "${RAM_GB}",
    "os": "${OS_INFO}"
  },
  "total_ms_median": ${median_time:-0},
  "memory_peak_mb_median": ${median_mem:-0},
  "entities_total_median": ${median_entities:-0}
}
JSON

    echo -e "  ${GREEN}Median total: ${median_time}ms | Mem: ${median_mem}MB${NC}"
}

# ─── Helper: run a single suite ─────────────────────────────────────
benchmark_suite() {
    local suite_name="$1"
    local test_files_dir="$2"
    local repo_name="$3"

    echo -e "\n${YELLOW}${BOLD}[Suite: ${suite_name}]${NC}"

    # Create isolated repo directory
    local tmp_repo
    tmp_repo="$(mktemp -d)"
    cp "$test_files_dir"/* "$tmp_repo/" 2>/dev/null || true

    echo -e "  ${CYAN}Files: $(find "$tmp_repo" -type f | wc -l)${NC}"

    for i in $(seq 1 "$PERF_ITERATIONS"); do
        start_databases
        run_benchmark_iteration "$suite_name" "$tmp_repo" "$repo_name" "$i"
        stop_databases
    done

    aggregate_iterations "$suite_name"
    rm -rf "$tmp_repo"

    echo -e "  ${GREEN}Suite ${suite_name} complete${NC}"
}

# ─── Main benchmark logic ────────────────────────────────────────────

# Use dotenvy to load .env if available
if [ -f "$PROJECT_ROOT/.env" ]; then
    set -a
    # shellcheck source=/dev/null
    . "$PROJECT_ROOT/.env"
    set +a
fi

# Trap cleanup on exit
cleanup() {
    echo -e "\n${YELLOW}Cleaning up benchmark environment...${NC}"
    stop_databases
    echo -e "${GREEN}Cleanup complete${NC}"
}
trap cleanup EXIT INT TERM

# Build once
echo -e "${YELLOW}Building knot-indexer (release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --bin knot-indexer 2>&1 | tail -3
echo ""

# Run benchmarks
TESTING_FILES="$SCRIPT_DIR/testing_files"

case "${FOCUS_SUITE}" in
    rust_e2e|rust)
        benchmark_suite "rust_e2e" "${TESTING_FILES}" "perf_rust_e2e"
        ;;
    java_e2e|java)
        benchmark_suite "java_e2e" "${TESTING_FILES}" "perf_java_e2e"
        ;;
    kotlin_e2e|kotlin)
        benchmark_suite "kotlin_e2e" "${TESTING_FILES}" "perf_kotlin_e2e"
        ;;
    python_e2e|python)
        benchmark_suite "python_e2e" "${TESTING_FILES}" "perf_python_e2e"
        ;;
    all|*)
        # These need specific file sets per language
        benchmark_suite "rust_e2e" "${TESTING_FILES}" "perf_rust_e2e"
        benchmark_suite "java_e2e" "${TESTING_FILES}" "perf_java_e2e"
        benchmark_suite "kotlin_e2e" "${TESTING_FILES}" "perf_kotlin_e2e"
        benchmark_suite "python_e2e" "${TESTING_FILES}" "perf_python_e2e"
        ;;
esac

# ─── Generate aggregated summary ─────────────────────────────────────
SUMMARY_FILE="${METRICS_DIR}/aggregated.json"
echo -e "\n${BLUE}${BOLD}Generating aggregated summary...${NC}"

cat > "$SUMMARY_FILE" <<JSON
{
  "run_id": "${RUN_ID}",
  "date": "${RUN_DATE}",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "commit": "${COMMIT_HASH}",
  "hardware": {
    "cpu_cores": "${CPU_CORES}",
    "ram_gb": "${RAM_GB}",
    "os": "${OS_INFO}"
  },
  "suites": {}
}
JSON

# Merge suite results into summary (basic: just list files)
suite_list=$(find "$METRICS_DIR" -name '*_aggregated.json' -type f 2>/dev/null || true)

if [ -n "$suite_list" ]; then
    echo -e "${GREEN}Results saved to:${NC}"
    echo -e "  ${CYAN}${METRICS_DIR}${NC}"
    echo ""
    echo -e "${GREEN}Suite aggregated files:${NC}"
    for f in $suite_list; do
        echo -e "  ${CYAN}$(basename "$f")${NC}"
    done
fi

echo ""
echo -e "${BLUE}${BOLD}========================================${NC}"
echo -e "${GREEN}${BOLD}Performance benchmarks complete${NC}"
echo -e "${BLUE}${BOLD}========================================${NC}"

exit 0
