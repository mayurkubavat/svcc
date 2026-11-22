#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -d ".github/workflows" ]]; then
  echo "FATAL: .github/workflows directory detected — GitHub Actions are strictly forbidden." >&2
  exit 1
fi

echo "==> [1/8] cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> [2/8] cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> [3/8] cargo test --workspace"
cargo test --workspace

echo "==> [4/8] CLI SV <-> C++17 (clang++ -Werror + svlang + Z3 CRV/SVA, typed RTL proof, C2RTL) suite"
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/apb_timer_item.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/axi4_burst_env.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/uart_ral_sva.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- roundtrip-check testdata/sv/alu_pipe_rtl.sv --shims testdata/cpp_shims
cargo run --quiet -p svcc -- prove-sec testdata/sc/alu_golden_c2rtl.hpp testdata/sv/alu_pipe_rtl.sv --latency 2 >/dev/null
cargo run --quiet -p svcc -- prove-equiv testdata/sv/apb_timer_item.sv testdata/sc/apb_timer_item.hpp >/dev/null

echo "==> [5/8] Zero-svlang decoupling check for InFormal embedding (--no-default-features)"
cargo clippy --no-default-features -p svcc-ir -p svcc-front-sc -p svcc-passes -p svcc-emit-sv -p svcc-equiv -- -D warnings

echo "==> [6/8] PyO3 Python extension crate check (bindings/python)"
cargo check --manifest-path bindings/python/Cargo.toml

echo "==> [7/8] Typed RTL path: generated SystemC proved against its source (corpus, probes, designs 13..20)"
cargo build --quiet -p svcc
scripts/rtl_prove.sh

echo "==> [8/8] Typed RTL path: generated SystemC vs Verilator differential simulation (testdata/rtl)"
if [[ "${SVCC_SKIP_COSIM:-0}" == "1" ]]; then
  echo "SKIPPED by SVCC_SKIP_COSIM=1 (the generated SystemC was proved in step 7 but NOT simulated against Verilator in this run)."
else
  command -v verilator >/dev/null || { echo "FATAL: verilator not on PATH (set SVCC_SKIP_COSIM=1 to skip explicitly)." >&2; exit 1; }
  cargo build --quiet -p svcc
  scripts/rtl_cosim.sh
fi

echo "ALL CHECKS PASSED."
