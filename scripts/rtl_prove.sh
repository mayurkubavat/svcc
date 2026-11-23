#!/usr/bin/env bash
# Formal gate of the typed RTL path: for every design, the SystemC that svcc generates is
# read back with C++ evaluation rules and proved against the SystemVerilog source
# (`svcc rtl-prove`). Needs z3 on PATH; needs neither SystemC nor Verilator.
#
# Three outcomes per design: proved, FAILED (a difference, or no answer), and NOT COVERED
# (the design is outside what the proof models; simulation is the only evidence for it).
# Without arguments the script runs the repository's designs and fails if a design is not
# covered that is not listed in EXPECT_NOT_COVERED below, so coverage cannot shrink silently.
# Usage: scripts/rtl_prove.sh [design.sv ...]
set -uo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
# array_elems, edges_misc, clock_gating, clock_vector: more than one clock or a computed clock
# (clock_vector gates each bank separately; clock_gating
# has a falling-edge clock and a clock made from register outputs).
# array_ports: multi-dimensional array port, and array ports in the generated header.
EXPECT_NOT_COVERED="array_elems.sv array_ports.sv clock_gating.sv clock_vector.sv edges_misc.sv"
FILES=("$@")
STRICT=0
if [[ ${#FILES[@]} -eq 0 ]]; then
  STRICT=1
  FILES=(testdata/rtl/*.sv benchmarks/scav_compare/probes/*.v benchmarks/corpus/1[3-9]_*.sv benchmarks/corpus/20_*.sv)
fi
SVCC="${SVCC:-target/debug/svcc}"
pass=0; fail=0; bounded=0; uncovered=0; failed=(); notcov=(); surprise=()
for f in "${FILES[@]}"; do
  n="$(basename "$f")"
  out="$("$SVCC" rtl-prove "$f" 2>&1)"; rc=$?
  # A module that instantiates itself is never a top on its own; name it after the file.
  stem="${n%.*}"
  if [[ $rc -ne 0 ]] && grep -q "no module found to convert" <<<"$out" && grep -qE "^[[:space:]]*module[[:space:]]+$stem\b" "$f"; then
    out="$("$SVCC" rtl-prove "$f" --top "$stem" 2>&1)"; rc=$?
  fi
  if [[ $rc -eq 0 ]]; then
    pass=$((pass + 1))
    if grep -q "(bounded)" <<<"$out"; then bounded=$((bounded + 1)); echo "$out"; fi
  elif [[ $rc -eq 3 ]]; then
    uncovered=$((uncovered + 1)); notcov+=("$n")
    echo "[----] $n"; grep "NOT COVERED" <<<"$out" | cut -c1-240 | sed 's/^/    /'
    if [[ $STRICT -eq 1 && " $EXPECT_NOT_COVERED " != *" $n "* ]]; then surprise+=("$n"); fi
  else
    echo "[FAIL] $n"; echo "$out" | sed 's/^/    /' | head -"${SVCC_PROVE_LINES:-16}"; fail=$((fail + 1)); failed+=("$n")
  fi
done
echo "rtl-prove: $pass proved ($((pass - bounded)) for all runs, $bounded bounded only), $uncovered not covered${notcov:+ (${notcov[*]})}, $fail failed${failed:+ (${failed[*]})}"
if [[ ${#surprise[@]} -gt 0 ]]; then
  echo "rtl-prove: not covered but expected to be proved: ${surprise[*]}" >&2
  exit 1
fi
[[ $fail -eq 0 ]]
