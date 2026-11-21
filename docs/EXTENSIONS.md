# Extending `uvir` Within the SV <-> C++17 Ecosystem

`uvir` keeps its core crates decoupled through four traits in `svcc-ir` (`Frontend`, `TransformPass`, `BackendEmitter`, `DialectExtension`) plus `CppAstExtractor` in `svcc-front-sc`:

1. **Full Translation-Unit `libclang` Extractor (`crates/svcc-front-sc --features libclang`)**:
   - Implement `CppAstExtractor` over `clang-sys` + `compile_commands.json` to resolve C++17 preprocessor macros, `constexpr` template parameters, and multi-header VIP inheritance trees without changing `svcc-ir` or `svcc-emit-sv`.
2. **New Optimization / Legalization Passes (`TransformPass`)**:
   - Register passes in `PassPipeline` (for example: constraint-block partitioning for `P1` Boost.Proto compile-time scaling, `dist` $\text{LCM}$ weight normalization for `P2`, or dual-`uint64_t` `(val, xz_mask)` 4-state lowering for `P7`).
3. **Custom Verification Metadata (`DialectExtension`)**:
   - Attach vendor pragmas, UPF power domains, or formal assumptions under `UvirModule::extensions` without altering core IR serialization.
4. **In-Memory `PyO3` / MCP / WASM Packaging**:
   - Because `uvir` is pure Rust with zero C++ link-time dependencies, the entire transpiler + Z3 SMT-LIB2 miter generator embeds directly into AI verification harnesses or browser WASM artifacts.
