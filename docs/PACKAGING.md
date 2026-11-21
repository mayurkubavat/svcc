# Multi-Ecosystem Packaging Plan for `svcc`

Registry availability verified on `2026-11-21`:
- **GitHub**: `https://github.com/mayurkubavat/svcc` (created & active, `PRIVATE` during development)
- **Rust (`crates.io`)**: `svcc` and `svcc-ir` — **AVAILABLE** (`HTTP 404`)
- **Python (`PyPI`)**: `svcc` — **AVAILABLE** (`HTTP 404`)
- **JavaScript / WASM (`npmjs.com`)**: `svcc` — **AVAILABLE** (`HTTP 404`)

---

## 1. Rust SDK & CLI (`crates.io`: `svcc`)

`crates/svcc` mirrors the `svlang` umbrella crate pattern — exposing both `lib.rs` (`use svcc::{sv_to_sc, sc_to_sv, prove_sources_equivalent};`) and `[[bin]] name = "svcc"`:

- **Library consumers** (such as `InFormal` for native `C2RTL` / `SEC`):
  ```toml
  [dependencies]
  svcc = "0.2"
  ```
- **CLI users**:
  ```bash
  cargo install svcc
  svcc roundtrip-check design_vip.sv
  ```
- **Pre-publish checklist for `crates.io`**:
  1. Ensure `svlang` is published to `crates.io` (or switch `svlang` dependency from `git = ...` to `version = "0.4"` in `[workspace.dependencies]`, since `crates.io` rejects git-only dependencies at publish time).
  2. Publish in topological order: `svcc-ir` -> `svcc-front-sv`, `svcc-front-sc`, `svcc-passes`, `svcc-equiv`, `svcc-emit-sc`, `svcc-emit-sv` -> `svcc`.

---

## 2. Python Wheels & CLI (`PyPI`: `pip install svcc`)

Scaffolded at `bindings/python/pyproject.toml` using **`maturin` + `PyO3`**:
- Compiles the pure-Rust `svcc` crate directly into a self-contained native CPython extension (`abi3-py39`, so a single wheel per OS/arch works across Python 3.9–3.14+ with **zero** runtime Rust/LLVM dependency!).
- **Planned Python API (`import svcc`)** for AI RTL/DV harnesses:
  ```python
  import svcc

  cpp_hdr = svcc.sv_to_sc("apb_item.sv", sv_code)
  sv_out  = svcc.sc_to_sv("apb_item.hpp", cpp_hdr)
  verdict = svcc.prove_equivalent("apb_item.sv", sv_code, "apb_item.hpp", cpp_hdr)
  ```
- **Build command**:
  ```bash
  cd bindings/python && maturin build --release
  ```

---

## 3. WebAssembly / TypeScript (`npmjs.com`: `svcc`)

Because the default feature set of `svcc` has **zero C/C++ link dependencies** (`svlang` is 100% pure Rust):
- Compiles cleanly to `wasm32-unknown-unknown` / `wasm32-wasip1` via `wasm-pack` for:
  1. Browser-based interactive SV <-> SystemC playground (companion artifact for arXiv / DAC / ICCAD papers).
  2. VS Code / IDE extension live preview alongside `svlang` LSP and `InFormal` workbench.

---

## 4. C / C++ Shared Library (`libsvcc.{dylib,so,a}` + `svcc.h`)

Adding `crate-type = ["rlib", "cdylib", "staticlib"]` + `cbindgen` lets C++17 EDA tools and SystemC simulators invoke `svcc_sv_to_sc(...)` directly over a stable C ABI.
