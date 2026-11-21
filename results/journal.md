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

