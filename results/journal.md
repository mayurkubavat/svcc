# `uvir` Engineering & Paper Result Journal

Chronological log of verified compiler builds, Z3 equivalence proofs, and latency benchmarks.

## [2026-11-21T21:21:00+00:00] `M0-hub-and-spoke-baseline` — 8-crate multi-language UVIR workspace, TargetCapabilities legalizer, Z3 QF_BV equivalence, and APB4 round-trip
- **Commit**: `uncommitted` (`Darwin-arm64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)`)
- **Command**: `./scripts/check.sh && cargo run --release -p uvir-cli -- bench testdata/sv/apb_timer_item.sv --iters 200`
- **Measured Metrics**:
  - `avg_sc2sv_us`: `228`
  - `avg_sv2sc_us`: `228`
  - `clippy_warnings`: `0`
  - `coverage_isomorphic`: `True`
  - `crates`: `8`
  - `crv_arena_nodes`: `34`
  - `pinned_svlang_rev`: `478a0d10a4e91ea8199d3179ee0a8fc111d1e692`
  - `smtlib2_bytes`: `1252`
  - `workspace_tests_passed`: `6`
  - `z3_check_us`: `8086`
  - `z3_verdict`: `EquivalentUnsat`
- **Notes**: Verified SV(1800-2023) <-> SC(1666-2023+CRAVE+FC4SC) round-trip including tolerance inside expansion, value-only CRAVE copy-ctor synthesis, UVM_NOCOMPARE preservation, RAL uvm_reg -> uvm_randomized_reg lowering, and off-by-one Z3 SAT bug detection. Isolated svlang worktree branch feature/uvir-hooks @ 9cf0d14 tested (76/76 passing).

## [2026-11-21T21:45:18+00:00] `M1-sv-cpp-full-dialect-parity` — Complete SV <-> C++17 parity (enum, dist, randc, fork/disable fork, clocking #1step, RAL block maps, SVA bitmask observer + Z3 BMC, clang++ -Werror)
- **Commit**: `71787ea` (`Darwin-arm64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)`)
- **Command**: `./scripts/check.sh && ./target/release/uvir bench testdata/sv/{apb_timer_item,axi4_burst_env,uart_ral_sva}.sv --iters 200`
- **Measured Metrics**:
  - `apb4_sc2sv_us`: `301`
  - `apb4_sv2sc_us`: `214`
  - `axi4_sc2sv_us`: `376`
  - `axi4_sv2sc_us`: `339`
  - `clang_werror_verified_suites`: `3`
  - `predictions_logged`: `8`
  - `svlang_verified_suites`: `3`
  - `uart_ral_sva_sc2sv_us`: `235`
  - `uart_ral_sva_sv2sc_us`: `280`
  - `z3_crv_unsat_classes`: `7`
  - `z3_sva_bmc_unsat_properties`: `2`
- **Notes**: Scoped strictly to SV (1800-2023/1800.2-2020) <-> C++17 (SystemC 3.0.3/UVM-SystemC/CRAVE/FC4SC). Every emitted C++17 header passes Apple clang++ 21 -std=c++17 -Wall -Wextra -Werror -fsyntax-only; every emitted SV file passes svlang; all CRV/Enum/Dist and pipelined SVA ##[m:n] observers proven EquivalentUnsat in Z3.

## [2026-11-21T22:17:44+00:00] `M1.1-svcc-rename-and-sdk-packaging` — Renamed repo/CLI/SDK to svcc; verified crates.io, PyPI, and npm availability + maturin/PyO3 & umbrella SDK
- **Commit**: `59913d9` (`Darwin-arm64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)`)
- **Command**: `./scripts/check.sh`
- **Measured Metrics**:
  - `clippy_warnings`: `0`
  - `crates_io_svcc_available`: `True`
  - `npm_svcc_available`: `True`
  - `pypi_svcc_available`: `True`
  - `workspace_tests_passed`: `7`
- **Notes**: Renamed GitHub repository to mayurkubavat/svcc (PRIVATE), moved workspace crates to svcc-*, exposed umbrella Rust SDK crate svcc (lib.rs + bin svcc), and added bindings/python/pyproject.toml + docs/PACKAGING.md.

