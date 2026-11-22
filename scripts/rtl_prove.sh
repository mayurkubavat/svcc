#!/usr/bin/env bash
# Formal gate of the typed RTL path: for every design, the SystemC that svcc generates is
# read back with C++ evaluation rules and proved against the SystemVerilog source
# (`svcc rtl-prove`). Needs z3 on PATH; needs neither SystemC nor Verilator.
# Usage: scripts/rtl_prove.sh [design.sv ...]
set -uo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
FILES=("$@")
if [[ ${#FILES[@]} -eq 0 ]]; then
  FILES=(testdata/rtl/*.sv benchmarks/scav_compare/probes/*.v benchmarks/corpus/1[3-9]_*.sv benchmarks/corpus/20_*.sv)
fi
SVCC="${SVCC:-target/debug/svcc}"
pass=0; fail=0; bounded=0; failed=()
for f in "${FILES[@]}"; do
  n="$(basename "$f")"
  if out="$("$SVCC" rtl-prove "$f" 2>&1)"; then
    pass=$((pass + 1))
    if grep -q "(bounded)" <<<"$out"; then bounded=$((bounded + 1)); echo "$out"; fi
  else
    echo "[FAIL] $n"; echo "$out" | sed 's/^/    /' | head -"${SVCC_PROVE_LINES:-16}"; fail=$((fail + 1)); failed+=("$n")
  fi
done
echo "rtl-prove: $pass proved ($((pass - bounded)) for all runs, $bounded bounded only), $fail failed${failed:+ (${failed[*]})}"
[[ $fail -eq 0 ]]
