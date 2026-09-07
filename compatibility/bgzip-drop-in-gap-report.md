# TaraBG versus HTSlib `bgzip`: drop-in compatibility gap report

**Snapshot:** 2026-09-07 (post-`--rebgzip` audit)  
**Reference implementation:** HTSlib `bgzip` 1.19 installed from Ubuntu's `tabix` package  
**Compatibility target:** documented `bgzip` CLI/file behavior and interoperable BGZF / `.gzi`, not byte-identical compressed output.

## Executive answer

TaraBG already covers the core BGZF workflow: streaming compression and
decompression, CRC/ISIZE validation, indexes, range reads, file mode, and
parallel compression.  It is **not yet safe to call a complete `bgzip`
drop-in replacement**.

`-g/--rebgzip` is now implemented: it requires `-I`, rejects incompatible
index-creation/reindex combinations, works in stdout and file modes, and has
dedicated tests for round trips, file side effects, empty/EOF index boundaries,
option precedence, and native-bgzip block splits. The workspace test suite is
**27/27 passing** and `cargo build --release` passes.

Even after `-g` lands, two documented behaviors still need implementation or
an explicit compatibility decision:

1. **Text-aware blocking versus `--binary`.** HTSlib normally tries to end
   text blocks at newlines; `--binary` disables that. TaraBG accepts
   `--binary` as a no-op and always blocks by byte count. That is a real
   observable difference for index/block layout.
2. **Parser and file-semantics edge parity.** In particular, HTSlib accepts
   `--compress-level`, `--index-name`, compression level `-1`, and has the
   documented double-`--force` behavior. TaraBG does not currently expose all
   of these semantics.

The remaining work is therefore compatibility completeness and release
confidence—not the previously missing `-g` feature.

## Post-`-g` audit result

### Verified complete in this checkout

- `-g` engine and CLI wiring in both stdout and file modes.
- Required `-I` index handling and rejection of `-g` with `-i`/`-r`.
- Re-bgzip block-boundary comparison test against native bgzip when that binary
  is available.
- Core test suite: 13 round-trip tests + 14 integration tests, all passing.
- Release build passes.

### Still required before calling TaraBG a full drop-in replacement

1. Implement newline-aware default text blocking and real `--binary` behavior.
2. Add missing documented aliases/value/file-semantics parity listed below.
3. Run the native-bgzip portion of the suite in this exact checkout and expand
   the differential matrix; the present Windows session has no `bgzip.exe` on
   PATH, so those tests skip locally. CI is the current native reference gate.
4. Add parser fuzzing and regression seeds.
5. Produce a bare-metal native-Linux benchmark/profiling baseline.
6. Publish verified multi-platform release artifacts with checksums/signatures.
7. Reconcile documentation: `README.md` now marks `-g` done, but the current
   `ROADMAP.md` still calls it a stub and must be updated before release.

## Evidence and scope

- HTSlib's [bgzip 1.17 manual](https://www.htslib.org/doc/1.17/bgzip.html)
  documents the option set, default file behavior, BGZF layout, and `.gzi`
  binary format. The locally exercised reference was HTSlib 1.19, whose
  `--help` advertises the same relevant options.
- HTSlib's [documentation index](https://www.htslib.org/doc/) says to consult
  the manual page matching the installed release. This report therefore treats
  1.19 command-line output as the runtime reference and the official manual as
  the semantic reference.
- TaraBG assessment is from `README.md`, `ROADMAP.md`, `src/main.rs`,
  `src/bgzf.rs`, `tests/compatibility.rs`, and the CI/release workflows in this
  workspace. “Verified” below means source + existing test coverage; it does
  **not** mean every edge was differentially tested.

## Option and behavior matrix

