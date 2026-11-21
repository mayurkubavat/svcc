# `uvir` — Universal Verification Intermediate Representation & Multi-Language Compiler

`uvir` is a high-performance Rust compiler, target legalizer, and formal equivalence verifier bridging modern hardware verification ecosystems through a single **hub-and-spoke Verification IR (`UVIR`)**:

- **Active Spoke 1 (`sv-uvm`)**: IEEE 1800-2023 SystemVerilog + IEEE 1800.2-2020 UVM (powered by pinned [`svlang`](https://github.com/mayurkubavat/svlang) `478a0d10`).
- **Active Spoke 2 (`sc-uvm`)**: IEEE 1666-2023 SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC (C++17).
- **Planned Spokes (`O(N)` Extension Architecture)**: Python (`cocotb 2.0` + `pyuvm` + `PyVSC`), Accellera Portable Stimulus (`IEEE 1800.1 PSS`), Rust/Verilator/Arcilator native harnesses (`rust-dv`), VHDL-2019 (`OSVVM`/`UVVM`), and MLIR/CIRCT (`moore`/`verif`/`smt`).

## Workspace Layout

```text
uvir/
├── Cargo.toml                           # Pinned svlang rev + LTO release profiles
├── .cargo/config.toml.worktree-example  # Local [patch] for ../svlang-uvir co-development
├── AGENTS.md                            # Hard repo invariants (zero GH Actions, mandatory result logs)
├── STATUS.md                            # Milestone status & verified capability matrix
├── crates/
│   ├── uvir-ir/        # 6-dialect arena IR, TargetCapabilities, LanguageRegistry, extension traits
│   ├── uvir-front-sv/  # IEEE 1800-2023 SV/UVM frontend over svlang + svlang-model + RAL
│   ├── uvir-front-sc/  # C++17 SystemC/UVM + CRAVE + FC4SC frontend (+ optional libclang hook)
│   ├── uvir-passes/    # Ownership, width/signedness, field-op, & TargetLegalizerPass
│   ├── uvir-equiv/     # Language-agnostic Z3 QF_BV SMT-LIB2 miter & UCIS 1.0 isomorphism checker
│   ├── uvir-emit-sc/   # Idiomatic C++17 SystemC/UVM + CRAVE + FC4SC emitter
│   ├── uvir-emit-sv/   # Lint-clean IEEE 1800-2023 SV/UVM emitter with svlang self-verification
│   └── uvir-cli/       # `uvir` CLI (`targets`, `transpile`, `sv2sc`, `sc2sv`, `roundtrip-check`, `prove-equiv`, `bench`)
├── docs/
│   ├── ARCHITECTURE.md       # 3-Tier generic IR, TargetCapabilities legalizer, & 7 SV<->SC rules
│   ├── EXTENSIONS.md         # Adding new language spokes (Python/cocotb, PSS, Rust, VHDL), dialects, MCP, WASM
│   ├── SVLANG_WORKTREE.md    # Isolated ../svlang-uvir git worktree workflow
│   └── PAPER_EXPERIMENTS.md  # DAC / ICCAD / DVCon / TCAD benchmark & evaluation protocol
├── results/
│   ├── README.md             # Schema for reproducible experiment tracking
│   ├── experiment_log.jsonl  # Machine-readable milestone & benchmark telemetry
│   └── journal.md            # Chronological engineering lab notebook
├── scripts/
│   ├── check.sh              # Full fmt + clippy (-D warnings) + test + Z3 round-trip gate
│   └── record_result.py      # Appends verified runs to results/{experiment_log.jsonl,journal.md}
└── testdata/
    ├── sv/apb_timer_item.sv  # IEEE 1800-2023 sequence item, tolerance inside, & covergroup fixture
    └── sc/apb_timer_item.hpp # SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC counterpart fixture
```

## Quick Start

```bash
# Run formatting, strict clippy, unit/integration tests, and Z3 round-trip check
./scripts/check.sh

# Inspect registered & planned multi-language capability profiles
cargo run --release -p uvir-cli -- targets

# Bidirectional round-trip + svlang parse verification + Z3 QF_BV equivalence proof
cargo run --release -p uvir-cli -- roundtrip-check testdata/sv/apb_timer_item.sv

# Latency & solver telemetry benchmark
cargo run --release -p uvir-cli -- bench testdata/sv/apb_timer_item.sv --iters 100
```
