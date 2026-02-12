# Protofetch Benchmark Suite

Benchmark harness for measuring protofetch performance using the cx-web-workspace's real protofetch configuration (53 dependencies).

## Quick Start

```bash
# Full benchmark (cold + warm cache, 3 runs each)
./bench/bench.sh

# Just warm cache (faster, good for iteration)
./bench/bench.sh --warm --runs 5

# Skip rebuild (reuse last binary)
./bench/bench.sh --skip-build

# Compare a custom/external binary
./bench/bench.sh --binary /path/to/protofetch
```

## Optimization Workflow

```bash
# 1. Baseline measurement
./bench/bench.sh
# Results saved to bench/results/cold_YYYYMMDD_HHMMSS.json etc.

# 2. Make changes to src/
vim src/fetch.rs

# 3. Re-benchmark (rebuilds automatically)
./bench/bench.sh

# 4. Compare before vs after
./bench/compare.sh
```

## Options

| Flag | Description |
|------|-------------|
| `--cold` | Cold cache only (clears cache each run) |
| `--warm` | Warm cache only (pre-populated cache) |
| `--runs N` | Number of runs per benchmark (default: 3) |
| `--skip-build` | Skip `cargo build --release` |
| `--binary PATH` | Use specific binary instead of building |
| `--verbose` | Enable RUST_LOG=debug |

## Output

Results are saved as JSON in `bench/results/`:

```json
{
  "type": "cold",
  "timestamp": "2026-02-11T10:00:00Z",
  "runs": 3,
  "durations_ms": [45000, 43000, 44000],
  "avg_ms": 44000,
  "min_ms": 43000,
  "max_ms": 45000,
  "deps_count": 53,
  "proto_files_output": 906
}
```

## Directory Structure

```
bench/
├── bench.sh          # Main benchmark script
├── README.md         # This file
├── workspace/        # (generated) protofetch.toml + lock copied here
├── results/          # (generated) JSON result files
└── .bench-cache/     # (generated) isolated protofetch cache
```

## What's Measured

- **Cold cache**: Full git clone + fetch + copy for all 53 repos (network-bound)
- **Warm cache**: Resolution + worktree + copy from local cache (I/O-bound)

## Tips

- Cold cache benchmarks are network-dependent — run on a stable connection
- Warm cache benchmarks are more reproducible for measuring code changes
- Use `--warm --runs 5` for quick iteration during optimization
- The `--binary` flag lets you compare the installed npm version vs your local build
