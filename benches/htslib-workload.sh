#!/usr/bin/env bash
set -euo pipefail

# Exercise an HTSlib VCF workload against files produced by both compressors.
# Usage: htslib-workload.sh INPUT.vcf TARABG HTSLIB_BIN_DIR [OUTPUT_DIR]
input=${1:?usage: $0 INPUT.vcf TARABG HTSLIB_BIN_DIR [OUTPUT_DIR]}
tarabg=${2:?usage: $0 INPUT.vcf TARABG HTSLIB_BIN_DIR [OUTPUT_DIR]}
htsbin=${3:?usage: $0 INPUT.vcf TARABG HTSLIB_BIN_DIR [OUTPUT_DIR]}
out=${4:-benches/results/htslib-workload}

for executable in "$tarabg" "$htsbin/bgzip" "$htsbin/htsfile" "$htsbin/tabix"; do
  [[ -x $executable ]] || { echo "Missing executable: $executable" >&2; exit 2; }
done
[[ -f $input ]] || { echo "Missing input VCF: $input" >&2; exit 2; }
[[ ! -e $out ]] || { echo "Output directory already exists: $out" >&2; exit 2; }
mkdir -p "$out"

time_one() {
  local destination=$1
  shift
  /usr/bin/time -f 'wall_seconds,user_seconds,system_seconds,peak_rss_kb\n%e,%U,%S,%M' \
    -o "$destination" "$@"
}

time_one "$out/bgzip-compress.csv" "$htsbin/bgzip" -@ 4 -l 6 -c "$input" > "$out/bgzip.vcf.gz"
time_one "$out/tarabg-compress.csv" "$tarabg" -@ 4 -l 6 -c "$input" > "$out/tarabg.vcf.gz"

for producer in bgzip tarabg; do
  file="$out/$producer.vcf.gz"
  "$htsbin/bgzip" -t "$file"
  "$htsbin/htsfile" "$file" > "$out/$producer.htsfile.txt"
  time_one "$out/$producer-index.csv" "$htsbin/tabix" -f -p vcf "$file"
  "$htsbin/tabix" -H "$file" > "$out/$producer.header"
  "$htsbin/tabix" "$file" '22:16000000-17000000' > "$out/$producer.region"
  "$htsbin/bgzip" -dc "$file" | sha256sum > "$out/$producer.sha256"
done

sha256sum "$input" > "$out/input.sha256"
cmp "$out/bgzip.header" "$out/tarabg.header"
cmp "$out/bgzip.region" "$out/tarabg.region"
input_hash=$(cut -d' ' -f1 < "$out/input.sha256")
for producer in bgzip tarabg; do
  output_hash=$(cut -d' ' -f1 < "$out/$producer.sha256")
  [[ $input_hash == "$output_hash" ]] || { echo "$producer decompression differs from input" >&2; exit 1; }
done

printf 'producer,compressed_bytes\n' > "$out/sizes.csv"
for producer in bgzip tarabg; do
  printf '%s,%s\n' "$producer" "$(wc -c < "$out/$producer.vcf.gz")" >> "$out/sizes.csv"
done

echo "HTSlib validation, identification, indexing, region-query, and byte-identity checks passed."
echo "Results: $out"
