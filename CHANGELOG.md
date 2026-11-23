# Changelog

## Unreleased (0.1.3)

- `rtl-prove` and `prove-sec` exit with code 2 when a check is only bounded (induction did not
  close and the bounded check from reset found no difference), and print `[BND ]` instead of
  `[PASS]`. Before, such a result exited 0. `--accept-bounded` restores the old exit code.
- `rtl-prove` takes a multi-file design with the arguments of `rtl2sc` (more source files,
  `-f`, `+incdir+`, `+define+`, `-G name=value`). The reference netlist is built from the same
  arguments by `svlang`, without svcc code.
- `scripts/rtl_prove.sh` counts a bounded-only design as a failure.

## 0.1.2 (2026-11-23)

Documentation and packaging only; conversion and checks behave as in 0.1.1.

- README rewritten for the RTL path: install, quick start, what each verdict means, known limits.
- `docs/formal_gates.md` and the crate and Python READMEs updated to the 0.1.1 results and the
  clock-gating conversion; `benchmarks/results/rtl_stress.md` marked as the first run.
- `svcc` help text: `rtl-prove` takes one source file and `--header`.
- Package descriptions on crates.io and PyPI describe the RTL path.
- Added `CITATION.cff`, this changelog and `paper/README.md`; the SoftwareX and preprint drafts
  are in `paper/`. The first technical report moved to `paper/early-draft/` and its build
  artifacts and arXiv tarball were removed (its RTL results came from a gate later found unsound).

## 0.1.1 (2026-11-23, GitHub release and Zenodo archive only)

- Clock-gating cells, clock buffers and inverters become clock enables on the root clock,
  including gated clocks kept as bits of a packed vector.
- `rtl-cosim` builds the Verilator reference from the same text and wraps a self-instantiating top.
- The crate version in `Cargo.toml` was not bumped for this release, so `svcc --version`
  printed 0.1.0.

## 0.1.0 (2026-11-22)

First release on crates.io and PyPI: typed RTL conversion (`rtl2sc`), the proof of the
generated C++ against the source (`rtl-prove`), C2RTL proofs (`prove-sec`) and co-simulation
against Verilator (`rtl-cosim`), on svlang 0.5.1.
