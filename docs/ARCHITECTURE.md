# `uvir` Architecture: 3-Tier Multi-Language Hub-and-Spoke Verification IR

## 1. Why a Generic Middle Layer (`O(N)` vs. `O(N^2)`)

Modern silicon verification spans multiple languages across simulation speed, AI-driven stimulus generation, formal sign-off, and legacy VIP reuse:

| Spoke ID | Ecosystem | Primary Strength |
| :--- | :--- | :--- |
| `sv-uvm` | IEEE 1800-2023 SV + IEEE 1800.2-2020 UVM | Sign-off simulators, SVA, VIP ecosystem |
| `sc-uvm` | SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC | Native C++17 speed (`10x-50x` inner-loop iteration), VP co-sim |
| `py-uvm` | Python `cocotb 2.0` + `pyuvm` + `PyVSC` | AI/ML agent harnesses, rapid Python prototyping |
| `pss` | Accellera IEEE 1800.1 Portable Stimulus | SoC-level declarative intent & bare-metal C test synthesis |
| `rust-dv` | Rust + Verilator / Arcilator | Memory-safe parallel fuzzing & zero-overhead native testbenches |
| `vhdl-osvvm` | VHDL-2019 + OSVVM / UVVM | Safety-critical / aerospace FPGA & ASIC verification |

Building pairwise translators between $N$ languages requires $N(N-1)$ codebases and duplicates constraint-width inference, coverage bin normalization, and lifetime analysis everywhere.

`uvir` solves this with a **3-Tier Hub-and-Spoke Compiler Architecture**:

```text
  [SV/UVM]   [SystemC/CRAVE]   [Python/pyuvm]   [IEEE 1800.1 PSS]   [VHDL/OSVVM]
     │              │                 │                 │                 │
     ▼              ▼                 ▼                 ▼                 ▼
 ┌────────────────────────────────────────────────────────────────────────────┐
 │ Tier 1: High-Level Verification Semantic Core (`UVIR-H`, Language-Neutral) │
 │   Dialects: `types` | `uvm` (topo/RAL) | `crv` | `cov` (UCIS) | `sched` | `sva`
 └────────────────────────────────────┬───────────────────────────────────────┘
                                      │
                                      ▼  Shared Formal Proof (`uvir-equiv`: Z3 QF_BV + UCIS)
 ┌────────────────────────────────────────────────────────────────────────────┐
 │ Tier 2: Capability-Driven Target Legalizer (`TargetCapabilities` -> UVIR-L)│
 │   - OwnershipPass (`GarbageCollected` vs `ValueAndRaii` vs `Declarative`)  │
 │   - WidthSignednessPass (IEEE 1800 context widths -> explicit casts/types) │
 │   - TargetLegalizerPass (`[v +/- t]` expansion, 4-state split, SVA->FSM)   │
 └────────────────────────────────────┬───────────────────────────────────────┘
                                      │
     ┌──────────────┬─────────────────┼─────────────────┬─────────────────┐
     ▼              ▼                 ▼                 ▼                 ▼
 [Emit SV]      [Emit SC]        [Emit PyUVM]      [Emit PSS]        [Emit Rust]
```

## 2. The Three Architectural Rules Keeping the Generic Layer Clean

### Rule A: Never Leak Source Syntax into Core Dialects
Core `UVIR` dialects (`crates/uvir-ir`) represent **verification mathematics and semantics**, never C++ or SV syntax:
- **`types`**: Exact bit-widths (`BitVector`, `LogicVector`), explicit signedness, and 4-state flags — never raw C++ `unsigned long` or SV `reg`.
- **`crv`**: Contiguous `CrvArena` (`ExprId(u32)`, 4 bytes per edge, zero `Box` heap chasing) encoding pure bitvector/set logic (`Implies`, `Iff`, `Inside`, `Unique`, `ConstraintClause { is_soft, soft_weight }`).
- **`cov`**: Pure Accellera UCIS 1.0 hierarchy (`CovergroupDecl` -> `CoverpointDecl` -> `CovBin` -> `CrossDecl`).
- **`sched` & `sva`**: Structured concurrency (`JoinAll`, `JoinAny`, `JoinNone`, `cancel_on_exit`), sampling barriers (`SamplePreTimestep`), and clocked temporal properties.

Source-specific hints (e.g., original macro names or formatting pragmas) live in sidecar `Provenance` and `UvirModule::extensions` (`BTreeMap<String, serde_json::Value>`), validated by `DialectExtension` plugins. Passes that do not care about an extension preserve or drop it cleanly without breaking semantic correctness.

### Rule B: Declarative `TargetCapabilities` + Shared Legalization (`Tier 2`)
Languages differ sharply in runtime capabilities. Instead of hardcoding SV->SC or SV->Python hacks inside emitters, each `BackendEmitter` returns a `TargetCapabilities` struct:
1. **`memory_model`**:
   - `GarbageCollected` (`sv-uvm`, `py-uvm`): Objects pass by GC handle.
   - `ValueAndRaii` (`sc-uvm`, `rust-dv`): `OwnershipPass` marks `crv_variable` sequence items with `needs_value_copy_ctor = true` so TLM-2.0 by-value copies never corrupt Boost.Proto/solver internal pointers.
2. **`native_inside_tolerance`**:
   - `true` (`sv-uvm` IEEE 1800-2023 `[center +/- delta]` and `[center +%- pct]`).
   - `false` (`sc-uvm`, `py-uvm`, `rust-dv`): `TargetLegalizerPass` rewrites tolerance items into standard `InsideElem::Range { low, high }` inside `CrvArena` before the emitter ever runs.
3. **`native_four_state`**:
   - `true` (`sv-uvm` `logic`, `sc-uvm` `sc_lv<W>`).
   - `false` (`py-uvm`, `rust-dv`): Legalizer lowers 4-state vectors into `(value_bits, xz_mask_bits)` dual bitvectors.
4. **`solver_model`**:
   - `LanguageBuiltin` (`sv-uvm`, `pss`), `EmbeddedLibraryAst` (`sc-uvm` CRAVE, `py-uvm` PyVSC), or `SynthesizedSmtCall` (`rust-dv` / bare C++).
5. **`temporal_model`**:
   - `NativeSva` (`sv-uvm`), `NativePsl` (`vhdl-osvvm`), or `LoweredAutomaton` (`sc-uvm`, `py-uvm`, `rust-dv` — lowering temporal properties into cycle-accurate observer state machines in `sched`).

### Rule C: Universal Verification Equivalence at the Hub (`uvir-equiv`)
Because `uvir-equiv` encodes `CrvArena` directly into SMT-LIB2 `QF_BV` and checks UCIS coverage isomorphism on `UvirModule`, **adding a new language spoke requires zero new equivalence-prover code**. Once `py-uvm` lowers into `UvirModule`, `uvir prove-equiv pkt.sv pkt.py` proves `SAT(C_sv XOR C_py) == unsat` via Z3 out of the box.

## 3. Performance Design
- **Arena-Allocated IR (`CrvArena`)**: Constraint AST nodes live in a single flat `Vec<CrvExpr>` indexed by `ExprId(u32)` — cache-line dense, `Clone`/`Serialize`-trivial, and sub-100 microsecond per class.
- **Zero-Copy SV Front-End (`svlang`)**: Calls `svlang::Compilation` and `svlang_model::Design` directly in-process in Rust (zero IPC, zero JSON serialization overhead).
- **Thin-LTO Single-Binary CLI**: `[profile.release]` uses `lto = "thin"` and `codegen-units = 1`.
