# TaraBG

TaraBG is a clean-room Rust implementation of BGZF (Blocked GZIP), designed
for interoperability with the common `bgzip` data path.

**v0.1 status:** TaraBG writes standards-compliant BGZF, reads BGZF, validates
each block's CRC32 and ISIZE, and supports `-c`, `-d`, `-t`, `-l 0..9`, `-@`,
stdin, and stdout. It is not yet a complete CLI replacement for every HTSlib
`bgzip` option: index creation, `-b`/`-s` random reads, and the remaining
bgzip flags are planned work. Do not represent v0.1 as feature-complete bgzip.

## Install

### Prebuilt Windows release

Download `tarabg-v0.1.0-windows-x86_64.zip` from the GitHub release, extract
it, then either place `tarabg.exe` on `PATH` or invoke it directly:

```powershell
Expand-Archive .\tarabg-v0.1.0-windows-x86_64.zip -DestinationPath .\tarabg
$env:Path += ";$PWD\tarabg"
tarabg --version
```

### Build from source (Linux, macOS, or Windows)

Install the stable Rust toolchain, then:

```bash
git clone https://github.com/agkomyint/tarabg.git
cd tarabg
cargo build --release
# Optional: copy target/release/tarabg somewhere on PATH
```

On Debian/Ubuntu, install HTSlib's comparison tool with `sudo apt install tabix`.

## Common workflow

```bash
# compress to stdout
tarabg -@ 4 -l 6 -c input.vcf > input.vcf.gz

# validate and decompress
tarabg -t input.vcf.gz
tarabg -d -c input.vcf.gz > restored.vcf

# independent compatibility check
bgzip -t input.vcf.gz
bgzip -d -c input.vcf.gz | cmp input.vcf -
```

TaraBG output does not need to byte-match `bgzip` output. Compatibility means
valid BGZF, exact decompressed content, and acceptance by HTSlib tools.

## Tests

```bash
cargo test
```

The suite tests all compression levels, rejects corrupt CRCs, and—when `bgzip`
is installed—verifies both TaraBG → bgzip and bgzip → TaraBG decompression.

## Fair, neck-to-neck benchmarks

Benchmark only native binaries in the same operating-system environment. Do
not time a Windows executable through WSL, a container boundary, or a network
filesystem. Build TaraBG and install `bgzip` on the same Linux host:

```bash
sudo apt install tabix linux-tools-common
cargo build --release
bash benches/benchmark.sh sample.vcf target/release/tarabg
```

The harness refuses Windows binaries and emits
`benches/results/results.csv` for levels `1,6,9` and thread counts
`1,2,4,8,16`. It records compression/decompression throughput, wall time,
CPU time, peak RSS, and compressed size. Validate the outputs before comparing:

```bash
bgzip -t benches/results/tarabg-1-6.gz
perf stat -d target/release/tarabg -@ 1 -l 6 -c sample.vcf > /dev/null
```

## Development rule

Correct → compatible → profile → change one thing → benchmark → keep or revert.
No performance claim is accepted until compatibility tests and an equal-environment
benchmark both pass.

## License

MIT licensed; see [LICENSE-MIT](LICENSE-MIT).
