#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -d ".github/workflows" ]]; then
  echo "FATAL: .github/workflows directory detected — GitHub Actions are strictly forbidden." >&2
  exit 1
fi

echo "==> [1/5] cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> [2/5] cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> [3/5] cargo test --workspace"
cargo test --workspace

echo "==> [4/5] CLI SV <-> C++17 (clang++ -Werror + svlang + Z3 CRV/SVA/RTL-SEC/C2RTL) suite"
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/apb_timer_item.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/axi4_burst_env.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/uart_ral_sva.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/alu_pipe_rtl.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- prove-sec testdata/sc/alu_golden_c2rtl.hpp testdata/sv/alu_pipe_rtl.sv --latency 2 >/dev/null
cargo run --quiet -p svcc -- prove-equiv testdata/sv/apb_timer_item.sv testdata/sc/apb_timer_item.hpp >/dev/null

echo "==> [5/6] Zero-svlang decoupling check for InFormal embedding (--no-default-features)"
cargo clippy --no-default-features -p svcc-ir -p svcc-front-sc -p svcc-passes -p svcc-emit-sv -p svcc-equiv -- -D warnings

echo "==> [6/6] PyO3 Python extension crate check (bindings/python)"
cargo check --manifest-path bindings/python/Cargo.toml

echo "ALL CHECKS PASSED."