## [2026-11-21T22:36:38+00:00] `M2-rtl-c2rtl-btor2-sec-and-pyo3` — Bidirectional RTL (module <-> SC_MODULE), Untimed C++17 SSA C2RTL, Btor2+Z3 Sequential Equivalence (latency=L), and PyO3 bindings
- **Commit**: `ba2f07e` (`Darwin-arm64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)`)
- **Command**: `./scripts/check.sh && ./target/release/svcc bench testdata/sv/alu_pipe_rtl.sv --iters 200 && ./target/release/svcc prove-sec testdata/sc/alu_golden_c2rtl.hpp testdata/sv/alu_pipe_rtl.sv --latency 2`
- **Measured Metrics**:
  - `c2rtl_latency2_btor2_lines`: `80`
  - `c2rtl_latency2_smt_bytes`: `30235`
  - `c2rtl_latency2_z3_us`: `7188`
  - `c2rtl_verdict`: `EquivalentUnsat`
  - `clippy_warnings`: `0`
  - `pyo3_binding_compiled`: `True`
  - `rtl_sc2sv_us`: `179`
  - `rtl_sec_btor2_lines`: `89`
  - `rtl_sec_z3_us`: `6539`
  - `rtl_sv2sc_us`: `213`
  - `workspace_tests_passed`: `10`
