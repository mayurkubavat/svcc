# DAC 2027 Engineering Track Extended Summary: `svcc`

- **Title:** `svcc`: Translation-Validated Bidirectional Compilation Between IEEE 1800 SystemVerilog/UVM and C++17 SystemC
- **Author:** Mayur Kubavat (`Independent Researcher, Mountain View, CA, USA`, ORCID `0009-0009-1087-8558`)
- **Primary Track:** Design Verification, Validation, and Formal Equivalence (Engineering Track / Open-Source EDA)
- **arXiv Categories:** `cs.AR` (Hardware Architecture), `cs.PL` (Programming Languages), `cs.SE` (Software Engineering)

## 1. Industrial Problem Statement
Verification teams and AI hardware coding agents iterate rapidly in C++17 / IEEE 1666-2023 SystemC (sub-second `clang++` rebuilds, zero license contention, native UVM-SystemC + CRAVE + FC4SC libraries), while tape-out sign-off requires IEEE 1800-2023 SystemVerilog RTL, IEEE 1800.2-2020 UVM VIPs, and concurrent SVA monitors. Ad-hoc Python translators and unverified LLM rewrites introduce silent bugs across:
1. TLM value-copy constructors corrupting `crave::crv_variable<T>` solver references and derived constructors popping `crave::crv_object_name` hierarchy stacks prematurely.
2. Context-determined SV bitvector widths vs. C++ `int` promotion (`uint8_t + uint8_t -> int`).
3. Preponed `#1step` clocking vs. SystemC delta-cycle `posedge_event()` races.
4. Overlapping pipelined `|-> ##[m:n]` SVA `assert` and `cover` multiplicity.
5. Superlinear `clang++ -O2` template instantiation on large Boost.Proto `crv_constraint` blocks, `dist` `:/` integer-floor collapse, and CRAVE's 16-bit native `randc` ceiling.

## 2. Key Technical Innovations in `svcc` (`12,600` Rust LOC)
- **7-Dialect `svcc-ir` & Capability Legalizer (`svcc-passes`):** Lowers both SV and C++17 into orthogonal `types`, `uvm`, `crv` (`CrvArena`), `cov` (UCIS 1.0), `sched`, `sva`, and `rtl` dialects normalized by target-capability passes (`WidthSignednessPass`, `FieldOpSynthesisPass`, `OwnershipPass`, `TargetLegalizerPass`, including hard-bound `dist` interval clipping and $u128$ LCM `:/` weight scaling).
- **Multi-Crate `svlang` + `svlang-lint` Stack (`svcc-front-sv` / `svcc-emit-sv`):** Pairs `sv-semantic` (`ElaboratedDesign` widths + `TypeOracle` symbolic constant folding), `svlang-model`, and `svlang-lint` (`formal-ready` emission gate) with `sv-syntax` lossless CST fallback on standalone VIP files without `-f` `uvm_pkg` closure (`12/12` valid vs. `4/12` semantic-only, $3.0\times$).
- **Clause Partitioning & Cycle-Walking Feistel `randc` (`svcc-emit-sc` / `svcc-front-sc`):** Splits large CRAVE constraints (`--partition-clauses 8`) to cut `clang++ -O2` compile latency by **`6.25x`** at $K=120$ clauses (`1,845.4 ms -> 295.1 ms`), and emits header-only 4-round `svcc_randc_feistel<W>` ($1 \le W \le 32$, **`341x` faster** than CRAVE history exclusion at $W=10$).
- **4-Way Solver Translation Validation & `C2RTL` SEC (`svcc-equiv`):**
  1. Z3 `QF_BV` CRV + Enum + `dist` support miter.
  2. $K$-step Z3 BMC proofs for both $O(1)$ `uint64_t pending_window_` `assert property` observers and per-age `pending_age_` `cover property` multiplicity counters (`32 ms` Z3 proof; eliminates $3\times$–$26\times$ burst under-counting at `1.42–3.92 ns/cycle`).
  3. Bijective UCIS 1.0 covergroup/bin/cross isomorphism.
  4. IEEE 1666.1 fail-closed C++17 `// svcc:c2rtl` SSA lowering + `svlang` `Netlist` -> `sv-btor2` -> Z3 SEC across pipeline latency $L \ge 0$ (plus a `--no-default-features` zero-`svlang` Rust SDK slice for unbounded IC3/PDR sign-off in `InFormal`).

## 3. Summary of Verified Empirical Results (`m3_benchmark_summary.json` & `m4_extended_summary.json`)
- **20-Design Corpus (`803` SV LOC + `8` C2RTL Golden Models):** `20/20` pass `clang++ -Werror`, `svlang`, and `svlang-lint`; `272 us` median `sv2sc` and `689 us` median `sc2sv`; `20/20` Z3 round-trip `EquivalentUnsat` (`6.91 ms` median); `8/8` C2RTL `EquivalentUnsat` across $L \in \{0, 1, 2\}$ (`6.14 ms` median).
- **Mutation Sensitivity & Specificity:** `20/20` (`100%`) semantic bugs caught across CRV, SVA, UCIS, and RTL/C2RTL; `4/4` (`100%`) non-trivial AST-divergent equivalent rewrites proved `EquivalentUnsat` (`0%` false alarms).
- **Live Accellera Execution & 12-File External Audit:** Verified against live SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC (`200` `randomize()` draws, `100.0%` FC4SC coverage), $10^5$-sample `dist` sweeps (`TVD = 0.500 -> 0.0043` on int-floor `:/`, and `TVD = 0.0978 -> 0.0016` / **`60.8x` reduction** on hard-bound sliced `weighted_range`), and 12 unmodified external SV files (`1,515` LOC across OpenTitan, PULP, OpenHW, Ibex, InFormal: `9/12` lowered standalone, `2/2` self-contained OpenTitan RTL primitives passing `clang++ -Werror`).
