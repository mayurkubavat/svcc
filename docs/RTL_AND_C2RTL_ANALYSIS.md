# RTL Conversion & `InFormal` C2RTL / SLEC Integration Analysis

Saved design analysis covering:
1. Extending the compiler beyond UVM/DV (`types`, `uvm`, `crv`, `cov`, `sched`, `sva`) to **bidirectional RTL (`SV RTL <-> SystemC/C++17 RTL & Golden Models`)**.
2. Exact technical limitations and fail-closed boundaries in both directions.
3. Enabling a **C++/SystemC-to-RTL Sequential Equivalence Checking (`C2RTL` / `DPV` / `SLEC`)** Formal App inside **`InFormal`** on top of `svlang` and `check_sec`.

---

## 1. Feasibility Over Our Existing `svlang` Foundation

Pinned `svlang` (`478a0d10`) already provides:
- Full IEEE 1800-2023 RTL elaboration (`module`, `interface`/`modport`, `generate`, parameters, `always_ff`, `always_comb`, `always_latch`).
- `sv-netlist`: Word-level RTL netlist IR with symbolic procedural execution, bit-level driver analysis, memory blasting, and bound SVA properties.
- `sv-btor2` & `sv-aiger`: Word-level Btor2 and bit-blasted AIGER transition-system writers.

Adding an **`rtl` dialect** (`Module`, `Port`, `CombProcess`, `SyncProcess`, `Signal`, `Instance`) alongside the 6 DV dialects unifies **RTL + UVM Testbench** transpilation in one binary — eliminating VPI/DPI marshaling overhead in hybrid simulations and enabling end-to-end formal equivalence.

---

## 2. Six Hard Technical Limitations & Boundaries

| # | Limitation | Primary Direction | Root Cause & Fail-Closed Strategy |
| :--- | :--- | :--- | :--- |
| **1** | **Unrestricted C++17 vs. IEEE 1666.1 Synthesizable Subset** | `SystemC -> SV RTL` | Heap allocation (`new`, dynamic `std::vector`), pointers, recursion, and virtual dispatch inside `SC_METHOD`/`SC_CTHREAD` have no hardware equivalent. Must enforce the **IEEE 1666.1 synthesizable subset** (`SC_MODULE`, static ports/signals, bounded loops, `sc_uint`/`sc_int`/`sc_bv`/`ac_int`/fixed structs) and fail closed with exact `file:line` diagnostics otherwise. |
| **2** | **Multi-`wait()` `SC_CTHREAD` Implicit-State Threads** | `SystemC -> SV RTL` | Converting a multi-`wait()` `SC_CTHREAD` to `always_ff` SV requires synthesizing explicit FSM state registers (`state_q`). This alters internal register topology (requiring $k$-step sequential equivalence checking rather than 1-to-1 flop LEC, and preventing verbatim line-for-line round-trip back to multi-`wait()` C++). |
| **3** | **Elaboration Flattening vs. `#(parameter W)` <-> `template <int W>`** | Both | Post-elaboration lowering (`sv-netlist`) handles arbitrary `generate` and `$clog2` bit-exactly, but flattens `#(parameter int W = 32)` into concrete specializations (`mod_W32`). Preserving parameterized templates requires a pre-elaboration structural tier restricted to parameters that do not conditionally add/remove ports beyond C++17 `if constexpr` idioms. |
| **4** | **SV Stratified Event Queue vs. SystemC Evaluate-Update Deltas** | Both | SV `always_comb` auto-computes callee sensitivity and fires at time 0 after `initial`; SystemC `SC_METHOD` requires explicit `sensitive << ...` lists, and `sc_signal::write()` commits on the next delta (like `<=`), whereas C++ member variables commit immediately (like `=`). Shared procedural variables must be SSA-normalized before emission. |
| **5** | **4-State Tri-States (`inout`), Drive Strengths, & Switch Primitives** | `SV RTL -> SystemC` | `tran`/`nmos`, `strong1`/`pull0`, UDPs, and multi-driver `wand`/`wor` nets lack fast native C++17 equivalents (`sc_signal_resolved` + `sc_lv` incurs a $5\times\text{--}10\times$ penalty and still omits strength levels). Scope to single-driver ASIC RTL + top-level pad `{in, out, oe}` splitting. |
| **6** | **Sub-Cycle Combinational Glitches, Latches, & Async CDC** | `SV RTL -> SystemC` | Cycle-scheduled C++ collapses intra-cycle glitches and transparent latches (`always_latch`); event-driven `SC_METHOD` preserves multi-clock edges better at higher scheduling cost. |

