# Extending `uvir`: New Languages, Dialects, Passes, & Tooling Integrations

`uvir` is designed so new verification languages, domain-specific dialects, and AI-harness bindings can be added in $O(1)$ effort without modifying existing frontends or backends.

## 1. Adding a New Language Spoke (e.g., Python `cocotb` + `pyuvm` + `PyVSC`)

Adding language $L_{k}$ requires implementing two traits from `uvir_ir` and registering them in `LanguageRegistry`:

### Step 1: Declare `TargetCapabilities`
Define or select the capability descriptor in `uvir_ir::TargetCapabilities` (for example, `TargetCapabilities::python_cocotb_pyuvm()`):
- `memory_model: MemoryModel::GarbageCollected`
- `concurrency_model: ConcurrencyModel::AsyncCoroutines`
- `solver_model: SolverModel::EmbeddedLibraryAst` (`@vsc.randobj` / `vsc.rand_bit_t(W)`)
- `temporal_model: TemporalModel::LoweredAutomaton`
- `native_four_state: false`, `native_inside_tolerance: false`

Because `PassPipeline::for_target(caps)` runs `TargetLegalizerPass` automatically, IEEE 1800-2023 constructs like `[addr +/- 16]` or `uvm_reg` bit-slices are already normalized in `UVIR` before your emitter sees them.

### Step 2: Implement `BackendEmitter` (and/or `Frontend`)
Create `crates/uvir-emit-py` implementing `uvir_ir::BackendEmitter`:
```rust
use uvir_ir::{BackendEmitter, TargetCapabilities, UvirError, UvirModule};

pub struct PyUvmEmitter;

impl BackendEmitter for PyUvmEmitter {
    fn target_name(&self) -> &'static str { "py-uvm" }
    fn capabilities(&self) -> TargetCapabilities {
        TargetCapabilities::python_cocotb_pyuvm()
    }
    fn emit(&self, module: &UvirModule) -> Result<String, UvirError> {
        // Walk `module.classes`, `module.crv_arena`, and `cls.covergroups`
        // to emit `@vsc.randobj class <Name>(uvm_sequence_item): ...`
        todo!()
    }
}
```

### Step 3: Register in `LanguageRegistry`
Register `"py-uvm"` in `uvir-cli`:
```rust
reg.register_backend("py-uvm", Box::new(PyUvmEmitter));
```
Immediately, **both** `uvir transpile --from sv-uvm --to py-uvm` and `uvir transpile --from sc-uvm --to py-uvm` work with zero pairwise glue!

---

## 2. Planned Language Spokes & Mapping Blueprint

| Spoke Crate (Planned) | `uvm` / Topology Mapping | `crv` Mapping | `cov` Mapping | `sched` Mapping |
| :--- | :--- | :--- | :--- | :--- |
| `uvir-{front,emit}-py` | `pyuvm.uvm_sequence_item`, `uvm_driver` | `PyVSC` (`@vsc.constraint`, `vsc.rangelist`) | `vsc.covergroup` / `cocotb-coverage` | `async def` + `RisingEdge` + `First()` |
| `uvir-{front,emit}-pss` | `struct` / `action` / `component` | PSS `constraint` (`in [lo..hi]`) | PSS `covergroup` | `activity { parallel / select }` |
| `uvir-emit-rust` | Rust `struct` + trait harness | Direct Z3 / Bitwuzla Rust crate synthesis | Compact Rust bitmap + UCIS XML writer | `async`/`await` coroutines over Verilator/Arcilator |
| `uvir-{front,emit}-vhdl` | VHDL-2019 protected types / UVVM VVC | OSVVM `RandomPkg` (`RandInt` / set weights) | OSVVM `CovPType` (`AddBins`, `AddCross`) | `process` + `wait until rising_edge` |
| `uvir-emit-circt` | `moore` / `verif` dialect | `smt` dialect (`smt.bv.*`) | `verif` cover ops | `llhd` / `moore.coroutine` |

---

## 3. Enabling Full `libclang` C++17 Extraction in `uvir-front-sc`
`uvir-front-sc` isolates C++ parsing behind `pub trait CppAstExtractor`:
- **Default (`StructuralCppExtractor`)**: Zero external C/LLVM runtime dependencies, `<50 us` header lowering.
- **Full Translation Unit (`--features libclang`)**: Implement `CppAstExtractor` using `clang-sys` against `compile_commands.json` to resolve arbitrary C++17 `constexpr` templates, preprocessor macros, and multi-header inheritance hierarchies into `UvirModule`.

## 4. Adding Custom Dialects (`DialectExtension`)
Domain-specific extensions (e.g., `ams` real-number modeling, `upf` low-power supply states, or formal assumptions) attach under `UvirModule::extensions["<namespace>"]` and register a `DialectExtension` schema validator in `LanguageRegistry::register_extension`.

## 5. WASM Playground & AI Agent MCP / `PyO3` Bindings
Because `uvir-ir`, `uvir-front-sv` (`svlang`), `uvir-passes`, `uvir-emit-sc`, and `uvir-emit-sv` are 100% pure Rust with zero C++ link dependencies:
- **WebAssembly (`wasm32-unknown-unknown`)**: Compiles cleanly for an interactive browser SV <-> SystemC <-> Python playground (paper artifact).
- **MCP Server / `PyO3`**: Expose `transpile` and `prove_class_equivalence` as sub-millisecond in-memory tools for closed-loop AI RTL/DV agents.
