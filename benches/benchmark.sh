#!/usr/bin/env bash
set -euo pipefail
# Linux benchmark harness. Usage: benches/benchmark.sh sample.vcf [tarabg binary]
input=${1:?usage: $0 sample.vcf [tarabg]}
tarabg=${2:-target/release/tarabg}
if [[ $(uname -s) != Linux ]]; then
  echo "This harness must run on native Linux; benchmark both tools in the same OS environment." >&2
  exit 2
fi
if ! command -v bgzip >/dev/null; then
  echo "bgzip is required on PATH (for example: sudo apt install tabix)." >&2
  exit 2
fi
if [[ ! -x $tarabg ]]; then
  echo "TaraBG binary is missing or not executable: $tarabg" >&2
  exit 2
fi
if file -b "$tarabg" | grep -q 'PE32'; then
  echo "Refusing a Windows TaraBG executable: build and benchmark native Linux binaries." >&2
  exit 2
fi
mkdir -p benches/results
input_bytes=$(wc -c < "$input")
printf 'tool,threads,level,mode,wall_seconds,cpu_seconds,peak_rss_kb,input_bytes,output_bytes,throughput_mb_s\n' > benches/results/results.csv

record() {
  local tool=$1 threads=$2 level=$3 mode=$4 outfile=$5; shift 5
  local stats; stats=$(mktemp)
  /usr/bin/time -f '%e,%U,%M' -o "$stats" "$@"
  IFS=, read -r wall cpu rss < "$stats"; rm "$stats"
  local output_bytes=0 bytes=$input_bytes
  [[ $mode == compress ]] && output_bytes=$(wc -c < "$outfile")
  local rate; rate=$(awk -v b="$bytes" -v s="$wall" 'BEGIN { if (s > 0) printf "%.3f", b / 1000000 / s; else print "0.000" }')
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' "$tool" "$threads" "$level" "$mode" "$wall" "$cpu" "$rss" "$input_bytes" "$output_bytes" "$rate" >> benches/results/results.csv
}

for threads in 1 2 4 8 16; do for level in 1 6 9; do
  for tool in bgzip "$tarabg"; do
    label=$(basename "$tool") gz="benches/results/${label}-${threads}-${level}.gz"
    record "$label" "$threads" "$level" compress "$gz" "$tool" -@ "$threads" -l "$level" -c "$input" > "$gz"
    record "$label" "$threads" "$level" decompress /dev/null "$tool" -@ "$threads" -d -c "$gz" > /dev/null
  done
done; done
echo "Wrote benches/results/results.csv"
