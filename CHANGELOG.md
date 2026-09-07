# Changelog

## [Unreleased] — v0.2 (planned)

- Deflate backend switched from zlib-ng to libdeflate (levels map 1:1;
  `-l 0` remains a stored/raw block). Output stays valid BGZF accepted by
  `bgzip`, with exact decompressed bytes; compressed bytes differ from v0.1
  and are typically smaller. Same-environment Linux benchmark vs bgzip 1.19:
  faster compression at every level (notably `-l 9` at identical size),
  decompression at parity.
- Streaming decompression reworked to single-pass block parsing (no
  reconstructed-block copy); `test`/`reindex`/range reads no longer buffer
  the whole file and stop early once fulfilled
- Phase 7: Native Linux performance baseline
- Phase 8: CI release artifacts for Linux, macOS, Windows
- Fuzzing targets for BGZF and .gzi parsers
- `-g`/`--rebgzip` full implementation

## [0.1.0]

### Added
- Clean-room BGZF compressor/decompressor in Rust
- Compression levels 0–9 (`-l`)
- Parallel block compression (`-@`)
- `-c` stdout pipeline, `-d` decompress, `-t` integrity test
- File-mode: default `.gz` output, input removal on success
- `-k`/`--keep`: retain input file
- `-f`/`--force`: overwrite existing output
- `-o FILE`: explicit output path
- Multiple positional input files
- `.gzi` index creation (`-i -I`), rebuild (`-r`), random reads (`-b`, `-s`)
- Long-option aliases: `--stdout`, `--decompress`, `--test`, `--index`, `--reindex`, `--keep`, `--force`, `--offset`, `--size`, `--level`, `--threads`
- `--binary` flag (accepted, no-op; TaraBG always treats input as binary)
- `-g`/`--rebgzip`: exits with "not yet implemented"
- Streaming compression and decompression (bounded memory, no whole-file buffering)
- Format hardening: rejects ISIZE > 65536, malformed gzip/BGZF headers, truncated blocks
- Atomic file writes (`.tmp` → rename)
- Comprehensive test suite: roundtrip, format hardening, streaming, file-mode, bgzip interop
