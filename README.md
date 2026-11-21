# `svcc` — SystemVerilog <-> C/C++ (SystemC/UVM) Bidirectional Compiler & Prover

`svcc` is a high-performance Rust compiler, SDK, and Z3 formal equivalence prover translating bidirectionally between **IEEE 1800-2023 SystemVerilog / IEEE 1800.2-2020 UVM (`sv-uvm`)** and **IEEE 1666-2023 SystemC 3.0.3 / UVM-SystemC / CRAVE / FC4SC (`sc-uvm`, C++17)** over a 6-dialect intermediate representation (`UVIR`).

## Workspace & Packaging Layout

- **`crates/svcc`**: Top-level Rust SDK (`use svcc::{sv_to_sc, sc_to_sv, prove_sources_equivalent};`) + `svcc` CLI binary (`crates.io` name `svcc` available).
- **`crates/svcc-{ir,front-sv,front-sc,passes,equiv,emit-sc,emit-sv}`**: Modular compiler stages (`svlang` pinned at `478a0d10`).
- **`bindings/python/pyproject.toml`**: `maturin` + `PyO3` configuration for `pip install svcc` (`PyPI` name `svcc` available).
- **`docs/PACKAGING.md`**: Release blueprint for `crates.io` (`svcc`), `PyPI` (`svcc`), `npm`/WASM (`svcc`), and C ABI (`libsvcc`).
- **`docs/RTL_AND_C2RTL_ANALYSIS.md`**: Roadmap & IEEE 1666.1 boundaries for RTL conversion + `InFormal` `C2RTL` / `SLEC` integration.
- **`docs/PREDICTIONS.md` & `results/`**: Falsifiable predictions (`P1`–`P10`) and measured verification/benchmark logs.

## Quick Start

```bash
# Full gate: rustfmt + clippy (-D warnings) + SDK/CLI tests + clang++ -Werror + svlang + Z3 QF_BV & SVA BMC
./scripts/check.sh

# Round-trip check SV/UVM -> C++17 (clang++ -Werror) -> SV (svlang) + Z3 proofs
cargo run --release -p svcc -- roundtrip-check testdata/sv/axi4_burst_env.sv --shims testdata/cpp_shims
cargo run --release -p svcc -- roundtrip-check testdata/sv/uart_ral_sva.sv --shims testdata/cpp_shims
```
