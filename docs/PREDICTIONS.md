# `uvir` Falsifiable Engineering & Research Predictions Ledger

While building and verifying `uvir` (`IEEE 1800-2023 SV/UVM` <-> `IEEE 1666-2023 SystemC 3.0.3 / UVM-SystemC / CRAVE / FC4SC`), we record concrete, falsifiable predictions uncovered at the boundary of both languages and solvers. Each prediction (`P1`-`P8`) defines an evaluation experiment for upcoming benchmarks and paper sections (`results/predictions.jsonl`).

---

## `P1` — Boost.Proto Expression-Template Compile-Time Knee (`K > 35` Clauses)
- **Root Cause**: CRAVE encodes every clause inside `crave::crv_constraint{this, "c", e1, ..., eK}` into a nested C++17 Boost.Proto expression-template type. While `uvir` transpiles SV -> C++17 in `~230 us`, `clang++ -O2` template instantiation scales superlinearly ($\approx O(K^{1.6}\text{--}K^{2.1})$) in time and RSS.
- **Prediction**: Beyond $K \approx 35\text{--}50$ clauses per `crv_constraint`, C++17 compile wall-clock time exceeds `5 s` per VIP class (dominating short `<10^4`-item AI inner loops), whereas splitting into $B$-clause sub-constraints (`c__part_0`, `c__part_1`, ...) reduces `clang++` compile time by $>3\times$ with zero change to the Z3 `QF_BV` solution set.
- **Planned Evaluation**: Sweep $K \in \{5, 15, 30, 60, 120\}$ clauses on `clang++ -std=c++17 -O2`, comparing monolithic `crv_constraint` vs. `B=8` partitioned emission.

## `P2` — `dist` `:/` Non-Divisible Range Rounding Divergence Despite `QF_BV` Equivalence
- **Root Cause**: Z3 `QF_BV` proves *support-set* equivalence (every value with `weight > 0` is reachable), not probability mass accuracy. Under IEEE 1800-2023 `[lo:hi] :/ W`, when range cardinality $S = (hi - lo + 1) \nmid W$ (or $S > W$), SV simulators and CRAVE `add_weighted_range` apply different integer/float remainder rules.
- **Prediction**: When $S \nmid W$ and $S > W$, empirical Jensen-Shannon / $\chi^2$ divergence over $10^5$ samples will exceed `0.05` unless `svcc-passes` pre-scales all weights in a `dist` block by $\text{LCM}_i(S_i)$ before emitting C++17.
- **Planned Evaluation**: Draw $10^5$ samples on `[0:2] :/ 10` and `[0:15] :/ 5` before and after $\text{LCM}$ weight scaling; measure TVD and $\chi^2$ $p$-value.

## `P3` — Bitmask SVA Observer Exactness on Assertions vs. Multi-Hit Under-Counting on `cover property`
- **Root Cause**: `svcc-emit-sc` lowers `ant |-> ##[min:max] con` to an $O(1)$ `uint64_t pending_window_` shift pipeline, proven 100% failure-equivalent by `prove_sva_bmc_equivalence`. However, clearing all bits in `sat_mask` on `con == 1` satisfies multiple overlapping in-flight `ant` triggers simultaneously (`++pass_count_` increments once instead of once per in-flight attempt).
- **Prediction**: `fail_count_` matches IEEE 1800 `assert property` on 100% of traces, whereas `pass_count_` on `cover property` under back-to-back overlapping `ant` bursts under-counts IEEE 1800 per-attempt non-vacuous cover hits by up to $(max - min)\times$ unless lowered to an array of per-age counters.
- **Planned Evaluation**: Feed synthetic burst traces (`ant = 11100`, `con = 00010` on `##[1:3]`) to both SV simulator and `uvir_sva_*` C++17 observer; compare `fail_count_` and `pass_count_`.

