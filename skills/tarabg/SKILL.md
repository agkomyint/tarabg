---
name: tarabg
description: Implement, test, benchmark, or release TaraBG's clean-room BGZF and bgzip-compatibility work.
---

# TaraBG

Use this skill for work that changes TaraBG's BGZF behavior, bgzip CLI
compatibility, interoperability tests, benchmarks, or releases.

## Objective

Move toward a real `bgzip` drop-in replacement without sacrificing correct,
interoperable BGZF. v0.1 supports compression, decompression, integrity tests,
levels, worker counts, and streaming workflows; unsupported bgzip features must
remain documented as unsupported.

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
implemented. Do not label TaraBG a complete drop-in replacement until coverage
for required bgzip flags, indexing, and random access is demonstrably present.