- **Notes**: Implemented 7th dialect (rtl), synthesizable SV module <-> SystemC SC_MODULE with auto-inferred sensitive<< lists and IEEE 1666.1 fail-closed heap/container guards, untimed C++17 SSA + bounded-loop unroller (// svcc:c2rtl), svlang Netlist -> Btor2 -> Z3 QF_BV SEC prover, and PyO3 Python bindings.

## [2026-11-21T23:08:43+00:00] `M3-benchmarks-and-arxiv-manuscript` — Zero-svlang SDK decoupling, 20-design M3 benchmark + ablation suite, and 7-page IEEEtran arXiv/DAC manuscript
- **Commit**: `77974fa` (`Darwin-arm64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)`)
- **Command**: `./scripts/check.sh && python3 benchmarks/run_benchmarks.py && make -C paper check arxiv-tar`
- **Measured Metrics**:
  - `bib_entries`: `27`
  - `c2rtl_sec_unsat`: `8/8`
  - `corpus_designs`: `20`
  - `corpus_roundtrip_unsat`: `20/20`
  - `equiv_rewrites_unsat`: `4/4`
  - `latex_warnings`: `0`
  - `median_c2rtl_z3_ms`: `5.9`
  - `median_sc2sv_us`: `210`
  - `median_sv2sc_us`: `200`
  - `median_z3_roundtrip_ms`: `6.82`
  - `overfull_hboxes`: `0`
  - `p1_k120_clang_o2_speedup_x`: `5.95`
  - `p6_hybrid_vs_strict`: `12/12 vs 0/12`
  - `paper_pages`: `7`
  - `rust_loc`: `10899`
  - `semantic_mutations_caught`: `20/20`
- **Notes**: Confirmed Predictions P1 (5.95x clang++ -O2 speedup at K=120 via --partition-clauses 8) and P6 (12/12 vs 0/12 standalone VIP recovery); compiled paper/main.pdf (7 pages, 27 cited references, 0 warnings, 0 overfull/badness-10000 boxes, 0 shared prose 7-grams).

## [2026-11-22T00:20:17+00:00] `M4-svlang-lint-live-crave-and-external-corpus` — 5-crate svlang+svlang-lint integration, P2/P3/P5 live CRAVE+Z3 passes, 12-file external audit, and camera-ready 8-page paper
- **Commit**: `ff0a4e2` (`Darwin-arm64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)`)
- **Command**: `./scripts/check.sh && python3 benchmarks/run_m4_extended.py && make -C paper clean check arxiv-tar`
- **Measured Metrics**:
  - `arxiv_abstract_bytes`: `1898`
  - `live_uvm_fc4sc_cov_pct`: `100.0`
  - `p2_lcm_tvd`: `0.0043`
  - `p2_sliced_clamp_tvd`: `0.0978`
  - `p3_cover_z3_ms`: `32.0`
  - `p3_per_age_ns_per_cyc`: `3.88`
  - `p5_feistel_pure_ns`: `4.7`
  - `p5_w10_speedup_x`: `263.1`
  - `paper_pages`: `8`
  - `rq5_external_lowered`: `9/12`
  - `rq5_opentitan_clang_pass`: `2/2`
  - `workspace_rust_loc`: `12305`
- **Notes**: Verified against mayurkubavat development forks of SystemC 3.0.3, UVM-SystemC, CRAVE, and FC4SC; ORCID vector badge + realigned TikZ Fig. 1 + balanced two-column refs.

## [2026-11-22T00:55:19+00:00] `M4.1-svcc-ir-unification-and-sliced-dist-fix` — Unify UVIR -> svcc-ir/svcc_* across workspace and add hard-bound dist interval clipping + rational LCM scaling
- **Commit**: `cfc730d` (`Darwin-arm64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)`)
- **Command**: `./scripts/check.sh && python3 benchmarks/run_benchmarks.py && python3 benchmarks/run_m4_extended.py && make -C paper clean check arxiv-tar`
- **Measured Metrics**:
  - `arxiv_abstract_bytes`: `1919`
  - `p1_k120_speedup_x`: `6.25`
  - `p2_sliced_tvd_after`: `0.0016`
  - `p2_sliced_tvd_before`: `0.0978`
  - `p2_sliced_tvd_reduction_x`: `60.8`
  - `p5_w10_speedup_x`: `341.3`
  - `p6_cst_recovery`: `12/12 vs 4/12`
  - `workspace_rust_loc`: `12600`
- **Notes**: Zero uvir identifiers remaining; all 5 evaluated predictions (P1, P2, P3, P5, P6) confirmed; citation 10 removed.

## [2026-11-22T19:01:10+00:00] `M5-scav-head-to-head` — SCAV vs svcc RTL head-to-head (Verilog -> SystemC, real SystemC 2.3.4 co-simulation)
- **Commit**: `c59da4a` (`Linux-x86_64`, `rustc 1.97.0 (2d8144b78 2026-07-07)`)
- **Command**: `cargo build --release -p svcc && python3 benchmarks/scav_compare/run_compare.py`
- **Measured Metrics**:
  - `corpus_rtl_scav_compiles`: `0/8`
  - `corpus_rtl_svcc_cosim_match`: `8/8`
  - `probes_scav_compiles`: `2/18`
  - `probes_scav_cosim_match`: `0/18`
  - `probes_scav_translator_crash`: `9/18`
  - `probes_svcc_cosim_match`: `3/18`
  - `probes_svcc_roundtrip_pass`: `5/18`
  - `probes_svcc_roundtrip_pass_but_cosim_mismatch`: `2`
  - `scav_commit`: `2ef2e6b`
  - `scav_own_examples_compile`: `3/9`
  - `scav_own_examples_cosim_match`: `2/9`
- **Notes**: Cloud Linux container, g++ + libsystemc-dev 2.3.4, Icarus Verilog. svcc roundtrip-check reports EquivalentUnsat on p16_width and p17_wrap while the emitted SystemC mismatches RTL in simulation (SV context-determined width not modelled in C++). svcc RTL output needed stand-in uvm/crave2/fc4sc headers to compile against stock SystemC.
## [2026-11-22T19:08:38+00:00] `M5-typed-rtl-hdl` — Typed, elaborated SV RTL -> SystemC path with Verilator differential simulation
- **Commit**: `8da6eaf` (`Linux-x86_64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13)`)
- **Command**: `SYSTEMC_HOME=$HOME/.local/opt/accellera-svcc/systemc ./scripts/check.sh`
- **Measured Metrics**:
  - `baseline_legacy_corpus_compiles`: `3/16`
  - `benchmark_rtl_13_20_cosim_pass`: `8/8`
  - `corpus_cosim_pass`: `16/16`
  - `corpus_cosim_steps_each`: `20000`
  - `corpus_output_comparisons`: `3830212`
  - `host`: `Linux x86_64`
  - `scav_probes_cosim_pass`: `18/18`
  - `systemc`: `mayurkubavat/systemc development 9767bff (3.0.x, C++17)`
  - `verilator`: `5.052`
- **Notes**: New rtl2sc and rtl-cosim commands; sv2sc routes RTL modules through the typed path. Legacy text-based RTL path kept only for roundtrip-check and C2RTL SEC. Evidence is random-stimulus simulation, not proof. SCAV probes and benchmark designs were run once by hand and are not part of check.sh.

## [2026-11-22T19:21:21+00:00] `M5.1-typed-rtl-macos-dev-stack` — Typed RTL path re-checked on macOS arm64 against the SystemC development stack
- **Commit**: `96b5f30` (`Darwin-arm64`, `rustc 1.100.0 (0ef1bc386 2026-11-09) (Homebrew)`)
- **Command**: `SYSTEMC_HOME=$HOME/.local/opt/accellera-svcc/systemc ./scripts/check.sh`
- **Measured Metrics**:
  - `benchmark_rtl_13_20_cosim_pass`: `8/8`
  - `cargo_tests_failed`: `0`
  - `cargo_tests_passed`: `18`
  - `check_sh`: `ALL CHECKS PASSED (exit 0)`
  - `corpus_cosim_pass`: `16/16`
  - `corpus_cosim_steps_each`: `20000`
  - `corpus_output_comparisons`: `3893207`
  - `linux_per_design_counts_identical`: `True`
  - `scav_probes_cosim_pass`: `18/18`
  - `systemc`: `oss-workbench systemc development 9767bff (3.0.3, C++17), libsystemc.dylib`
  - `uvm_systemc`: `development 8faac2c (installed, not used by these checks)`
  - `verilator`: `5.052`
- **Notes**: rtl-cosim only looked for libsystemc.so/.a, so all 16 designs failed on macOS before any model was built; it now also accepts libsystemc.dylib. Per-design comparison counts match a Linux rerun exactly (sum 3893207); the 3830212 total in the M5-typed-rtl-hdl entry is stale. SCAV probes and designs 13..20 were run by hand with svcc rtl-cosim --steps 20000 against Verilator, not through run_compare.py (Icarus), and are not part of check.sh. Evidence is random-stimulus simulation, not proof.

## [2026-11-22T20:19:29+00:00] `M6-typed-formal-gates` — Formal gates moved to the typed RTL path; text-based RTL path removed
- **Commit**: `3c486e4` (`Darwin-arm64`, `rustc 1.100.0 (0ef1bc386 2026-11-09) (Homebrew)`)
- **Command**: `SYSTEMC_HOME=$HOME/.local/opt/accellera-svcc/systemc ./scripts/check.sh; scripts/rtl_prove.sh; python3 benchmarks/formal_gate/mutate.py --per-design 5 --steps 5000 --ops unwrap <42 designs>; python3 benchmarks/formal_gate/mutate.py --per-design 6 --steps 5000 --ops widen,resign,uncast <42 designs>; svcc rtl-cosim --steps 20000 on 18 probes and designs 13..20`
- **Measured Metrics**:
  - `c2rtl_prove_sec`: `8/8 ProvedInductive (from an arbitrary state, resets held inactive)`
  - `cargo_tests_failed`: `0`
  - `cargo_tests_passed`: `26`
  - `check_sh`: `ALL CHECKS PASSED (exit 0), 8 steps`
  - `cosim_corpus`: `16/16`
  - `cosim_probes_and_13_20`: `26/26 at 20000 steps`
  - `golden_model_defect_found`: `14_mac16_pipe2_golden: uint16_t*uint16_t overflows int (undefined behaviour); fixed to uint32_t(a)*b`
  - `historical_width_bugs_caught`: `2/2 with counterexample (p16: a=b=0xff SV 0x7f C++ 0xff)`
  - `host`: `macOS arm64`
  - `lowering_stage`: `42/42 by induction`
  - `mutants_compiled`: `281`
  - `mutants_differ_and_proof_accepts`: `0`
  - `mutants_differ_and_proof_finds_difference`: `14`
  - `mutants_differ_and_reader_rejects`: `3`
  - `mutants_differ_in_cosim`: `17`
  - `mutants_not_compiling_dropped`: `70`
  - `mutants_same_in_cosim_proof_accepts`: `255`
  - `mutants_same_in_cosim_reader_rejects`: `9`
  - `negative_tests`: `6 defects must fail the gate, all fail`
  - `rtl_prove_designs`: `42/42 (16 corpus, 18 probes, 8 designs 13..20)`
  - `rtl_prove_method`: `42 by induction over paired state, 0 bounded only`
  - `systemc`: `development 9767bff (3.0.3) in ~/.local/opt/accellera-svcc`
  - `verilator`: `5.052`
- **Notes**: What a pass means and what it assumes is in docs/formal_gates.md: two-state reference (X reads 0, division by zero yields 0), svlang netlist as the meaning of SV on both sides, one clock with a synchronous view of asynchronous reset, svcc's own model of C++ and SystemC integer semantics (checked against SystemC 3.0.3 with a probe and by the mutation check, not a compiler front end). Only 17 of 281 mutants change behaviour, so the mutation check is a small sample. The UVM-side gates were not re-examined. Paper text and benchmarks/results/m3_benchmark_summary.json still describe the removed gate and were not regenerated. The earlier roundtrip-check verdicts for RTL in this log (EquivalentUnsat from the textual miter) did not examine the generated C++ and should not be cited as proofs of it.
## [2026-11-22T21:25:32+00:00] `M5.1-typed-rtl-real-designs` — Typed RTL path stressed on 15 open-source designs (2,184 modules) against Verilator
- **Commit**: `7c85167` (`Linux-x86_64`, `unknown`)
- **Command**: `SYSTEMC_HOME=$HOME/.local/opt/accellera-svcc/systemc ./scripts/check.sh && python3 benchmarks/rtl_stress/run_stress.py --census-tool <svlang>/tools/quality --corpus <rtl-census> --stage cosim --steps 3000`
- **Measured Metrics**:
  - `converted`: `1191`
  - `cosim_match`: `1170`
  - `cosim_mismatch`: `14`
  - `cosim_mismatch_unexplained`: `4`
  - `cosim_not_built`: `7`
  - `cpp_compiles`: `1191`
  - `memout_or_timeout`: `13`
  - `modules`: `2184`
  - `repo_corpus_cosim`: `20/20`
  - `svcc_rejects`: `763`
  - `svcc_silent_bugs_fixed`: `3`
  - `svlang`: `478a0d1`
  - `svlang_defects_found`: `3`
  - `svlang_rejects`: `217`
  - `systemc`: `development 9767bff`
  - `verilator`: `5.052`
- **Notes**: See benchmarks/results/rtl_stress.md. Caliptra was re-run after the last fix and merged into the result. Remaining mismatches: 3 svlang parameter fill literal, 4 derived or generated clocks, 2 reference-side, 1 out-of-range under placeholder parameters, 4 unexplained (cvfpu).

## [2026-11-22T21:31:41+00:00] `M6-typed-formal-gates` — Stress-run forms under the formal gate (branch rtl/stress-prove, not on main)
- **Commit**: `b697994` (`Darwin-arm64`, `rustc 1.100.0 (0ef1bc386 2026-11-09) (Homebrew)`)
- **Command**: `SYSTEMC_HOME=$HOME/.local/opt/accellera-svcc/systemc ./scripts/check.sh`
- **Measured Metrics**:
  - `cargo_tests_failed`: `0`
  - `cargo_tests_passed`: `33`
  - `check_sh`: `ALL CHECKS PASSED (exit 0)`
  - `cosim_testdata_rtl`: `20/20`
  - `forms_not_covered`: `lambda for replication of more than 16 copies into vectors over 64 bits, multi-dimensional array ports, multi-clock designs`
  - `forms_proved`: `1-D array ports, posedge_event edges (gated clock), per-bit dynamic range writes, shift-based out-of-range reads, underscore-suffixed module names`
  - `host`: `macOS arm64`
  - `rtl_prove_failed`: `0`
  - `rtl_prove_not_covered`: `3/46: array_elems and edges_misc (more than one clock), array_ports (multi-dimensional array port)`
  - `rtl_prove_proved`: `43/46, all by induction`
- **Notes**: The stress corpus (outside the repository, multi-file) was not put to the proof; its evidence is the simulation in benchmarks/results/rtl_stress.md. Rebase conflicts: cosim.rs, lib.rs, main.rs, docs/rtl_subset.md (kept both sides), result logs (both entries kept).

## [2026-11-22T23:26:12+00:00] `R0.1-release` — svcc 0.1.0 release candidate on svlang 0.5.1 with derived-clock fail-closed check
- **Commit**: `9cce2b3` (`Darwin-arm64`, `rustc 1.100.0 (0ef1bc386 2026-11-09) (Homebrew)`)
- **Command**: `SYSTEMC_HOME=~/.local/opt/accellera-svcc/systemc ./scripts/check.sh; run_stress.py --stage cosim --steps 3000 (linux, 16 designs)`
- **Measured Metrics**:
  - `check_sh`: `pass`
  - `lfsr_with_svlang_0_5_1`: `3/3 match`
  - `repo_cosim`: `20/20`
  - `rtl_prove_not_covered`: `3`
  - `rtl_prove_proved`: `43`
  - `stress_match`: `1128`
  - `stress_mismatch_svlang_0_4`: `10`
  - `stress_modules`: `2184`
  - `stress_rejected_by_clock_check`: `68`
- **Notes**: Clock check rejects gated/derived/blocking-assigned/expression clocks. Remaining mismatches with svlang 0.5.1: popcount, prim_pad_attr (reference), stream_mux (placeholder parameter), fpnew_cast_multi and fpnew_opgroup_block x2 (unexplained).

## [2026-11-23T02:34:36+00:00] `M5.2-clock-gating` — Clock-gating cells, clock buffers and inverters, and register-derived clocks in the typed RTL path
- **Commit**: `16a53ca` (`Linux-x86_64`, `unknown`)
- **Command**: `SYSTEMC_HOME=$HOME/.local/opt/accellera-svcc/systemc ./scripts/check.sh && python3 benchmarks/rtl_stress/run_stress.py --census-tool <svlang>/tools/quality --corpus <rtl-census> --stage cosim --steps 3000`
- **Measured Metrics**:
  - `clock_check_rejections_after`: `13`
  - `clock_check_rejections_before`: `65`
  - `cosim_match_after`: `1185`
  - `cosim_match_before`: `1127`
  - `newly_matching`: `caliptra-rtl 32, cv32e40p 11, serv 5, opentitan 5, common_cells 2, cv32e40x 2, cvfpu 1, ibex 1`
  - `regressions`: `0`
  - `repo_corpus_cosim`: `23/23`
  - `rtl_prove`: `45 proved, 4 not covered`
  - `stress_modules`: `2184`
  - `svlang`: `0.5.1 (crates.io)`
  - `verilator`: `5.052`
- **Notes**: Remaining clock-check rejections are clock muxes, dividers whose output depends on an input clock, clocks from inputs through logic, VeeR BHT bank clocks (bits of a packed vector), and a testbench. Remaining mismatches: stream_mux (Verilator wraps out-of-range writes), fpnew_cast_multi x2 (fixed by unreleased svlang af574c9), fpnew_fma_multi x2 (differs in the special-result path even with af574c9; not root-caused).

## [2026-11-23T05:16:59+00:00] `M6.1-rtl-prove-main` — rtl-prove over the repository designs at main
- **Commit**: `00c80f0` (`Linux-x86_64`, `rustc 1.97.0 (2d8144b78 2026-07-07)`)
- **Command**: `cargo build --release -p svcc && ./scripts/rtl_prove.sh (release binary, z3 from pip)`
- **Measured Metrics**:
  - `bounded_only`: `0`
  - `designs`: `50`
  - `failed`: `0`
  - `not_covered`: `5`
  - `not_covered_designs`: `array_elems array_ports clock_gating clock_vector edges_misc`
  - `proved_inductive`: `45`
- **Notes**: Run in a Linux x86_64 cloud container for the paper drafts (paper/rtl-tv, paper/softwarex). No SystemC or Verilator there, so no co-simulation in this run.

