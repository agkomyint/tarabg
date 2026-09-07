# TaraBG Benchmarks

Benchmarks compare TaraBG and HTSlib `bgzip` only on native Linux hardware with both
binaries built and run natively. Results captured on Windows, WSL, Docker, or cloud VMs
may not reflect real performance.

## Setup

```bash
# Native Linux only
sudo apt install tabix linux-tools-common
cargo build --release
```

## Run

```bash
bash benches/benchmark.sh sample.vcf target/release/tarabg
```

Outputs `benches/results/results.csv` with columns:
`tool,level,threads,wall_s,cpu_s,rss_kb,compressed_bytes,throughput_mb_s`

## Interpreting results

- Compare only rows with the same `level` and `threads`.
- Validate output with `bgzip -t` before trusting any result.
- Discard results from runs with different input sizes or paths.
