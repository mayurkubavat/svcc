#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -d ".github/workflows" ]]; then
  echo "FATAL: .github/workflows directory detected — GitHub Actions are strictly forbidden." >&2
  exit 1
fi

echo "==> [1/4] cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> [2/4] cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> [3/4] cargo test --workspace"
cargo test --workspace

echo "==> [4/4] CLI round-trip + Z3 QF_BV equivalence smoke check"
cargo run --quiet -p uvir-cli -- roundtrip-check testdata/sv/apb_timer_item.sv
cargo run --quiet -p uvir-cli -- prove-equiv testdata/sv/apb_timer_item.sv testdata/sc/apb_timer_item.hpp >/dev/null

echo "ALL CHECKS PASSED."
