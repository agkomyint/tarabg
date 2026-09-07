# TaraBG roadmap: from v0.1 to a complete bgzip replacement

## Status (as of v0.1)

Phases 1–6 are complete. Phase 7 (performance baseline) requires native Linux
hardware and is infrastructure-ready. Phase 8 (release engineering) is in progress.

Implemented:
- BGZF read/write with CRC32/ISIZE validation and EOF marker
- All compression levels 0–9, parallel compression with `-@`
- `-c`, `-d`, `-t` with stdin/stdout pipelines
- File-mode: default `.gz` output, input removal, `-k`, `-f`, `-o FILE`
- Multiple positional input files
- `.gzi` index creation (`-i`), rebuild (`-r`), and random reads (`-b`/`-s`)
- Long-option aliases for all flags
- `--binary` (accepted, no-op), `-g` (stub with error)
- Streaming compression and decompression (bounded memory)
- Format hardening: ISIZE > 65536 rejected, malformed headers rejected

## Goal

Ship a clean-room Rust executable that users can substitute for HTSlib `bgzip`
in normal compression, decompression, indexing, integrity-checking, and
random-access workflows.

The compatibility target is the public behavior documented for HTSlib `bgzip`
1.24. TaraBG must implement BGZF from published specifications and black-box
behavioral tests only; it must not copy HTSlib implementation code.

“Complete” does **not** mean TaraBG produces byte-identical compressed streams.
It means the output is valid BGZF, decompressed content is identical, indexes
and offsets are compatible, CLI behavior is compatible where promised, and
performance comparisons are fair.

## Ground rules

1. Correctness before speed.
2. Compatibility tests are required for every user-visible behavior.
3. Test TaraBG → bgzip and bgzip → TaraBG whenever a native bgzip binary is
   available.
4. Profile before optimizing; change one hotspot at a time.
5. Benchmark only native binaries in the same OS, architecture, filesystem,
   input, compression level, and thread-count environment.
6. Do not claim a full drop-in replacement before the release gate is met.

## Current baseline

Implemented today:

- BGZF read/write with BGZF EOF marker
- CRC32 and ISIZE integrity validation
- stdin/stdout, `-c`, `-d`, `-t`, `-l 0..9`, `-@`
- Parallel block compression with ordered output
- `.gzi` creation (`-i -I`), rebuild (`-r -I`), loading, and random reads
  (`-b`, `-s`)
- Unit tests for levels, corruption, index serialization, reindexing, and
  random ranges
- Optional native bgzip integration tests

Known gaps:

- Conventional file output/removal behavior
- Multiple positional files
- `-f`, `-k`, `-o`, `--binary`, `-g`, help/version parity
- Complete error and exit-code parity
- Full test corpus and continuous native Linux compatibility CI
- Native same-environment performance baseline

## Phase 1 — Establish the compatibility lab

### Deliverables

- Pin a tested HTSlib/bgzip version in CI while retaining a matrix for other
  supported versions.
- Add a Linux CI job that installs native `bgzip` and runs all differential
  tests.
- Add reusable test helpers for temporary files, SHA-256 comparisons, process
  exit assertions, and bgzip invocation.
- Add deterministic fixtures:
  - empty, tiny, and exact block-boundary inputs
  - text/VCF-like records
  - random incompressible bytes
  - repetitive highly compressible bytes
  - long lines larger than a BGZF block
  - files spanning many blocks
  - intentionally corrupted and truncated BGZF files

### Exit gate

Every fixture can be compressed by TaraBG, validated by `bgzip -t`, decompressed
by bgzip, and compared byte-for-byte. The reciprocal bgzip → TaraBG path passes.

## Phase 2 — Complete basic command-line and file semantics

### Implement

- Default compression output: `input` becomes `input.gz` when `-c` and `-o`
  are absent.
- Default decompression output: remove a known compression suffix.
- Input removal after successful default-file operations.
- `-k` to retain input.
- `-f` behavior for existing output and nonstandard decompression suffixes.
- `-o FILE` output behavior, including overwrite behavior and input retention.
- Multiple positional input files, with independent success/failure reporting.
- `-h`/`--help` and `--version` behavior.
- File metadata handling where practical and platform-supported.

### Tests

For every command form, compare TaraBG and bgzip for:

- output path and file existence
- input retention/removal
- overwrite refusal or success
- standard output/stderr behavior
- exit status
- compressed validity and decompressed SHA-256

### Exit gate

The ordinary one-file and multi-file compression/decompression workflows behave
the same as bgzip for supported platforms and all fixtures.

## Phase 3 — Finish index and random-access compatibility

### Implement and verify

- Default `.gzi` naming for file-based compression and reindexing.
- `.gzi` compatibility for empty, one-block, multiblock, and text inputs.
- `-b OFFSET` reads beginning inside any block, at a boundary, at EOF, and past
  EOF.
- `-s SIZE` reads zero bytes, short final ranges, and ranges spanning blocks.
- Default index discovery and explicit `-I FILE` behavior.
- Validation and clear rejection of malformed, unsorted, truncated, or
  out-of-range `.gzi` files.
- Virtual/block offset boundary checks against bgzip.

### Differential matrix

1. TaraBG compress/index → bgzip random read.
2. bgzip compress/index → TaraBG random read.
3. TaraBG reindex → bgzip random read.
4. bgzip reindex → TaraBG random read.
5. Compare all requested bytes and statuses for offset/size boundary values.