| HTSlib behavior | TaraBG status | Evidence / gap |
|---|---|---|
| BGZF compression, concatenated blocks, EOF marker | **Implemented** | Streaming encoder emits BGZF blocks and EOF; unit/interoperability tests cover round trips. |
| Decompression and `-t/--test` integrity checking | **Implemented** | CRC32, ISIZE, malformed/truncated input tests; basic native bgzip interop. Add corrupted native fixtures. |
| `-c/--stdout`, stdin/stdout operation | **Implemented** | Source and tests cover stdout; add broken-pipe and TTY/error-message differential cases. |
| `-d/--decompress`; default `.gz` naming and input removal | **Implemented** | File-mode integration test exists. HTSlib's force/extension edge cases still need parity tests. |
| `-k/--keep` and `-f/--force` | **Mostly implemented** | Basic keep/overwrite tests pass. HTSlib permits force on unknown suffixes and documents two `--force` uses for both overwrite and suffix override; TaraBG has one Boolean force and rejects unsupported suffix naming. |
| `-l/--compress-level` levels `0..9`, `-1` default | **Partial** | TaraBG supports `0..9`, exposes `--level`, defaults to `6`; it rejects `-1` and lacks the canonical long alias `--compress-level`. Add aliases and native differential tests. |
| `-@/--threads` | **Implemented** | Parallel compression preserves ordering. Decompression is streaming/single-threaded; confirm HTSlib's observable handling of `-@` on decompression and invalid values. |
| `-i/--index`, `-I/--index-name`, `.gzi` | **Mostly implemented** | Create/read/rebuild/range interop is tested with native bgzip. TaraBG has `-I` but does **not** currently define the long alias `--index-name`; add it. Add malformed-index and overwrite/default-name cases. |
| `-r/--reindex` | **Implemented, edge coverage needed** | Reindex machinery exists. Add differential tests for stdin restriction, overwrite behavior, EOF/midstream EOF, and malformed BGZF. |
| `-b/--offset`, `-s/--size` | **Mostly implemented** | Range decoding + basic cross-tool test are present. Expand tests for 0, block boundaries, offsets after EOF, size 0, no index, stdin, and all documented option combinations. Preserve HTSlib's exact offset interpretation and errors. |
| `--binary` / default text newline alignment | **Missing semantic behavior** | TaraBG explicitly accepts it as a no-op. HTSlib's manual says default text input is newline-aligned where possible and `--binary` disables alignment. Implement and test line-boundary, >64 KiB-line, and binary cases, or clearly retain a reduced-compatibility claim. |
| `-g/--rebgzip` with mandatory `-I` | **Implemented** | Wired in stdout and file modes; requires `-I`, rejects `-i`/`-r`, has round-trip/file-mode/edge tests, and compares block splits against native bgzip when available. |
| `-h/--help`, `--version` | **Implemented** | Clap provides these; text need not be byte-identical, but option spelling/exit status should be tested. |
| Multiple input files | **Implemented** | TaraBG processes independently and reports per-file errors. Add native differential tests for partial failure and exit status. |
| `-o FILE` | **TaraBG extension** | Useful and documented, but not a documented HTSlib `bgzip` option. Keep it; ensure it never changes the behavior of standard invocations. |
| Combined short options, diagnostics, precedence | **Unverified** | Clap often accepts combined flags, but this has not been systematically differential-tested. Build a command/error matrix rather than assuming parser parity. |

## Format and interoperability status

TaraBG has the important format protections: BGZF BC-subfield parsing,
compressed-size bounds, CRC32 and ISIZE checks, `.gzi` little-endian offsets,
EOF marker handling, bounded streaming, and native bgzip interoperability
tests. The existing real-data runs also verified that `bgzip -t` accepts
TaraBG output and TaraBG decodes bgzip output byte-exactly.

What is still missing is *adversarial confidence*, not the basic happy path:

- Fuzz BGZF block/header/trailer parsing and `.gzi` parsing with cargo-fuzz or
  libFuzzer; seed from both tools and promote minimized crashes to fixtures.
- Differential-test invalid inputs and error exits, rather than only valid
  round trips.
- Test indexes and ranges across a richer fixture set: empty input, exact
  block boundaries, incompressible blocks, long text lines, multiple EOF
  markers, sparse/large index gaps, and corrupted indices.

## Required work, in execution order

### P0 — required before saying “CLI-complete”

1. Implement HTSlib-compatible newline-aware text blocking and make
   `--binary` choose raw byte blocking. Add native tests that inspect block
   start offsets, not compressed-byte identity.
2. Add parser aliases and value parity: `--compress-level`, `--index-name`,
   `-l -1`, and the documented force/suffix behavior—or document each as an
   intentional non-drop-in difference.
3. Add a native `bgzip` differential test matrix for every option, valid and
   invalid combination, and file-mode side effect.

### P1 — required before the “full drop-in” claim

4. Add fuzz targets and CI/scheduled fuzz regression coverage.
5. Publish a bare-metal native-Linux benchmark with input hash, tool versions,
   raw measurements, output sizes, RSS, and `perf stat -d`; do not label WSL
   results as a native baseline.
6. Finish release engineering: tested Linux/macOS/Windows artifacts, SHA-256
   checksums (and signatures if offered), release notes, and installation
   verification. The current release workflow only packages Windows.

### P2 — quality and maintenance

7. Expand documentation with an explicit compatibility-version target (for
   example HTSlib 1.19) and a list of intentional differences.
8. Keep the real-world benchmark corpus and all generated results out of git;
   retain only acquisition commands, hashes, and reproducible harnesses.

## Completion definition

After P0, TaraBG can reasonably be described as **feature-complete against the
targeted bgzip CLI** (subject to the documented version and any explicit
differences). After P0 + P1, it can credibly claim to be a **full drop-in
replacement**. Until then, the README should continue to say that it is a
clean-room BGZF implementation designed for interoperability, not a complete
replacement.