---

## 3. Enabling C++/SystemC vs. RTL Model Checking (`C2RTL` / `DPV` / `SLEC`) in `InFormal`

Commercial formal verification suites (**Synopsys VC Formal DPV / Hector**, **Cadence Jasper C2RTL**, **Siemens SLEC**) prove golden C++/SystemC architectural models against pipelined, clock-gated SystemVerilog RTL.

`InFormal` already implements the formal backend via `check_sec`:
- Dual-design product machine (`spec_files` vs. `imp_files`),
- Fixed pipeline skew (`latency: L`) and combinational keypoint LEC (`lec: true`),
- Primary-input `assumptions`, `cut_point` datapath abstraction, `convergence_diagnose`, and `cex_diagnose`.

### Three Workflows Unlocked by Connecting This Compiler to `InFormal`

1. **Untimed C++17 Algorithmic Reference vs. Pipelined/Stalled SV RTL (*DPV / C2RTL Mode*)**:
   - Unroll bounded C++ loops, convert mutable locals and early `return`s into SSA mux trees (`ite`), and emit a `0`-cycle `spec.sv` (with `(* src = "golden.cpp:LINE" *)` attributes) or directly construct `svlang::netlist::Netlist`.
   - **Fixed latency**: Feed directly to `InFormal` `check_sec(..., latency=L)`.
   - **Variable latency (`valid`/`ready` stalls)**: Emit a symbolic tracking-FIFO miter (`push(spec(in))` on `in_fire`, `assert(imp_out == pop())` on `out_fire`) for `InFormal` `prove` + `cex_diagnose`.
2. **Cycle-Accurate `SC_MODULE` vs. Retimed / Clock-Gated SV RTL (*SLEC Mode*)**:
   - Lower synthesizable `SC_MODULE` (`SC_METHOD` / `SC_CTHREAD`) into `spec.sv` / `sv-netlist` and prove sequential equivalence via $k$-induction / PDR + `cut_point` multiplier decomposition.
3. **Single-Source CRAVE / UVM Constraints -> D17-Safe `InFormal` Primary-Input `assume`s**:
   - Emit input legality constraints from `crv` directly into `check_sec(assumptions=[...])`, preventing drift between simulation CRV and formal assumptions while strictly respecting `InFormal` Decision D17 (primary-input boundary only).

### Four Mandatory Soundness Requirements for the C2RTL Front-End
1. **True `libclang` Semantic AST (`--features libclang`)**: Required to capture exact C++ integer promotions (`uint8_t + uint8_t -> int`), signed/unsigned conversions, `constexpr` templates, and overloaded operators on `sc_uint<W>`, `sc_int<W>`, `ac_int<W,S>`, and `ap_uint<W>`.
2. **Bounded Symbolic Execution & SSA Loop Unroller**: Inlines helper calls, unrolls statically bounded loops, converts early `return`/`break` paths into guarded `ite` muxes, and fails closed on input-dependent unbounded loops.
3. **Bit-Exact Fixed-Point Rounding & Saturation (`sc_fixed` / `ac_fixed`)**: Lowers `SC_RND`/`SC_TRN` and `SC_SAT`/`SC_WRAP` into exact bitvector arithmetic for DSP/AI datapath proofs.
4. **Source-Span Provenance Preservation**: Threads C++ `file:line:col` through `UVIR` -> `spec.sv` / `sv-netlist` so `InFormal` `cex_diagnose` highlights the exact C++ reference line alongside the diverging SV RTL driver line.
