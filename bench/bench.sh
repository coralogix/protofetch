#!/usr/bin/env bash

set -e

#
# Protofetch Benchmark Script
#
# Usage:
#   ./bench/bench.sh                  # Full benchmark (cold + warm, N runs each)
#   ./bench/bench.sh --cold           # Cold cache only
#   ./bench/bench.sh --warm           # Warm cache only
#   ./bench/bench.sh --runs 5         # 5 runs instead of default 1
#   ./bench/bench.sh --skip-build     # Skip cargo build (use existing binary)
#   ./bench/bench.sh --binary /path   # Use specific binary instead of building
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Configuration — uses local baseline copies (no external repo dependency)
PROTOFETCH_TOML="$SCRIPT_DIR/baseline/protofetch.toml"
PROTOFETCH_LOCK="$SCRIPT_DIR/baseline/protofetch.lock"
BENCH_DIR="$SCRIPT_DIR/workspace"
RESULTS_DIR="$SCRIPT_DIR/results"
CACHE_DIR="$SCRIPT_DIR/.bench-cache"
OUTPUT_DIR="$BENCH_DIR/proto-output"

# Defaults
RUNS=1
RUN_COLD=true
RUN_WARM=true
SKIP_BUILD=false
CUSTOM_BINARY=""
VERBOSE=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --cold)      RUN_WARM=false; shift ;;
        --warm)      RUN_COLD=false; shift ;;
        --runs)      RUNS="$2"; shift 2 ;;
        --skip-build) SKIP_BUILD=true; shift ;;
        --binary)    CUSTOM_BINARY="$2"; SKIP_BUILD=true; shift 2 ;;
        --verbose)   VERBOSE=true; shift ;;
        --help|-h)
            head -10 "$0" | tail -8
            exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color
BOLD='\033[1m'

log()  { echo -e "${BLUE}[bench]${NC} $*"; }
ok()   { echo -e "${GREEN}[bench]${NC} $*"; }
warn() { echo -e "${YELLOW}[bench]${NC} $*"; }

# ─── Setup ──────────────────────────────────────────────

setup_workspace() {
    log "Setting up benchmark workspace..."
    mkdir -p "$BENCH_DIR" "$RESULTS_DIR" "$CACHE_DIR"

    # Copy protofetch config from web-workspace
    cp "$PROTOFETCH_TOML" "$BENCH_DIR/protofetch.toml"
    cp "$PROTOFETCH_LOCK" "$BENCH_DIR/protofetch.lock"

    ok "Workspace ready at $BENCH_DIR"
    log "  protofetch.toml: $(wc -l < "$BENCH_DIR/protofetch.toml") lines"
    log "  protofetch.lock: $(wc -l < "$BENCH_DIR/protofetch.lock") lines"
    log "  Dependencies: $(grep -c '^\[' "$BENCH_DIR/protofetch.toml" | head -1) entries"
}

# ─── Build ──────────────────────────────────────────────

build_protofetch() {
    if [[ -n "$CUSTOM_BINARY" ]]; then
        PROTOFETCH_BIN="$CUSTOM_BINARY"
        log "Using custom binary: $PROTOFETCH_BIN"
        return
    fi

    # Detect cargo target dir (may be overridden by parent .cargo/config.toml)
    local cargo_target_dir
    cargo_target_dir=$(cd "$REPO_ROOT" && cargo metadata --no-deps --format-version 1 2>/dev/null \
        | python3 -c "import json,sys; print(json.load(sys.stdin)['target_directory'])" 2>/dev/null \
        || echo "$REPO_ROOT/target")
    PROTOFETCH_BIN="$cargo_target_dir/release/protofetch"

    if [[ "$SKIP_BUILD" == "true" ]] && [[ -f "$PROTOFETCH_BIN" ]]; then
        log "Skipping build (using existing binary)"
        return
    fi

    if [[ -f "$PROTOFETCH_BIN" ]]; then
        log "Using existing binary: $PROTOFETCH_BIN"
        return
    fi

    log "Building protofetch (release mode)..."
    local build_start
    build_start=$(python3 -c 'import time; print(int(time.time()*1000))')

    cd "$REPO_ROOT"
    cargo build --release 2>&1 | tail -5

    local build_end
    build_end=$(python3 -c 'import time; print(int(time.time()*1000))')
    local build_ms=$((build_end - build_start))

    if [[ ! -f "$PROTOFETCH_BIN" ]]; then
        echo ""
        warn "Binary not found at $PROTOFETCH_BIN"
        warn "This can happen if running inside a sandbox (e.g., Claude Code)."
        warn ""
        warn "Build manually first, then re-run:"
        warn "  cd $REPO_ROOT"
        warn "  cargo build --release"
        warn "  ./bench/bench.sh --skip-build"
        echo ""
        exit 1
    fi

    ok "Build complete in ${build_ms}ms"
    log "Binary: $PROTOFETCH_BIN ($(du -h "$PROTOFETCH_BIN" | cut -f1))"
}

