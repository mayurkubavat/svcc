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

