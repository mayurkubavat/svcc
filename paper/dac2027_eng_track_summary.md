# DAC 2027 Engineering Track Extended Summary: `svcc`

- **Title:** `svcc`: Translation-Validated Bidirectional Compilation Between IEEE 1800 SystemVerilog/UVM and C++17 SystemC
- **Author:** Mayur Kubavat (`Independent Researcher, Mountain View, CA, USA`, `mkubavat01@gmail.com`, ORCID `0009-0009-1087-8558`)
- **Primary Track:** Design Verification, Validation, and Formal Equivalence (Engineering Track / Open-Source EDA)
- **arXiv Categories:** `cs.AR` (Hardware Architecture), `cs.PL` (Programming Languages), `cs.SE` (Software Engineering)

## 1. Industrial Problem Statement
Verification teams and AI hardware coding agents iterate rapidly in C++17 / IEEE 1666-2023 SystemC (sub-second `clang++` rebuilds, zero license contention, native UVM-SystemC + CRAVE + FC4SC libraries), while tape-out sign-off requires IEEE 1800-2023 SystemVerilog RTL, IEEE 1800.2-2020 UVM VIPs, and concurrent SVA monitors. Ad-hoc Python translators and unverified LLM rewrites introduce silent bugs across:
1. TLM value-copy constructors corrupting `crave::crv_variable<T>` solver references.
2. Context-determined SV bitvector widths vs. C++ `int` promotion (`uint8_t + uint8_t -> int`).
3. Preponed `#1step` clocking vs. SystemC delta-cycle `posedge_event()` races.
4. Overlapping pipelined `|-> ##[m:n]` SVA assertions.
5. Superlinear `clang++ -O2` template instantiation on large Boost.Proto `crv_constraint` blocks.

## 2. Key Technical Innovations in `svcc`
- **7-Dialect `UVIR` (`svcc-ir`) & Capability Legalizer (`svcc-passes`):** Lowers both SV and C++17 into orthogonal `types`, `uvm`, `crv` (`CrvArena`), `cov` (UCIS 1.0), `sched`, `sva`, and `rtl` dialects normalized by target-capability passes (`WidthSignednessPass`, `FieldOpSynthesisPass`, `OwnershipPass`, `TargetLegalizerPass`).
- **Hybrid `svlang` Semantic + Lossless CST Recovery (`svcc-front-sv`):** Recovers `uvm_reg` / `uvm_reg_block` specializations, `extends` hierarchies, `` `uvm_field_int `` flags, and parameterized TLM port types on standalone VIP files without `-f` `uvm_pkg` closure (`12/12` valid vs. `0/12` semantic-only).
- **Associative Clause Partitioning (`svcc-emit-sc` / `svcc-front-sc`):** Splits large CRAVE constraints (`--partition-clauses 8`) to cut `clang++ -O2` compile latency by **`5.95x`** at $K=120$ clauses (`1,712.0 ms -> 287.5 ms`) with exact round-trip re-merging.
- **4-Way Solver Translation Validation & `C2RTL` SEC (`svcc-equiv`):**
  1. Z3 `QF_BV` CRV + Enum + `dist` support miter.
  2. $K$-step Z3 BMC proof of generated $O(1)$ `uint64_t pending_window_` C++17 SVA observers.
  3. Bijective UCIS 1.0 covergroup/bin/cross isomorphism.
  4. IEEE 1666.1 fail-closed C++17 `// svcc:c2rtl` SSA lowering + `svlang` `Netlist` -> `sv-btor2` -> Z3 SEC across pipeline latency $L \ge 0$ (plus a `--no-default-features` zero-`svlang` Rust SDK slice for unbounded IC3/PDR sign-off in `InFormal`).

## 3. Summary of Verified Empirical Results (`benchmarks/results/m3_benchmark_summary.json`)
- **20-Design Corpus (`803` SV LOC + `8` C2RTL Golden Models):** `20/20` pass `clang++ -Werror` and `svlang`; `200 us` median `sv2sc` and `210 us` median `sc2sv`; `20/20` Z3 round-trip `EquivalentUnsat` (`6.82 ms` median); `8/8` C2RTL `EquivalentUnsat` across $L \in \{0, 1, 2\}$ (`5.90 ms` median).
- **Mutation Sensitivity & Specificity:** `20/20` (`100%`) semantic bugs caught across CRV, SVA, UCIS, and RTL/C2RTL; `4/4` (`100%`) non-trivial AST-divergent equivalent rewrites proved `EquivalentUnsat` (`0%` false alarms).
