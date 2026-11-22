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
| `hdl`: typed, elaborated RTL (parameters, generate, hierarchy, `case`/`casez`, loops, functions, selects, memories, packed structs, enums, vectors wider than 64 bits; see `docs/rtl_subset.md`) | Lower (`svlang` elaboration + bound expressions) | Emit (`SC_METHOD`, `sc_uint`/`sc_biguint`, IEEE 1666.1 style, fail-closed) | Differential simulation against Verilator on real SystemC (`svcc rtl-cosim`, `scripts/rtl_cosim.sh`) | Simulated (16/16 corpus designs) |
| `rtl` (`C2RTL`): Untimed C++17 golden functions + bounded loop SSA -> SV RTL | Emit + SEC Miter | Lower (`// svcc:c2rtl` SSA mux + loop unroller) | `svlang` `Netlist` -> Btor2 -> Z3 SEC (`latency = L`) | Verified (M2) |
| Bindings: Rust SDK (`crates/svcc`) + Python `PyO3` (`bindings/python`) | `svcc` crate | `svcc` `abi3-py39` wheel target | `cargo test --workspace` + `cargo check` (`svcc-py`) | Verified (M2) |

## Next actions

- SCAV probe set (`benchmarks/scav_compare/`): the typed `hdl` path converts all 18 probes and designs 13..20, and each matches Verilator under `svcc rtl-cosim` (20000 steps, checked on macOS arm64 and Linux x86_64). `benchmarks/results/scav_compare_summary.json` still holds the earlier legacy-path run against Icarus (svcc 3 of 18); regenerate it with `run_compare.py` on the typed path before quoting a head-to-head number.
- `roundtrip-check` and C2RTL SEC still use the legacy text-based RTL path, so the gap where 2 probes passed `roundtrip-check` but mismatched in co-simulation is still open in that gate. Move the gate onto the typed path or retire the legacy one.
- The probes and designs 13..20 are run by hand; only `testdata/rtl` (16 designs) is in `scripts/check.sh`.
- Local `main` is ahead of `origin/main` and unpushed; push only when Mayur asks.

## Session log

- 2026-11-22: Landed the `svcc-svlang` adapter crate (`8da6eaf`, pushed). Landed the SCAV head-to-head comparison benchmark locally (`12aa039`, unpushed): corpus designs 13..20 svcc 8/8 match, SCAV 0/8 compile; probes svcc 3/18, SCAV 0/18. `./scripts/check.sh` passes (12 tests).
- 2026-11-22: Brought the typed RTL path (`rtl/typed-hdl`, from the Linux clone) onto local `main`, unpushed. Made `rtl-cosim` find `libsystemc.dylib` so step 7 of `scripts/check.sh` runs on macOS. On macOS arm64 against SystemC `development` `9767bff` (C++17) in `~/.local/opt/accellera-svcc` with Verilator 5.052: `./scripts/check.sh` passes, corpus 16/16 with 3893207 output comparisons, SCAV probes 18/18, designs 13..20 8/8. Per-design comparison counts are identical on Linux; the 3830212 total in the earlier Linux log entry is stale.