## `P4` — Cooperative `cancel_tok` vs. Preemptive SV `disable fork` Intra-Delta Side Effects
- **Root Cause**: SystemC cannot safely `kill()` a C++17 `sc_spawn` lambda mid-stack without leaking RAII destructors, so `uvir` lowers `fork ... join_any; disable fork;` using cooperative `std::atomic<bool> cancel_tok` checks at `wait()` boundaries.
- **Prediction**: Branches that perform state mutations *after* a `wait()` and *before* the next `wait()` within the same SystemC delta evaluation wave can execute one extra side effect if scheduled in the same delta notification list as the winning branch, unless `svcc-passes` inserts `if (cancel_tok->load()) return;` immediately after **every** `sc_core::wait(...)` inside forked branches.
- **Planned Evaluation**: Construct a 2-branch `join_any` race where both branches wake on the exact same `ns` timestamp (`#100ns` vs `#100ns`) and write to a shared scoreboard counter.

## `P5` — Explicit-Exclusion `randc` Solver Cliff Above $W \ge 11$ Bits
- **Root Cause**: Emulating SV `randc` in CRAVE via an explicit history vector (`x() != prev_0 && ... && x() != prev_m`) appends up to $2^W - 1$ disequalities per cycle.
- **Prediction**: History-exclusion `randc` will exhibit $O(2^{2W})$ cumulative cycle cost and exceed `10 ms/randomize()` for $W \ge 11$ (`2048` states), whereas a constrained Feistel permutation + rejection wrapper keeps median solve latency `<100 us` up to $W = 24$.
- **Planned Evaluation**: Benchmark full-period `2^W` `randomize()` loops in CRAVE for $W \in \{3, 6, 9, 12, 16\}$.

## `P6` — Hybrid Semantic + CST Fallback (`>3x` Higher Partial-VIP Completion Than Pure AST Lowering)
- **Root Cause**: Observed directly in M1 on `axi4_burst_env.sv`: when transpiling standalone SV files without full `-f` package closure (`uvm_pkg`), semantic engines erase unresolved parameterized member types (`uvm_analysis_port #(axi4_item)`) to `<error>`, breaking C++17 codegen (`<error>* item_ap`) until `written_property_type` CST recovery was added.
- **Prediction**: Across standalone files sampled from open-source SV/UVM VIPs (where include/package closure is frequently incomplete during incremental migration or AI generation), hybrid semantic + lossless CST recovery will transpile $>3\times$ more classes cleanly (`clang++ -Werror` passing) than strict semantic-AST-only front-ends.
- **Planned Evaluation**: Run `uvir sv2sc` with CST fallback enabled vs. disabled across 20+ open-source UVM files without `-f` filelists.

## `P7` — 4-State `sc_dt::sc_lv<W>` Runtime Overhead vs. Dual-`uint64_t` `(val, xz_mask)` Legalization
- **Root Cause**: `sc_dt::sc_lv<W>` allocates/manipulates multi-word 4-state logic digits, whereas hardware `logic [W-1:0]` ($W \le 64$) maps to two 64-bit native words (`val`, `xz_mask`).
- **Prediction**: Adding a `four_state_dual_word` legalization mode in `svcc-passes` for $W \le 64$ will speed up `10^6`-transaction SystemC scoreboards and BFM loops by $4\times\text{--}8\times$ over `sc_dt::sc_lv<W>` while preserving exact IEEE 1800 `===` / `!==` / X-propagation semantics.
- **Planned Evaluation**: Microbenchmark `10^6` 64-bit 4-state packet comparisons and masks in `sc_lv<64>` vs. dual-`uint64_t`.

## `P8` — SystemC 3.0.3 `SC_PRE_TIMESTEP` Elimination of Co-Simulation Monitor Sampling Races
- **Root Cause**: Standard SystemC monitors triggered on `clk.posedge_event()` read signals in the evaluate/delta phase rather than IEEE 1800's Preponed (`#1step`) region.
- **Prediction**: Driving `uvir_clocking_*::sample_pre_timestep()` from SystemC 3.0.3's `SC_PRE_TIMESTEP` stage callback eliminates 100% of 0-delay NBA vs. blocking driver-monitor sampling skews in Verilator + UVM-SystemC co-simulation.
- **Planned Evaluation**: Co-simulate an AXI4-Lite RTL slave under intentional 0-delay blocking vs. NBA stimulus with standard `posedge_event()` monitor vs. `SC_PRE_TIMESTEP` clocking sampler.
