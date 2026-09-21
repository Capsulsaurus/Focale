#!/usr/bin/env bash
# Render the committed determinism fixture in every export format and record
# "<label> <hash>" lines.
#
# docs/verification.md: the same (raw + sidecar) exported on two architectures
# must be byte-identical ([HARD-DET]). CI runs this on x86_64 and aarch64 and
# diffs the two outputs; run `mise run determinism` to reproduce it locally.
#
#   HASHES_OUT   where to write the hash list   (default: hashes.txt)
#   HASHES_WORK  scratch dir for rendered files (default: a fresh mktemp dir)
set -euo pipefail

cd "$(dirname "$0")/.."

RAW=crates/focale-core/tests/fixtures/synthetic.dng
SIDECAR=crates/focale-cli/tests/fixtures/determinism.fcl
OUT="${HASHES_OUT:-hashes.txt}"
if [ -n "${HASHES_WORK:-}" ]; then
  WORK="$HASHES_WORK"
  mkdir -p "$WORK"
else
  # Ours to clean up; the renders are only inputs to the hashes.
  WORK="$(mktemp -d)"
  trap 'rm -rf "$WORK"' EXIT
fi
: > "$OUT"

hash=$(cargo run -q -p focale-cli -- hash "$RAW" --sidecar "$SIDECAR")
echo "working $hash" >> "$OUT"

for fmt in tiff16 png16 png8 jpeg jxl avif; do
  hash=$(cargo run -q -p focale-cli -- render "$RAW" --sidecar "$SIDECAR" \
    --format "$fmt" --gamut display-p3 --hash --out "$WORK/out.$fmt")
  echo "$fmt $hash" >> "$OUT"
done

hash=$(cargo run -q -p focale-cli -- render "$RAW" --sidecar "$SIDECAR" \
  --format avif --gamut rec2020 --hdr pq --hash --out "$WORK/out-hdr.avif")
echo "avif-hdr-pq $hash" >> "$OUT"

hash=$(cargo run -q -p focale-cli -- render "$RAW" --sidecar "$SIDECAR" \
  --format jxl --gamut rec2020 --hdr hlg --hash --out "$WORK/out-hdr.jxl")
echo "jxl-hdr-hlg $hash" >> "$OUT"

cat "$OUT"