# ─── Benchmark Helpers ──────────────────────────────────

clear_cache() {
    rm -rf "$CACHE_DIR"
    mkdir -p "$CACHE_DIR"
}

clear_output() {
    rm -rf "$OUTPUT_DIR"
}

# Run a single protofetch fetch and return duration in milliseconds
run_fetch() {
    local label="$1"
    local run_num="$2"

    clear_output

    local start_ms
    start_ms=$(python3 -c 'import time; print(int(time.time()*1000))')

    cd "$BENCH_DIR"
    # Global options MUST come before the subcommand
    local global_opts=(
        --cache-directory "$CACHE_DIR"
        --output-proto-directory "$OUTPUT_DIR"
        --lockfile-location "$BENCH_DIR/protofetch.lock"
        --module-location "$BENCH_DIR/protofetch.toml"
    )

    local log_file="$RESULTS_DIR/${label}_${run_num}.log"
    if [[ "$VERBOSE" == "true" ]]; then
        RUST_LOG=debug "$PROTOFETCH_BIN" "${global_opts[@]}" fetch --locked 2>&1 | tee "$log_file" >&2
    else
        "$PROTOFETCH_BIN" "${global_opts[@]}" fetch --locked 2>&1 | tee "$log_file" >&2
    fi

    local end_ms
    end_ms=$(python3 -c 'import time; print(int(time.time()*1000))')

    local duration_ms=$((end_ms - start_ms))
    echo "$duration_ms"
}

# ─── Benchmark Runners ──────────────────────────────────

run_cold_benchmark() {
    log ""
    log "${BOLD}━━━ Cold Cache Benchmark ($RUNS runs) ━━━${NC}"
    log "Each run clears the cache completely before fetching."
    log ""

    local durations=()

    for i in $(seq 1 "$RUNS"); do
        clear_cache
        log "  Run $i/$RUNS (cold)..."
        local ms
        ms=$(run_fetch "cold" "$i")
        durations+=("$ms")
        local secs
        secs=$(echo "scale=2; $ms / 1000" | bc)
        ok "  Run $i: ${secs}s (${ms}ms)"
    done

    # Calculate stats
    local total=0
    local min=${durations[0]}
    local max=${durations[0]}
    for d in "${durations[@]}"; do
        total=$((total + d))
        ((d < min)) && min=$d
        ((d > max)) && max=$d
    done
    local avg=$((total / RUNS))

    echo ""
    echo -e "${CYAN}${BOLD}Cold Cache Results:${NC}"
    echo -e "  Runs:    $RUNS"
    echo -e "  Average: ${BOLD}$(echo "scale=2; $avg / 1000" | bc)s${NC} (${avg}ms)"
    echo -e "  Min:     $(echo "scale=2; $min / 1000" | bc)s (${min}ms)"
    echo -e "  Max:     $(echo "scale=2; $max / 1000" | bc)s (${max}ms)"
    echo -e "  Range:   $(echo "scale=2; ($max - $min) / 1000" | bc)s"

    # Save results
    local timestamp
    timestamp=$(date +%Y%m%d_%H%M%S)
    local result_file="$RESULTS_DIR/cold_${timestamp}.json"
    cat > "$result_file" <<EOF
{
  "type": "cold",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "runs": $RUNS,
  "durations_ms": [$(IFS=,; echo "${durations[*]}")],
  "avg_ms": $avg,
  "min_ms": $min,
  "max_ms": $max,
  "binary": "$PROTOFETCH_BIN",
  "deps_count": $(grep -c '^\[' "$BENCH_DIR/protofetch.toml" || echo 0),
  "proto_files_output": $(find "$OUTPUT_DIR" -name "*.proto" 2>/dev/null | wc -l | tr -d ' ')
}
EOF
    log "Results saved: $result_file"
}

