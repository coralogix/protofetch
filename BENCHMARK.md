# Protofetch Benchmark

Benchmark harness for measuring protofetch performance using a real-world configuration (59 dependencies, 906 proto files).

## Running

### Prerequisites

- Rust 1.75+
- SSH key in ssh-agent (`ssh-add`)
- Access to repos in `bench/baseline/protofetch.toml`

### Measure

```bash
cargo build --release
./bench/bench.sh
```

### Options

```bash
./bench/bench.sh --cold              # Cold cache only
./bench/bench.sh --warm              # Warm cache only
./bench/bench.sh --runs 3            # Multiple runs
./bench/bench.sh --binary /path      # Test a specific binary

RAYON_NUM_THREADS=10 ./bench/bench.sh  # Custom thread count (default 100)
```

### Output

Results saved to `bench/results/`:
- `cold_*.json`, `warm_*.json` — timing data
- `cold_1.log`, `warm_1.log` — detailed protofetch output with `[cached]`/`[fetch]` markers

### Benchmark Data

`bench/baseline/` contains a static copy of `protofetch.toml` and `protofetch.lock` from the cx-web-workspace repository (59 dependencies). This decouples the benchmark from any external repository.
