# TaraBG

TaraBG is a clean-room Rust implementation of BGZF (Blocked GZIP Format),
designed for interoperability with HTSlib `bgzip`.

> [!IMPORTANT]
> **v0.1 status:** TaraBG is not yet a complete bgzip replacement. See the
> "Supported options" table below for what is and is not implemented.

## Install

### Build from source (Linux, macOS, or Windows)

```bash
git clone https://github.com/agkomyint/tarabg.git
cd tarabg
cargo build --release
# Optional: copy target/release/tarabg somewhere on PATH
```

On Debian/Ubuntu, install HTSlib for interoperability testing:

```bash
sudo apt install tabix
```

### Prebuilt Windows release

Download `tarabg-v0.1.0-windows-x86_64.zip` from the GitHub release, extract
it, then either place `tarabg.exe` on `PATH` or invoke it directly:

```powershell
Expand-Archive .\tarabg-v0.1.0-windows-x86_64.zip -DestinationPath .\tarabg
$env:Path += ";$PWD\tarabg"
tarabg --version
```

## Supported options

| Short | Long            | Description                                                   | Status      |
|-------|-----------------|---------------------------------------------------------------|-------------|
| `-c`  | `--stdout`      | Write to stdout; keep input file                              | ✅ Done     |
| `-d`  | `--decompress`  | Decompress BGZF input                                         | ✅ Done     |
| `-t`  | `--test`        | Integrity test; no output written                             | ✅ Done     |
| `-l`  | `--level`       | Compression level 0–9 (default 6)                             | ✅ Done     |
| `-@`  | `--threads`     | Number of compression worker threads (default 1)              | ✅ Done     |
| `-k`  | `--keep`        | Keep (do not remove) input file after file-mode operation     | ✅ Done     |
| `-f`  | `--force`       | Overwrite existing output without prompting                   | ✅ Done     |
| `-o`  |                 | Write output to FILE instead of default path                  | ✅ Done     |
| `-i`  | `--index`       | Create `.gzi` index while compressing                         | ✅ Done     |
| `-I`  |                 | Explicit `.gzi` index file path                               | ✅ Done     |
| `-r`  | `--reindex`     | Rebuild `.gzi` index for an existing BGZF file                | ✅ Done     |
| `-b`  | `--offset`      | Start decompression at uncompressed byte offset               | ✅ Done     |
| `-s`  | `--size`        | Write at most N uncompressed bytes (used with `-b`)           | ✅ Done     |
|       | `--binary`      | Treat input as binary (accepted; TaraBG always uses binary)   | ✅ No-op    |
| `-g`  | `--rebgzip`     | Re-bgzip: not yet implemented                                 | ⚠️ Stub    |
| `-h`  | `--help`        | Print help                                                    | ✅ Done (clap) |
|       | `--version`     | Print version                                                 | ✅ Done (clap) |

## File-mode behavior

When no `-c` / `-o -` / stdin is used, TaraBG operates in **file mode**:

- **Compress:** `tarabg input.vcf` → writes `input.vcf.gz`, then **removes** `input.vcf`
- **Decompress:** `tarabg -d input.vcf.gz` → writes `input.vcf`, then **removes** `input.vcf.gz`
- **`-k`:** Retain the input file after the operation.
- **`-f`:** Overwrite an existing output file. Without `-f`, TaraBG refuses to overwrite.
- **`-o FILE`:** Write to `FILE` explicitly; input file is **always retained** when `-o` is used.
- Multiple positional files are supported; each is processed independently.

Writes are atomic (`.tmp` → rename) to avoid partial output on failure.

## Common workflows

```bash
# Compress to stdout with 4 threads
tarabg -@ 4 -l 6 -c input.vcf > input.vcf.gz

# Compress in file mode (input removed on success)
tarabg -@ 4 -l 6 input.vcf

# Keep input
tarabg -k input.vcf

# Overwrite existing .gz
tarabg -f input.vcf

# Validate integrity
tarabg -t input.vcf.gz

# Decompress to stdout
tarabg -d -c input.vcf.gz > restored.vcf

# Compress with .gzi index
tarabg -i -I input.vcf.gz.gzi -c input.vcf > input.vcf.gz

# Rebuild index for existing file
tarabg -r -I input.vcf.gz.gzi input.vcf.gz

# Read 100 uncompressed bytes starting at offset 65,270
tarabg -b 65270 -s 100 -I input.vcf.gz.gzi input.vcf.gz > slice.bin

# Interoperability check
bgzip -t input.vcf.gz
bgzip -d -c input.vcf.gz | cmp input.vcf -
```

TaraBG output does not need to byte-match `bgzip` output. Compatibility means
valid BGZF, exact decompressed content, and acceptance by HTSlib tools.

## `.gzi` index and random access

Creating an index with `-i` enables random access without decompressing the whole
file. The `.gzi` format stores pairs of `(compressed_offset, uncompressed_offset)`
for each block boundary after the first. TaraBG-produced indexes are accepted by
`bgzip` and vice versa.

## Streaming

TaraBG uses bounded-memory streaming pipelines:

- **Compression:** reads in batches of `threads × 4` chunks at a time; compresses
  in parallel; writes immediately. Peak RAM ≈ `threads × 4 × 65 KiB`.
- **Decompression:** reads and decompresses one BGZF block (~65 KiB) at a time.

A 10 GB VCF file does not require 10 GB of RAM.

## Tests

```bash
cargo test
```

The suite covers all compression levels, CRC32 rejection, index serialization,
reindexing, streaming round-trips, file-mode semantics, and—when `bgzip` is
installed—TaraBG → bgzip and bgzip → TaraBG interoperability.

## Benchmarks

Benchmark only native binaries in the same operating-system environment. Do
**not** time a Windows binary through WSL, a container boundary, or a network
filesystem. Build TaraBG and install `bgzip` on the same Linux host:

```bash
sudo apt install tabix linux-tools-common
cargo build --release
bash benches/benchmark.sh sample.vcf target/release/tarabg
```

The harness refuses Windows binaries and emits `benches/results/results.csv`.
See [`benches/README.md`](benches/README.md) for details.

## Development rule

Correct → compatible → profile → change one thing → benchmark → keep or revert.
No performance claim is accepted until compatibility tests and an equal-environment
benchmark both pass.

## License

MIT licensed; see [LICENSE-MIT](LICENSE-MIT).