run_warm_benchmark() {
    log ""
    log "${BOLD}━━━ Warm Cache Benchmark ($RUNS runs) ━━━${NC}"
    log "Cache is warm (pre-populated). Measures resolution + copy time."
    log ""

    # Ensure cache is warm — skip warmup if cache already has content (e.g., cold benchmark just ran)
    if [[ -d "$CACHE_DIR/github.com" ]]; then
        ok "  Cache already warm (skipping warmup)."
    else
        local warmup_start
        warmup_start=$(python3 -c 'import time; print(int(time.time()*1000))')
        log "  Warming cache (cold fetch to populate)..."
        run_fetch "warmup" 0 > /dev/null
        local warmup_ms=$(( $(python3 -c 'import time; print(int(time.time()*1000))') - warmup_start ))
        ok "  Cache warm. (warmup took $(echo "scale=1; $warmup_ms / 1000" | bc)s)"
    fi
    log ""

    local durations=()

    for i in $(seq 1 "$RUNS"); do
        local run_wall_start
        run_wall_start=$(python3 -c 'import time; print(int(time.time()*1000))')
        log "  Run $i/$RUNS (warm)..."
        local ms
        ms=$(run_fetch "warm" "$i")
        local run_wall_ms=$(( $(python3 -c 'import time; print(int(time.time()*1000))') - run_wall_start ))
        durations+=("$ms")
        local secs
        secs=$(echo "scale=2; $ms / 1000" | bc)
        local wall_secs
        wall_secs=$(echo "scale=2; $run_wall_ms / 1000" | bc)
        ok "  Run $i: ${secs}s protofetch, ${wall_secs}s wall clock"
    done

    # Calculate stats
    local total=0
    local min=${durations[0]}
    local max=${durations[0]}
    for d in "${durations[@]}"; do
        total=$((total + d))
        ((d < min)) && min=$d
        ((d > max)) && max=$d
    done
    local avg=$((total / RUNS))

    echo ""
    echo -e "${CYAN}${BOLD}Warm Cache Results:${NC}"
    echo -e "  Runs:    $RUNS"
    echo -e "  Average: ${BOLD}$(echo "scale=2; $avg / 1000" | bc)s${NC} (${avg}ms)"
    echo -e "  Min:     $(echo "scale=2; $min / 1000" | bc)s (${min}ms)"
    echo -e "  Max:     $(echo "scale=2; $max / 1000" | bc)s (${max}ms)"
    echo -e "  Range:   $(echo "scale=2; ($max - $min) / 1000" | bc)s"

    # Save results
    local timestamp
    timestamp=$(date +%Y%m%d_%H%M%S)
    local result_file="$RESULTS_DIR/warm_${timestamp}.json"
    cat > "$result_file" <<EOF
{
  "type": "warm",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "runs": $RUNS,
  "durations_ms": [$(IFS=,; echo "${durations[*]}")],
  "avg_ms": $avg,
  "min_ms": $min,
  "max_ms": $max,
  "binary": "$PROTOFETCH_BIN",
  "deps_count": $(grep -c '^\[' "$BENCH_DIR/protofetch.toml" || echo 0),
  "proto_files_output": $(find "$OUTPUT_DIR" -name "*.proto" 2>/dev/null | wc -l | tr -d ' ')
}
EOF
    log "Results saved: $result_file"
}

# ─── Compare ────────────────────────────────────────────

print_comparison_hint() {
    echo ""
    echo -e "${YELLOW}${BOLD}To compare results after optimization:${NC}"
    echo "  1. Run benchmark:  ./bench/bench.sh"
    echo "  2. Make changes to src/"
    echo "  3. Run again:      ./bench/bench.sh"
    echo "  4. Compare:        ./bench/compare.sh"
    echo ""
}

# ─── Main ───────────────────────────────────────────────

main() {
    echo ""
    echo -e "${BOLD}╔══════════════════════════════════════╗${NC}"
    echo -e "${BOLD}║     Protofetch Benchmark Suite       ║${NC}"
    echo -e "${BOLD}╚══════════════════════════════════════╝${NC}"
    echo ""

    setup_workspace
    build_protofetch

    # Count dependencies
    local dep_count
    dep_count=$(grep -c '^\[' "$BENCH_DIR/protofetch.toml" 2>/dev/null || echo "?")
    log "Benchmarking with $dep_count dependencies, $RUNS runs each"

    if [[ "$RUN_COLD" == "true" ]]; then
        run_cold_benchmark
    fi

    if [[ "$RUN_WARM" == "true" ]]; then
        run_warm_benchmark
    fi

    print_comparison_hint
}

main
