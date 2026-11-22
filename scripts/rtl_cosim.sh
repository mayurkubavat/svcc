#!/usr/bin/env bash
# Differential simulation of svcc-generated SystemC against Verilator over testdata/rtl.
# Needs verilator on PATH and SYSTEMC_HOME (or ~/.local/opt/accellera/systemc).
# Usage: scripts/rtl_cosim.sh [--steps N] [design.sv ...]
set -uo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
STEPS=20000
if [[ "${1:-}" == "--steps" ]]; then STEPS="$2"; shift 2; fi
FILES=("$@")
if [[ ${#FILES[@]} -eq 0 ]]; then FILES=(testdata/rtl/*.sv); fi
SVCC="${SVCC:-target/debug/svcc}"
WORK="${SVCC_COSIM_DIR:-target/rtl-cosim}"
pass=0; fail=0; failed=()
for f in "${FILES[@]}"; do
  n="$(basename "$f" .sv)"
  if out="$("$SVCC" rtl-cosim "$f" --top "$n" --steps "$STEPS" --keep "$WORK/$n" 2>&1)"; then
    echo "$out"; pass=$((pass + 1))
  else
    echo "[FAIL] $n"; echo "$out" | sed 's/^/    /' | head -${SVCC_COSIM_LINES:-14}; fail=$((fail + 1)); failed+=("$n")
  fi
done
echo "rtl-cosim: $pass passed, $fail failed${failed:+ (${failed[*]})}"
[[ $fail -eq 0 ]]
