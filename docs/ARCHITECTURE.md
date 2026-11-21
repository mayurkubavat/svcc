# `uvir` Architecture: SV/UVM (1800-2023) <-> C++17 SystemC/UVM Compiler & Prover

## 1. Compiler Pipeline Overview

`uvir` focuses strictly on lossless, formally verified bidirectional compilation between **IEEE 1800-2023 SystemVerilog / IEEE 1800.2-2020 UVM (`sv-uvm`)** and **IEEE 1666-2023 SystemC 3.0.3 / UVM-SystemC / CRAVE / FC4SC (`sc-uvm`)**, built over a capability-legalized 6-dialect intermediate representation (`UVIR`):

```text
  [IEEE 1800-2023 SV/UVM]                   [C++17 SystemC/UVM + CRAVE + FC4SC]
     (svlang CST + Model)                      (StructuralCppExtractor / libclang)
             │                                                │
             ▼                                                ▼
 ┌────────────────────────────────────────────────────────────────────────────┐
 │ 6-Dialect `UvirModule` (`crates/svcc-ir`)                                  │
 │   `types` | `uvm` (RAL/TLM) | `crv` (Arena) | `cov` (UCIS) | `sched` | `sva`
 └────────────────────────────────────┬───────────────────────────────────────┘
                                      │
          ┌───────────────────────────┴───────────────────────────┐
          ▼                                                       ▼
 [Formal Verification (`svcc-equiv`)]          [Capability Legalizer (`svcc-passes`)]
 - Z3 `QF_BV` CRV/Enum/`dist` miter            - `OwnershipPass` (TLM value copy-ctor)
 - Z3 `K`-cycle SVA <-> C++17 observer BMC     - `WidthSignednessPass` (`sv_signedness`)
 - UCIS 1.0 covergroup/bin/cross isomorphism   - `TargetLegalizerPass` (`[v +/- t]` etc.)
                                                                  │
                    ┌─────────────────────────────────────────────┴───────────┐
                    ▼                                                         ▼
   [`svcc-emit-sv` + `svlang` self-check]             [`svcc-emit-sc` + `clang++ -Werror` check]
```

## 2. How Each SV <-> C++17 Semantic Impedance Gap Is Solved

1. **GC Handles vs. C++17 TLM-2.0 Value Copies & CRAVE Reference Aliasing**:
   - `OwnershipPass` flags any `sc-uvm` sequence item containing `crv_variable<T>` with `needs_value_copy_ctor = true`.
   - `svcc-emit-sc` synthesizes an explicit copy constructor and `operator=` copying only scalar values while binding fresh `crv_variable` / `crv_constraint` members to `this`.
2. **IEEE 1800-2023 `inside` Tolerances, `dist { := , :/ }`, `enum`, & `randc`**:
   - `TargetLegalizerPass` expands `[c +/- d]` and `[c +%- p]` into exact `[lo : hi]` ranges for CRAVE.
   - `dist` lowers to `crave::dist(v(), crave::distribution<T>().add(...).add_range(...).add_weighted_range(...))`.
   - `typedef enum bit [W-1:0]` maps to width-exact C++17 `enum class` + Z3 legal-variant domain constraints.
3. **`fork` / `join_any` / `disable fork` Concurrency**:
   - Lowered in `svcc-emit-sc` to `sc_core::sc_spawn` lambdas synchronized via `sc_event_or_list` (`|`) / `sc_event_and_list` (`&`) and cancelled safely via `std::shared_ptr<std::atomic<bool>>` tokens without leaking C++ RAII stack frames.
4. **`clocking cb @(posedge clk); default input #1step;` Sampling**:
   - Lowered to `uvir_clocking_<cb>` (`sc_core::sc_module`) exposing `sample_pre_timestep()` for SystemC 3.0.3 `SC_PRE_TIMESTEP` race-free preponed sampling.
5. **Pipelined SVA (`assert`/`cover property (@(posedge clk) disable iff (rst) a |-> ##[m:n] b)`)**:
   - Lowered to an $O(1)$-per-cycle, zero-allocation 64-bit shift-window observer (`uint64_t pending_window_` inside `uvir_sva_<name>`), formally proven cycle-accurate against IEEE 1800 SVA via `prove_sva_bmc_equivalence` in Z3.
6. **UVM RAL (`uvm_reg` + `uvm_reg_block`) & Component/TLM Topologies**:
   - `svlang_model::ral` extracts register widths, field bit-slices/access policies, and `create_map`/`add_reg` offsets, emitting `uvm::uvm_randomized_reg` (`bind_field`) + `uvm::uvm_reg_block` in C++17 and round-tripping back to SV RAL.
