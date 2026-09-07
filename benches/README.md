# TaraBG Benchmarks

Benchmarks compare TaraBG and HTSlib `bgzip` with both binaries built and run
natively in the same OS. The gold standard is native Linux hardware. Results
captured across different OSes, through interop layers (e.g. a Windows binary
run from WSL), containers on different hosts, or network filesystems must not
be compared — and no performance claim is accepted without reproducible
commands plus same-environment evidence (see `ROADMAP.md` Phase 7).

## Setup

```bash
# Native Linux
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
- Level numbers are only comparable between tools that share a deflate
  backend. TaraBG (since v0.2.0) and bgzip 1.19 both use libdeflate with 1:1
  level mapping, so same-flag comparison is meaningful there. Against other
  bgzip builds, also compare at equal output size.

## Reproducing the v0.2.0 comparison (WSL2 Ubuntu, no sudo)

The v0.2.0 numbers were taken on WSL2 Ubuntu 24.04 (noble, 16 vCPUs) with
**both** binaries native Linux on the same ext4 filesystem — a valid
same-OS comparison, but virtualized hardware, so treat absolute numbers as
indicative. The box had no `sudo`, no C toolchain, and no Rust; the steps
below work around that. On a normal machine, prefer
`sudo apt install tabix build-essential` plus `rustup`.

### 1. Run bgzip without installing it

Ubuntu's `tabix` package ships `bgzip`. Download and unpack it as a user
(no root needed), resolving its one non-system dependency the same way:

```bash
mkdir -p ~/bgzip-pkg && cd ~/bgzip-pkg
apt-get download tabix libhtscodecs2
mkdir root
dpkg-deb -x tabix_1.19+ds-1.1build3_amd64.deb root
dpkg-deb -x libhtscodecs2_1.6.0-1build1_amd64.deb root
export LD_LIBRARY_PATH=$HOME/bgzip-pkg/root/usr/lib/x86_64-linux-gnu
./root/usr/bin/bgzip --version   # bgzip (htslib) 1.19
```

This works because the remaining libraries (`libdeflate`, `liblzma`,
`libbz2`, `zlib1g`, `libc6`) already ship with stock Ubuntu — confirm with
`ldconfig -p | grep -E 'libdeflate|liblzma|libbz2|libz\.'`.
Set `BGZIP=$HOME/bgzip-pkg/root/usr/bin/bgzip` for the commands below
(keep `LD_LIBRARY_PATH` exported in the same shell).

### 2. Build TaraBG for Linux

```bash
cp -r /mnt/d/tarabg ~/tarabgz   # or git clone on the Linux side
cd ~/tarabgz
cargo build --release            # needs cc + cmake for libdeflate-sys
./target/release/tarabg --version
```

### 3. Generate the deterministic input

```bash
python3 - <<'EOF'
bases = 'ACGT'
with open('sample.vcf', 'w') as f:
    f.write('##fileformat=VCFv4.2\n')
    for i in range(800000):
        f.write(
            f"chr1\t{1000 + i}\trs{i}\t{bases[i % 4]}\t{bases[(i + 1) % 4]}"
            f"\t30\tPASS\tDP=100;AF=0.5;MQ=60;QD=2.5;FS=1.0"
            f"\tGT:AD:DP:GQ:PL\t0/1:50,50:100:99:100,0,100\n"
        )
EOF
ls -la sample.vcf   # ~87 MB
sha256sum sample.vcf
```

### 4. Run the matrix (best-of-3 wall time, verify every byte)

```bash
TARABG=~/tarabgz/target/release/tarabg
for lvl in 1 6 9; do for thr in 1 4; do
  for tool in "$BGZIP" "$TARABG"; do
    name=$(basename "$tool"); gz="$name-$thr-$lvl.gz"; dec="$name-$thr-$lvl.dec"
    time (for i in 1 2 3; do "$tool" -@ "$thr" -l "$lvl" -c sample.vcf > "$gz"; done)
    "$tool" -d -c "$gz" > "$dec"
    sha256sum "$dec"   # must match the input hash
  done
  "$BGZIP" -t "tarabg-4-$lvl.gz" && "$TARABG" -t "bgzip-4-$lvl.gz"  # cross-acceptance
done
```

### 5. v0.2.0 results (87.0 MB VCF, best-of-3)

| lvl | thr | bgzip compr | tarabg compr | bgzip size | tarabg size | bgzip decomp | tarabg decomp |
|-----|-----|-------------|--------------|------------|-------------|--------------|---------------|
| 1   | 1   | 0.14–0.38s  | 0.11–0.22s   | 4.35 MB    | 4.35 MB     | ~0.10s       | ~0.09s        |
| 1   | 4   | 0.22–0.38s  | ~0.12s       | 4.35 MB    | 4.35 MB     | ~0.10s       | ~0.09s        |
| 6   | 1   | 0.43–0.50s  | 0.30–0.39s  | 3.85 MB    | 4.04 MB     | ~0.08s       | ~0.10s        |
| 6   | 4   | 0.18–0.38s  | 0.15–0.19s   | 3.85 MB    | 4.04 MB     | ~0.08s       | ~0.10s        |
| 9   | 1   | ~12–13.5s   | 0.76–1.5s    | 3.74 MB    | 3.74 MB     | ~0.1s        | ~0.1–0.3s     |
| 9   | 4   | ~4.3–5.4s   | 0.37–0.63s   | 3.74 MB    | 3.74 MB     | ~0.1s        | ~0.1s         |

Ranges span three full runs — bgzip's own numbers move run-to-run on shared
hardware, so quote the conservative end. Headline: `-l 9` compresses ~8x
faster at identical output size; `-l 6` ~1.2–1.4x faster at ~5% larger size;
decompression is at parity (bgzip ~1.2x ahead at `-l 6`); `tarabg -t` was the
fastest integrity check in most cells.

### Caveats on this evidence

- Single synthetic input type; real VCFs and incompressible data will differ.
- WSL2 is virtualized — absolute throughput won't match bare metal, though
  both tools shared the same conditions back-to-back.
- `perf stat -d` hotspot profiling was not possible here (no PMU in WSL2);
  a native-Linux `perf` pass is still the right gate before optimizing
  further.

## HTSlib downstream workload

Use `htslib-workload.sh` to have one HTSlib build validate, identify, index,
and region-query VCF BGZF produced by both HTSlib bgzip and TaraBG:

```bash
benches/htslib-workload.sh INPUT.vcf target/release/tarabg HTSLIB_BIN_DIR OUTPUT_DIR
```

The harness also compares headers, query results, and decompressed SHA-256
hashes, and records compression/index timing and peak RSS. It refuses to
overwrite an existing result directory. The 2026-09-07 audit is recorded in
`compatibility/htslib-workload-audit-2026-09-07.md`.
