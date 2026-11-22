#!/usr/bin/env bash
set -euo pipefail

# Build, test and run the complete software demo.
# Usage: ./scripts/demo.sh [output-dir]

cd "$(dirname "$0")/.."
OUT="${1:-out/demo}"

echo "== host tests =="
cargo test --workspace

echo "== digital-twin suite =="
cargo run -q -p granite-sim --bin granite-twin -- suite --out "$OUT/suite"

echo "== host analysis of the healthy twin capture =="
cargo run -q -p granite-host --bin granite-host -- analyze \
  --wav "$OUT/suite/healthy/capture.wav" \
  --reference "$OUT/suite/healthy/reference.txt" \
  --out "$OUT/host-healthy"

echo
echo "Open: $OUT/suite/showcase.html     (interactive dashboard)"
echo "      $OUT/suite/crack/scope.html   (single animated scope + audio)"
echo "      $OUT/host-healthy/report.html (host analysis report)"

if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && command -v xdg-open >/dev/null 2>&1; then
  xdg-open "$OUT/suite/showcase.html" >/dev/null 2>&1 &
fi
