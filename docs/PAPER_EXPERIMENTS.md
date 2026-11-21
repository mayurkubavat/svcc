# Publication & Experimental Evaluation Protocol

## Core Thesis
> **"Transpile -> Scale Closure at Native C++17 Speed -> Prove & Deploy in Target Language of Choice"**:
> Using a capability-legalized Universal Verification IR (`UVIR`) with automated Z3 `QF_BV` constraint equivalence and UCIS 1.0 coverage isomorphism proofs to close AI-driven RTL/DV loops in native SystemC/C++17 and deploy verified collateral back to IEEE 1800-2023 SystemVerilog/UVM (or Python/PSS).

## Target Venues
1. **DAC 2027 Engineering Track** (Deadline: Jan 11, 2027) — Tool architecture + industrial VIP round-trip speedups.
2. **arXiv (`cs.AR` / `cs.PL`)** — Technical report + reproducible benchmark suite.
3. **IEEE/ACM ICCAD 2027 / ACM/IEEE MLCAD 2027 / IEEE LAD 2027** — Full research paper (*UVIR + Z3 Equivalence + AI Verification Loop*).
4. **DVCon US / Europe / India** — Practitioner deep dive on solving the 7 SV <-> SystemC semantic impedance mismatches.
5. **IEEE TCAD / Elsevier *Integration*** — Archival journal extension covering multi-language (`SV`, `SystemC`, `Python/pyuvm`, `PSS`) hub-and-spoke legalization.

## Benchmark Suite & Tracked Metrics (`results/experiment_log.jsonl`)
Every milestone records:
- **Compilation & Transpilation Latency (`avg_sv2sc_us`, `avg_sc2sv_us`)**: Microseconds per class/package across APB4, AXI4-Lite, UART-16550, and multi-channel DMA VIPs.
- **Formal Equivalence Rate (`z3_verdict`, `z3_solver_us`)**: Percentage of constraint blocks proven `EquivalentUnsat` via SMT-LIB2 `QF_BV` and off-by-one/signedness mutations caught (`CounterexampleSat`).
- **Coverage Closure Isomorphism (`coverage_isomorphic`)**: Exact bin/cross cardinality match in UCIS 1.0.
- **Simulation Wall-Clock Speedup**: `10^4 - 10^6` transaction regressions on SystemC+Verilator vs. SV/UVM simulators, plus compile-edit-run inner-loop latency for AI harnesses.
