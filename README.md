# `uvir` — Bidirectional IEEE 1800-2023 SV/UVM <-> C++17 SystemC/UVM Compiler & Verifier

`uvir` is a high-performance Rust compiler, capability legalizer, and Z3 formal equivalence prover translating bidirectionally between:

- **`sv-uvm`**: IEEE 1800-2023 SystemVerilog + IEEE 1800.2-2020 UVM (powered by pinned [`svlang`](https://github.com/mayurkubavat/svlang) `478a0d10`).
- **`sc-uvm`**: IEEE 1666-2023 SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC (C++17, verified via `clang++ -std=c++17 -Wall -Wextra -Werror`).

## Verified Capabilities (M1)
- **`types` & `uvm`**: 2-state/4-state bitvectors, signedness, `typedef enum` <-> `enum class`, `uvm_sequence_item` (synthesizing TLM-safe value-only copy constructors for CRAVE), `UVM_NOCOMPARE`/`UVM_NOCOPY` field policies, `uvm_reg` + `uvm_reg_block` address maps, and UVM component hierarchies (`build_phase` `type_id::create` + `connect_phase` TLM ports).
- **`crv` & `cov`**: Hard & `soft` constraints, implication, `unique`, IEEE 1800-2023 `inside` tolerance ranges (`[c +/- d]`, `[c +%- p]`), weighted distributions (`dist { := , :/ }` <-> `crave::dist`), `randc`, and `covergroup`/`coverpoint`/`cross` <-> `fc4sc::covergroup` (with Z3 `QF_BV` constraint miters + UCIS 1.0 isomorphism checks).
- **`sched` & `sva`**: `fork`/`join_any`/`disable fork` <-> SystemC `sc_spawn` + cooperative `std::atomic<bool>` cancel tokens, `clocking #1step` <-> `SC_PRE_TIMESTEP` sampler modules, and clocked `assert`/`cover property` (`|-> ##[m:n]`, `disable iff`) <-> $O(1)$ 64-bit shift-window C++17 observers proven trace-equivalent via Z3 Bounded Model Checking (BMC).

## Quick Start

```bash
# Full gate: rustfmt + clippy (-D warnings) + tests + clang++ -Werror + svlang + Z3 QF_BV & SVA BMC
./scripts/check.sh

# Round-trip check any SV/UVM file through C++17 (clang++ -Werror) and back to SV (svlang) + Z3 proofs
cargo run --release -p uvir-cli -- roundtrip-check testdata/sv/axi4_burst_env.sv --shims testdata/cpp_shims
cargo run --release -p uvir-cli -- roundtrip-check testdata/sv/uart_ral_sva.sv --shims testdata/cpp_shims

# Release latency & Z3 equivalence benchmark
cargo run --release -p uvir-cli -- bench testdata/sv/axi4_burst_env.sv --iters 200
```

## Key Documentation
- `docs/ARCHITECTURE.md`: 6-dialect `UVIR`, capability legalizer, and the 7 SV <-> C++ semantic impedance rules.
- `docs/PREDICTIONS.md` & `results/predictions.jsonl`: Falsifiable architectural & performance predictions (`P1`–`P8`) queued for experimental evaluation.
- `docs/SVLANG_WORKTREE.md`: Isolated `../svlang-uvir` (`feature/uvir-hooks`) co-development workflow.
- `results/journal.md` & `results/experiment_log.jsonl`: Measured milestone benchmarks and Z3 equivalence logs.
