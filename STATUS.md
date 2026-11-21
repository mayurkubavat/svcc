# `uvir` Milestone & SV <-> C++17 Capability Status

Scoped strictly to **IEEE 1800-2023 SystemVerilog / IEEE 1800.2-2020 UVM** (`sv-uvm`) <-> **IEEE 1666-2023 SystemC 3.0.3 / UVM-SystemC / CRAVE / FC4SC** (`sc-uvm`).

| Dialect / Construct | `sv-uvm` (1800-2023) | `sc-uvm` (C++17 + CRAVE + FC4SC) | Verification Gate | Status |
| :--- | :--- | :--- | :--- | :--- |
| `types`: 2/4-state vectors, widths, signedness, `typedef enum` | Lower + Emit | Lower + Emit (`uint*_t`, `sc_uint`, `sc_lv`, `enum class`) | `clang++ -Werror` + `svlang` + Z3 enum domain | Verified (M1) |
| `uvm`: `sequence_item`, field ops (`UVM_NOCOMPARE`/`NOCOPY`), copy-ctor | Lower + Emit | Lower + Emit (`uvm_randomized_sequence_item`, value copy-ctor) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `uvm`: RAL `uvm_reg` & `uvm_reg_block` (`create_map`, `add_reg`) | Lower + Emit | Lower + Emit (`uvm_randomized_reg`, `bind_field`, `uvm_reg_block`) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `uvm`: Component hierarchy & TLM (`analysis_port`, `connect_phase`) | Lower + Emit | Lower + Emit (`type_id::create`, `.connect(...)`) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `crv`: Hard/soft, `->`, `inside`, `[v +/- t]`, `unique`, `dist { :=, :/ }`, `randc` | Lower + Emit | Lower + Emit (`crv_constraint`, `crv_soft_constraint`, `crave::dist`) | Z3 `QF_BV` `EquivalentUnsat` + bug-injection `sat` | Verified (M1) |
| `cov`: `covergroup`, `coverpoint`, range/value/ignore bins, `cross` | Lower + Emit | Lower + Emit (`fc4sc::covergroup`, `COVERPOINT`, `fc4sc::cross`) | UCIS 1.0 isomorphism | Verified (M1) |
| `sched`: `fork`/`join_any`/`disable fork`, `#delay`, `clocking #1step` | Lower + Emit | Lower + Emit (`sc_spawn` + `atomic<bool>` cancel token + `uvir_clocking_*`) | `clang++ -Werror` + `svlang` | Verified (M1) |
| `sva`: Clocked `assert`/`cover property` (`\|-> ##[m:n]`, `disable iff`) | Lower + Emit | Lower + Emit (`uvir_sva_*` 64-bit `O(1)` shift-window observer) | Z3 `K`-cycle BMC `EquivalentUnsat` | Verified (M1) |
