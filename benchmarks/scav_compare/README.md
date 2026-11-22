# SCAV head-to-head (RTL, Verilog -> SystemC)

Compares svcc with SCAV (https://github.com/zsf1998/SCAV, branch `all`, commit `2ef2e6b`), the
released code behind "High-Efficiency Bidirectional Translator between SystemC and Verilog"
(ACM JETC 2026, doi 10.1145/3798042).

    cargo build --release -p svcc
    python3 benchmarks/scav_compare/run_compare.py

- `probes/` holds 18 small synthesizable Verilog-2005 designs written for this study.
- `cosim.py` drives the same random stimulus into the original RTL (Icarus Verilog) and the
  generated SystemC module (real Accellera SystemC) and diffs the outputs per cycle.
- `run_compare.py` runs both translators on the probes, on corpus designs 13..20 and on SCAV's
  own examples, and writes `benchmarks/results/scav_compare_summary.json`.

SCAV is cloned at run time into the output directory. Nothing from it is stored here: only its
`v2sc/` directory carries a license file (Apache-2.0, from Pyverilog); `sc2v/` has none.

Needs g++, libsystemc-dev, iverilog, z3 and the Python `ply` package.
