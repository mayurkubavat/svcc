# `svcc` Milestone & SV <-> C++17 Capability Status

Scoped strictly to **IEEE 1800-2023 SystemVerilog (RTL + UVM)** <-> **IEEE 1666-2023 SystemC 3.0.3 / C++17 (RTL + UVM-SystemC + CRAVE + FC4SC + C2RTL)**.

| Dialect / Construct | `sv-uvm` / SV RTL (1800-2023) | `sc-uvm` / C++17 (1666-2023 + 1666.1) | Verification Gate | Status |
| :--- | :--- | :--- | :--- | :--- |
| `types`: 2/4-state vectors, widths, signedness, `typedef enum` | Lower + Emit | Lower + Emit (`uint*_t`, `sc_uint`, `sc_lv`, `enum class`) | `clang++ -Werror` + `svlang` + Z3 enum domain | Verified (M1) |
| `uvm`: `sequence_item`, field ops (`UVM_NOCOMPARE`/`NOCOPY`), copy-ctor | Lower + Emit | Lower + Emit (`uvm_randomized_sequence_item`, value copy-ctor) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `uvm`: RAL `uvm_reg` & `uvm_reg_block` (`create_map`, `add_reg`) | Lower + Emit | Lower + Emit (`uvm_randomized_reg`, `bind_field`, `uvm_reg_block`) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `uvm`: Component hierarchy & TLM (`analysis_port`, `connect_phase`) | Lower + Emit | Lower + Emit (`type_id::create`, `.connect(...)`) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `crv`: Hard/soft, `->`, `inside`, `[v +/- t]`, `unique`, `dist { :=, :/ }`, `randc` | Lower + Emit | Lower + Emit (`crv_constraint`, `crv_soft_constraint`, `crave::dist`) | Z3 `QF_BV` `EquivalentUnsat` + bug-injection `sat` | Verified (M1) |
| `cov`: `covergroup`, `coverpoint`, range/value/ignore bins, `cross` | Lower + Emit | Lower + Emit (`fc4sc::covergroup`, `COVERPOINT`, `fc4sc::cross`) | UCIS 1.0 isomorphism | Verified (M1) |
| `sched`: `fork`/`join_any`/`disable fork`, `#delay`, `clocking #1step` | Lower + Emit | Lower + Emit (`sc_spawn` + `atomic<bool>` cancel token + `uvir_clocking_*`) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `sva`: Clocked `assert`/`cover property` (`\|-> ##[m:n]`, `disable iff`) | Lower + Emit | Lower + Emit (`uvir_sva_*` 64-bit `O(1)` shift-window observer) | Z3 `K`-cycle BMC `EquivalentUnsat` | Verified (M1) |
| `rtl`: `module`, `always_comb`, `always_ff` <-> `SC_MODULE` (`SC_METHOD`, `SC_CTHREAD`) | Lower + Emit | Lower + Emit (auto `sensitive <<`, IEEE 1666.1 fail-closed) | `clang++ -Werror` + `svlang` `Netlist` + Btor2/Z3 SEC | Verified (M2) |
| `rtl` (`C2RTL`): Untimed C++17 golden functions + bounded loop SSA -> SV RTL | Emit + SEC Miter | Lower (`// svcc:c2rtl` SSA mux + loop unroller) | `svlang` `Netlist` -> Btor2 -> Z3 SEC (`latency = L`) | Verified (M2) |
| Bindings: Rust SDK (`crates/svcc`) + Python `PyO3` (`bindings/python`) | `svcc` crate | `svcc` `abi3-py39` wheel target | `cargo test --workspace` + `cargo check` (`svcc-py`) | Verified (M2) |

## Next actions

- Triage the SCAV probe set (`benchmarks/scav_compare/`): svcc matches the Icarus reference on 3 of 18 probes. First the 2 probes that pass `roundtrip-check` but mismatch in co-simulation (a gap in the equivalence gate), then the 15 that fail earlier.
- Local `main` is ahead of `origin/main` and unpushed; push only when Mayur asks.

## Session log

- 2026-11-22: Landed the `svcc-svlang` adapter crate (`8da6eaf`, pushed). Landed the SCAV head-to-head comparison benchmark locally (`12aa039`, unpushed): corpus designs 13..20 svcc 8/8 match, SCAV 0/8 compile; probes svcc 3/18, SCAV 0/18. `./scripts/check.sh` passes (12 tests).
