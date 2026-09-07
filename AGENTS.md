# TaraBG agent guide

## Current implementation status (updated 2026-09-07)

**19/19 tests passing. `cargo build --release` and `cargo clippy` clean.**

### What is fully implemented

- BGZF read/write with CRC32 + ISIZE validation and EOF marker
- Compression levels 0–9 (`-l`/`--level`), parallel compression (`-@`/`--threads`)
- Streaming pipeline: compression in `threads×4` block batches; decompression
  block-by-block — **no whole-file buffering**; a 10 GB input does not need 10 GB RAM
- `-c`/`--stdout`, `-d`/`--decompress`, `-t`/`--test` (stdin/stdout pipelines)
- **File-mode:** default `.gz` output, input removal on success, atomic writes
- `-k`/`--keep` — retain input; `-f`/`--force` — overwrite; `-o FILE` — explicit output
- Multiple positional input files (processed independently, errors reported per-file)
- `.gzi` index create (`-i`), rebuild (`-r`), default naming, and random reads (`-b`/`-s`)
- Long-option aliases for every flag (`--stdout`, `--decompress`, etc.)
- `--binary` accepted as no-op; `-g`/`--rebgzip` stubs with a clear error
- Format hardening: ISIZE > 65536 rejected, malformed headers rejected, EOF markers
  in the middle of a stream skipped as no-ops
- CI: `.github/workflows/ci.yml` tests on Ubuntu, macOS, and Windows; includes
  native bgzip differential tests on Ubuntu

### What is NOT yet done (remaining planned work)

- `-g`/`--rebgzip` full implementation (currently exits with "not yet implemented")
- Fuzzing targets (cargo-fuzz / libFuzzer for BGZF and .gzi parsers)
- Native Linux performance baseline (infrastructure in `benches/` is ready)
- Signed/checksummed multi-platform release artifacts from CI

### One-line status for other agents

> **Phases 1–8 of ROADMAP.md are structurally complete. Only `-g`/`--rebgzip`,
> fuzzing, and a native perf baseline remain before "full drop-in" can be claimed.**

---

## Product direction

TaraBG is a clean-room, command-line-compatible replacement for HTSlib `bgzip`.
Do not claim full drop-in status until `-g`/`--rebgzip` is implemented, fuzzing
is in CI, and a native same-environment benchmark exists.

## Non-negotiable correctness contract

- Implement BGZF from published format documentation; never copy HTSlib code.
- Compressed bytes need not match `bgzip` byte-for-byte.
- Every format change must preserve: valid BGZF, `bgzip -t` acceptance, exact
  decompressed bytes, and TaraBG's ability to decode bgzip output.
- Extend `tests/compatibility.rs` whenever a new user-visible bgzip behavior is
  added. Prefer tests that use an installed native `bgzip`; skip only when it
  is absent.
- Preserve block ordering under parallel compression and the BGZF EOF marker.

## Performance discipline

Compatibility comes before optimization. Benchmark only TaraBG and bgzip native
binaries in the same OS, architecture, filesystem, and input environment.
Never compare a Windows executable through WSL with a Linux executable.

Use `benches/benchmark.sh` after building `target/release/tarabg`. For a real
optimization, record a before/after CSV and run compatibility tests. Use
`perf stat -d` or `perf record` on native Linux to identify a hotspot before
changing it. Keep or revert based on measured results, not intuition.

## Scope and repository conventions

- Current supported flags are documented in `README.md` (supported-options table).
- `-c` pipelines and stdin/stdout are core workflows and must remain reliable.
- Generated benchmark data and CSV/GZ results are ignored; do not commit them.
- Read `skills/tarabg/SKILL.md` before implementing format, compatibility,
  benchmark, or release work.
- Run `cargo test` and `cargo build --release` before handing off a change.

## Publishing

Do not publish releases or push branches merely because code changed. When
explicitly asked, ensure documentation and release notes accurately describe
the implemented compatibility scope and attach only artifacts built by CI.
