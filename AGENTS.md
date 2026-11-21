# Repository Instructions for AI Agents (`uvir`)

## Core Mission
`uvir` is a high-performance, Rust-based bidirectional compiler and semantic equivalence verifier between:
- **SystemVerilog/UVM**: IEEE 1800-2023 + IEEE 1800.2-2020
- **SystemC/UVM**: IEEE 1666-2023 (`SystemC 3.0.3`) + `UVM-SystemC` + `CRAVE` + `FC4SC`

## Hard Invariants (Mandatory)
1. **No GitHub Actions**: NEVER create, enable, or trigger `.github/workflows/` in this repository or in `svlang-uvir`. All verification is executed locally via `./scripts/check.sh`.
2. **Pinned `svlang` + Separate Worktree**:
   - Keep `[workspace.dependencies]` in `Cargo.toml` pinned to an explicit commit SHA of `https://github.com/mayurkubavat/svlang.git`.
   - When adding upstream front-end hooks to `svlang`, NEVER modify `/Users/mkubavat/Silicon/git/svlang` directly. Work exclusively inside the isolated worktree `/Users/mkubavat/Silicon/git/svlang-uvir` (`feature/uvir-hooks`), test locally via `.cargo/config.toml` (`[patch]`), push/merge when verified, and bump the pinned `rev` in `Cargo.toml`.
3. **Mandatory Experiment & Result Logging**:
   - Every milestone, pass optimization, solver equivalence test, or benchmark run MUST be recorded in `results/experiment_log.jsonl` and summarized in `results/journal.md` (via `python3 scripts/record_result.py ...`).
   - Report only measured commands and outputs; compiling alone is not verification.
4. **Python Scripts**:
   - Never write `import antigravity` in any script (use `google.antigravity` if the Antigravity SDK is ever needed).
5. **Commits**:
   - Sign off commits (`git commit -s`) after `./scripts/check.sh` passes cleanly with zero warnings.
