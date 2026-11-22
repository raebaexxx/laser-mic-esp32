#!/usr/bin/env bash
set -euo pipefail

# CI-style checks.  Does not build the ESP32 firmware (it needs the Espressif
# toolchain); see firmware/README.md.
cd "$(dirname "$0")/.."

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p granite-sim --bin granite-twin -- selftest

echo "all host checks passed"
