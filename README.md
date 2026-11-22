# `svcc` — Bidirectional SystemVerilog/UVM and SystemC Compiler with SMT Translation Validation

`svcc` is an open-source Rust compiler, Python SDK (`svcc-py`), and Z3/Btor2 translation-validation verifier (`12,600` LOC) translating bidirectionally between **IEEE 1800-2023 SystemVerilog / IEEE 1800.2-2020 UVM (`sv-uvm`)** and **IEEE 1666-2023 SystemC 3.0.3 / UVM-SystemC / CRAVE / FC4SC (`sc-uvm`, C++17)** and untimed C++17 reference models (`C2RTL`) over a 7-dialect intermediate representation (`svcc-ir`: `types`, `uvm`, `crv`, `cov`, `sched`, `sva`, `rtl`).

## Workspace & Artifacts

- **`crates/svcc`**: Top-level Rust SDK (`use svcc::{sv_to_sc, sc_to_sv, prove_sources_equivalent, compile_cpp_to_btor2};`) + `svcc` CLI binary.
- **`crates/svcc-{ir,front-sv,front-sc,passes,equiv,emit-sc,emit-sv}`**: Modular front ends (`svlang` semantic + lossless CST recovery, `svlang-lint` emission gate, C++17 clause splitter & SSA lowers), semantic legalizer (`P2`/`P3`/`P5` CRAVE passes), and Z3 `QF_BV` / SVA BMC / UCIS 1.0 / Btor2 SEC provers.
- **`crates/svcc-svlang`**: The only crate that depends on `svlang`, `svlang-model` and `svlang-lint`; the front end, emitter lint gate and Btor2 SEC reach `svlang` through it.
- **`bindings/python`**: `maturin` + `PyO3` Python SDK (`svcc-py`).
- **`benchmarks/`**: 20-design benchmark suite (`run_benchmarks.py`), live Accellera SystemC 3.0.3 / UVM-SystemC / CRAVE / FC4SC evaluation (`run_m4_extended.py`), and external open-source corpus audit (`ibex`, `cv32e40p`, `uvm-verilator`, `accellera-crave`).
- **`paper/`**: Technical report source (`paper/main.tex`, `paper/refs.bib`) and compiled PDF (`paper/main.pdf`).

## Quick Start

```bash
# Full gate: rustfmt + clippy (-D warnings) + SDK/CLI tests + clang++ -Werror + svlang + svlang-lint + Z3 QF_BV / SVA BMC / Btor2 SEC
./scripts/check.sh

# Round-trip check SV/UVM -> C++17 (clang++ -Werror) -> SV (svlang + svlang-lint) + Z3 proofs
cargo run --release -p svcc -- roundtrip-check testdata/sv/axi4_burst_env.sv --shims testdata/cpp_shims
cargo run --release -p svcc -- roundtrip-check testdata/sv/uart_ral_sva.sv --shims testdata/cpp_shims

# Reproduce benchmark suite and live Accellera CRAVE / external corpus measurements
python3 benchmarks/run_benchmarks.py
python3 benchmarks/run_m4_extended.py
```

## License

Apache License 2.0 (`LICENSE`).
