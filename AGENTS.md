# TaraBG agent guide

## Product direction

TaraBG is becoming a clean-room, command-line-compatible replacement for
HTSlib `bgzip`. The immediate v0.1 implementation is **not** feature-complete;
never claim full drop-in status until the relevant bgzip options, indexing, and
random-access behavior have been implemented and tested.

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

- Current supported flags are documented in `README.md`; do not silently claim
  unsupported bgzip flags work.
- `-c` pipelines and stdin/stdout are core workflows and must remain reliable.
- Generated benchmark data and CSV/GZ results are ignored; do not commit them.
- Read `skills/tarabg/SKILL.md` before implementing format, compatibility,
  benchmark, or release work.
- Run `cargo test` and `cargo build --release` before handing off a change.

## Publishing

Do not publish releases or push branches merely because code changed. When
explicitly asked, ensure documentation and release notes accurately describe
the implemented compatibility scope and attach only artifacts built by CI.