### Exit gate

All generated indexes are consumable across implementations and all valid ranges
return precisely the requested uncompressed bytes.

## Phase 4 — Match remaining documented bgzip options

### `--binary`

Match bgzip’s text-aware block-boundary behavior and its binary mode. Test
newline alignment, block-sized lines, and very long lines.

### `-g` / `--rebgzip`

Implement only after index behavior is mature. Define the exact constraints,
failure modes, and matching-offset tests. Verify compressed block offsets against
the provided index rather than compressed-byte identity.

### Other option and parser parity

- Long-option aliases
- Combined short options where bgzip accepts them
- Numeric option validation
- Invalid combination diagnostics
- Multiple inputs and stdin restrictions

### Exit gate

Every documented, supported bgzip option has direct integration coverage.
Unsupported options are absent only if the project explicitly chooses a smaller
compatibility target; otherwise they block “drop-in” status.

## Phase 5 — Format hardening and adversarial testing

### Parser hardening

- Validate gzip/BGZF headers, XLEN, subfields, BC length, BSIZE, CRC32, and
  ISIZE.
- Reject malformed concatenations, invalid block sizes, truncated payloads,
  duplicate/malformed BC fields, and impossible index offsets.
- Avoid integer overflows and unbounded allocations from untrusted metadata.

### Fuzzing

- Add cargo-fuzz or libFuzzer targets for BGZF and `.gzi` parsing.
- Seed with valid files from TaraBG and bgzip.
- Add every minimized crash or mismatch to regression fixtures.

### Exit gate

Fuzzing runs in CI or on a scheduled runner, parser regressions are fixed, and
malformed input returns controlled errors rather than panics.

## Phase 6 — Streaming and resource correctness

### Implement

- Replace whole-file buffering with bounded streaming pipelines.
- Maintain block order while compressing in parallel.
- Bound queued input, compressed blocks, and worker memory.
- Stream decompression and range output without decoding the complete tail.
- Preserve correct handling of stdin/stdout and broken-pipe behavior.

### Tests

- Large files under a constrained memory limit.
- Pipelines with stdin/stdout.
- Slow readers/writers where practical.
- Multiple thread counts with identical decompressed output.

### Exit gate

Peak memory scales with configured pipeline capacity, not total input size, and
all compatibility tests remain green.

## Phase 7 — Native performance baseline

### Benchmark setup

Use one native Linux host, local storage, release builds, the same input, and
the same `-l`/`-@` values:

```bash
/usr/bin/time -v bgzip -@ 4 -l 6 -c sample.vcf > bgzip.gz
/usr/bin/time -v target/release/tarabg -@ 4 -l 6 -c sample.vcf > tarabg.gz
```

Run levels `1,6,9` and threads `1,2,4,8,16`. Capture:

- compression and decompression throughput
- wall time, CPU time, and peak RSS
- output size
- `perf stat -d` counters

Store machine details, input checksums, tool versions, and raw CSV beside each
published result.

### Optimization loop

1. Record a clean baseline.
2. Use `perf record`/`perf report` to identify a hotspot.
3. Optimize one measured cause only.
4. Run compatibility tests.
5. Run the same benchmark matrix.
6. Keep the change only when it improves the chosen metric without regressions.

### Exit gate

Published performance claims include reproducible commands and native,
same-environment evidence. No cross-WSL comparisons are published.

## Phase 8 — Release engineering

### CI and artifacts

- Test on Linux, macOS, and Windows where supported.
- Run native Linux bgzip differential tests.
- Build signed or checksummed release artifacts for supported targets.
- Include `tarabg --version`, license, README, and SHA-256 checksums.
- Publish source, Linux, macOS, and Windows installation instructions.

### Documentation

- Keep a precise supported-options table.
- Document migration from bgzip and known intentional differences.
- Include `.gzi`, random-access, stdout, and file-mode examples.
- Include reproducible benchmark instructions and results.
- Maintain a changelog with compatibility-impacting changes.

### Exit gate

A fresh user can install an artifact or build from source, run the documented
commands, and reproduce the compatibility checks.

## Final drop-in replacement release gate

TaraBG can be called a complete bgzip replacement only when all are true:

- All target bgzip CLI options and expected file semantics are implemented.
- Native differential tests pass across the complete fixture and option matrix.
- TaraBG BGZF and `.gzi` artifacts work with HTSlib tools such as bgzip/tabix.
- TaraBG safely reads bgzip artifacts, including indexed random ranges.
- Parser hardening and fuzz regression coverage are in place.
- Streaming resource behavior is bounded and tested.
- CI produces tested release artifacts for supported platforms.
- README, changelog, installation instructions, and release notes accurately
  describe compatibility scope and any intentional differences.
- Native same-environment benchmark evidence exists before making speed claims.

## Work sequence

1. Phase 1 compatibility lab
2. Phase 2 file and CLI semantics
3. Phase 3 index/random-access completion
4. Phase 4 remaining options
5. Phase 5 hardening
6. Phase 6 streaming
7. Phase 7 performance work
8. Phase 8 release engineering and final gate

At every phase: implement a small behavior, add a differential test, verify
TaraBG ↔ bgzip interoperability, then proceed.
