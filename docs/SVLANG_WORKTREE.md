# Isolated `svlang` Worktree Workflow (`svlang-uvir`)

`uvir` pins [`svlang`](https://github.com/mayurkubavat/svlang) to an exact commit SHA in `Cargo.toml` (`478a0d10a4e91ea8199d3179ee0a8fc111d1e692`) for deterministic builds and reproducible paper benchmarks.

## Directory Layout on Developer Machine

```text
/Users/mkubavat/Silicon/git/
├── svlang/       # Main svlang checkout (main branch) — NEVER touch during uvir experiments
├── svlang-uvir/  # Dedicated git worktree on branch `feature/uvir-hooks`
└── uvir/         # This repository (main branch)
```
Convenience symlinks mirror both under `/Users/mkubavat/Silicon/active/{svlang-uvir,uvir}`.

## Co-Developing `svlang` Enhancements Safely

1. Make front-end changes strictly inside `/Users/mkubavat/Silicon/git/svlang-uvir` (`feature/uvir-hooks`).
2. Copy `.cargo/config.toml.worktree-example` to `.cargo/config.toml` (gitignored) inside `uvir/`:
   ```bash
   cp .cargo/config.toml.worktree-example .cargo/config.toml
   ```
   Cargo's `[patch."https://github.com/mayurkubavat/svlang.git"]` transparently redirects `svlang`, `svlang-model`, and `svlang-lint` to `../svlang-uvir/crates/*`.
3. Run `cargo test --workspace` in both `../svlang-uvir` and `uvir`.
4. Once upstreamed to `mayurkubavat/svlang`, update the pinned `rev = "<new_sha>"` in `uvir/Cargo.toml`, remove `.cargo/config.toml`, and log the result via `scripts/record_result.py`.

## Planned Upstream Hooks for `feature/uvir-hooks`
- **Typed Constraint AST in `sv-semantic::model::Constraint`**: Expose bound `ExprArena` / `InsideItem` IDs directly on `svlang_model::Constraint` so `uvir-front-sv` can consume type-resolved parameter/enum constants without CST re-tokenization.
- **Semantic `Covergroup` Model in `svlang-model`**: Lift `SyntaxKind::CovergroupDecl`, `Coverpoint`, `Cross`, and `CoverageOption` into `svlang_model::Class` & `Design`.
- **Clocking Block (`SyntaxKind::ClockingDecl`) Skew Extractor**: Expose `input #1step` / `output #0` skews in `svlang-model` for `SC_PRE_TIMESTEP` lowering.
