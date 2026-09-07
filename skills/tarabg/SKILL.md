---
name: tarabg
description: Implement, test, benchmark, or release TaraBG's clean-room BGZF and bgzip-compatibility work.
---

# TaraBG

Use this skill for work that changes TaraBG's BGZF behavior, bgzip CLI
compatibility, interoperability tests, benchmarks, or releases.

## Current status (2026-09-07)

**41/41 tests passing. All ROADMAP phases 1–8 structurally complete.**

### Done
- BGZF engine: read/write, CRC32, ISIZE, EOF marker, all levels 0–9
- Streaming: compression in batches (`threads×4` chunks), decompression block-by-block (bounded RAM)
- Full CLI: `-c -d -t -l -@ -k -f -o -i -I -r -b -s -g --binary`, long aliases, multiple files
- File-mode: default `.gz` output, atomic writes, input removal, `-k`/`-f`/`-o`
- `.gzi` index: create, rebuild, default naming, random reads
- `-g`/`--rebgzip`: re-block per `.gzi` index (requires `-I`, rejects `-i`/`-r`),
  differential-tested against native bgzip block splits
- Format hardening: ISIZE > 65536 rejected, mid-stream EOF skipped, malformed headers rejected
- Tests: 17 roundtrip/parser + 24 integration (file-mode, bgzip interop, rebgzip)
- CI: Ubuntu + macOS + Windows matrix, native bgzip differential tests on Ubuntu
- Fuzzing: BGZF and `.gzi` libFuzzer targets with scheduled CI runs
- Docs: README options table, CHANGELOG, ROADMAP status, benches/README.md

### NOT done (remaining before "full drop-in")
- Native Linux perf baseline (benches/ infrastructure ready, needs actual Linux run)
- Confirm the signed/checksummed multi-platform workflow in a published release

## Objective

Move toward a real `bgzip` drop-in replacement without sacrificing correct,
interoperable BGZF. The bare-metal native Linux performance baseline is the
primary remaining gap. Unsupported features must remain documented as unsupported.

## Before changing behavior

Identify the corresponding observable `bgzip` behavior and add or extend a
compatibility test. Keep implementation clean-room: derive format behavior from
published specifications and black-box CLI behavior, never copied source.

For BGZF output, require all of the following:

1. `bgzip -t` accepts TaraBG output.
2. `bgzip -d -c` yields the exact source bytes.
3. TaraBG decodes bgzip output to the exact source bytes.
4. Corruption is rejected by integrity validation.

Byte-identical compressed streams are neither expected nor required.

## Benchmarking

Do not draw performance conclusions across WSL/Windows, containers, remote
filesystems, different hardware, or mismatched tool versions. Use native builds
on one host and run `benches/benchmark.sh` with the same input, levels, and
thread counts for both tools. Capture before and after data for any optimization.

Profile first on Linux with `perf stat -d` or `perf record`; change one measured
hotspot at a time. Compatibility tests and release-mode benchmarks decide whether
the change stays.

## Release handling

Release notes and README installation instructions must state what is genuinely
implemented. Do not label TaraBG a complete drop-in replacement until fuzzing
and a native same-environment benchmark are both in place.
