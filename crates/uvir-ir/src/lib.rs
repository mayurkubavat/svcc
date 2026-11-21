//! Core Universal Verification Intermediate Representation (`UVIR`).
//!
//! `uvir-ir` organizes verification semantics into six orthogonal dialects inside a
//! cache-friendly, index-addressed [`UvirModule`] (`ExprId(u32)` arena storage, zero
//! pointer-chasing):
//!
//! 1. **`types`**: Bit-accurate scalar/packed types, 2-state vs. 4-state tracking,
//!    IEEE 1800 context-determined expression widths, and GC-handle vs. C++17 value/factory
//!    ownership modes (`OwnershipMode`).
//! 2. **`uvm`**: Class roles (`uvm_sequence_item`, `uvm_sequence`, `uvm_reg`, `uvm_component`,
//!    etc.), IEEE 1800.2 field policy flags (`FieldOps`), phase methods, and RAL metadata.
//! 3. **`crv`**: Solver-neutral constraint expressions (`CrvExpr` arena), `soft` weights,
//!    implication (`->`), `unique`, and IEEE 1800-2023 `inside` set/range/tolerance items.
//! 4. **`cov`**: UCIS 1.0-isomorphic functional coverage (`CovergroupDecl`, `CoverpointDecl`,
//!    value/range/ignore/illegal `CovBin`s, and `CrossDecl`).
//! 5. **`sched`**: Process scheduling, `fork`/`join_any`/`disable fork` cancel-token blocks,
//!    and SystemC 3.0.3 `SC_PRE_TIMESTEP` ↔ SV `clocking #1step` sampling barriers.
//! 6. **`sva`**: Clocked temporal properties (`|->`, `|=>`, `##[m:n]`, `disable iff`).
//!
//! Third-party dialects, front-ends, passes, and code emitters plug in cleanly through
//! [`Frontend`], [`TransformPass`], [`BackendEmitter`], and [`DialectExtension`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Compact 32-bit index into [`CrvArena`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExprId(pub u32);

/// Source language of a lowered [`UvirModule`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceLang {
    SystemVerilogUvm,
    SystemCUvm,
    Synthetic,
}

/// Source provenance preserved across bidirectional round-trips.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_lang: Option<SourceLang>,
    pub file: String,
    pub line: u32,
    pub original_macros: Vec<String>,
}

// ============================================================================
// 1. `types` Dialect
// ============================================================================

/// Fundamental verification data category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScalarKind {
    /// 2-state packed vector (`bit [W-1:0]`, `sc_bv<W>`, `uint*_t`, `int*_t`).
    BitVector,
    /// 4-state packed vector (`logic [W-1:0]`, `reg [W-1:0]`, `sc_lv<W>`).
    LogicVector,
    /// Boolean predicate (`bit` / `bool`).
    Bool,
    /// Dynamic string (`string` / `std::string`).
    String,
    /// Real floating-point (`real` / `double`).
    Real,
    /// Handle or value-embedded object of another UVM/OOP class.
    ClassRef(String),
}

/// Lifetime & memory ownership lowered by `OwnershipPass`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnershipMode {
    #[default]
    Unspecified,
    /// SystemVerilog GC reference semantics (`pkt_h = new()`).
    SvGcHandle,
    /// SystemC/UVM TLM-2.0 & `uvm_sequence_item` by-value copy semantics (requires custom copy-ctor
    /// when `crave::crv_variable<T>` members are present).
    ScValueCopy,
    /// UVM-SystemC factory heap object (`type_id::create(...)`).
    ScFactoryRawPtr,
    /// Reference-counted shared ownership (`std::shared_ptr<T>`).
    ScSharedPtr,
}

/// Canonical width-, sign-, and state-resolved type descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UvirType {
    pub kind: ScalarKind,
    pub width: u32,
    pub signed: bool,
    pub is_four_state: bool,
}

impl UvirType {
    pub fn bitvec(width: u32, signed: bool) -> Self {
        Self {
            kind: ScalarKind::BitVector,
            width: width.max(1),
            signed,
            is_four_state: false,
        }
    }

    pub fn logicvec(width: u32, signed: bool) -> Self {
        Self {
            kind: ScalarKind::LogicVector,
            width: width.max(1),
            signed,
            is_four_state: true,
        }
    }

    pub fn bool_ty() -> Self {
        Self {
            kind: ScalarKind::Bool,
            width: 1,
            signed: false,
            is_four_state: false,
        }
    }
}

// ============================================================================
// 2. `uvm` Dialect
// ============================================================================

/// Semantic UVM role independent of source syntax (`extends uvm_*` vs `: public uvm::uvm_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UvmRole {
    PlainClass,
    Object,
    SequenceItem,
    Sequence,
    Reg,
    RegBlock,
    Component,
    Driver,
    Monitor,
    Sequencer,
    Agent,
    Env,
    Scoreboard,
    Subscriber,
    Test,
}

impl UvmRole {
    pub fn is_component(self) -> bool {
        matches!(
            self,
            Self::Component
                | Self::Driver
                | Self::Monitor
                | Self::Sequencer
                | Self::Agent
                | Self::Env
                | Self::Scoreboard
                | Self::Subscriber
                | Self::Test
        )
    }

    pub fn is_crv_capable(self) -> bool {
        matches!(
            self,
            Self::Object | Self::SequenceItem | Self::Sequence | Self::Reg
        )
    }
}

/// IEEE 1800.2 field operation policies (`UVM_ALL_ON`, `UVM_NOCOMPARE`, `do_execute_op`, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOps {
    pub copy: bool,
    pub compare: bool,
    pub print: bool,
    pub pack: bool,
    pub record: bool,
}

impl Default for FieldOps {
    fn default() -> Self {
        Self {
            copy: true,
            compare: true,
            print: true,
            pack: true,
            record: true,
        }
    }
}

/// Optional UVM RAL field slice metadata (for `uvm_reg` <-> `uvm_randomized_reg`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalFieldMeta {
    pub lsb: u32,
    pub width: u32,
    pub access: String,
    pub volatile: bool,
    pub reset: u64,
    pub has_reset: bool,
}

/// A class member property / `rand` variable / `crv_variable<T>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecl {
    pub name: String,
    pub ty: UvirType,
    pub is_rand: bool,
    pub is_randc: bool,
    pub ops: FieldOps,
    pub ral_meta: Option<RalFieldMeta>,
}

/// Standard UVM phase or lifecycle hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseKind {
    Build,
    Connect,
    EndOfElaboration,
    StartOfSimulation,
    Run,
    Extract,
    Check,
    Report,
    Final,
    SequenceBody,
    Custom,
}

/// Method / phase declaration on a [`ClassDecl`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodDecl {
    pub name: String,
    pub phase: PhaseKind,
    pub is_task: bool,
    pub is_virtual: bool,
    pub body_ops: Vec<SchedOp>,
}

/// Canonical UVM/OOP class representation in `UVIR`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassDecl {
    pub name: String,
    pub package_or_namespace: String,
    pub role: UvmRole,
    pub base_class: Option<String>,
    pub ownership: OwnershipMode,
    /// True when signed expression semantics follow IEEE 1800 (`set_sv_signedness(true)`).
    pub sv_signedness: bool,
    /// Synthesize explicit value-only copy-ctor / `operator=` for SystemC TLM safety.
    pub needs_value_copy_ctor: bool,
    /// Synthesize explicit `do_copy` / `do_compare` / `convert2string` hooks.
    pub synthesize_field_methods: bool,
    /// Register width in bits when `role == UvmRole::Reg`.
    pub reg_width_bits: Option<u32>,
    pub fields: Vec<FieldDecl>,
    pub constraints: Vec<ConstraintBlock>,
    pub covergroups: Vec<CovergroupDecl>,
    pub methods: Vec<MethodDecl>,
    pub provenance: Provenance,
}

// ============================================================================
// 3. `crv` Dialect (Arena-Allocated Constraint AST)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrvUnaryOp {
    Not,
    BitNot,
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrvBinaryOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    LogAnd,
    LogOr,
    Implies,
    Iff,
}

/// Element of an IEEE 1800-2023 `inside { ... }` or CRAVE `inside(...)` set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsideElem {
    Single(ExprId),
    Range {
        low: ExprId,
        high: ExprId,
    },
    /// IEEE 1800-2023 `[center +/- delta]` absolute tolerance range.
    ToleranceAbs {
        center: ExprId,
        delta: ExprId,
    },
    /// IEEE 1800-2023 `[center +%- pct]` percentage tolerance range.
    TolerancePct {
        center: ExprId,
        pct: ExprId,
    },
}

/// Solver-neutral constraint AST node stored in [`CrvArena`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrvExpr {
    Var(String),
    Const {
        value: u128,
        width: u32,
        signed: bool,
    },
    BoolConst(bool),
    Unary {
        op: CrvUnaryOp,
        arg: ExprId,
    },
    Binary {
        op: CrvBinaryOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    Inside {
        target: ExprId,
        elems: Vec<InsideElem>,
    },
    Unique {
        vars: Vec<String>,
    },
}

/// Contiguous arena for [`CrvExpr`] nodes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrvArena {
    nodes: Vec<CrvExpr>,
}

impl CrvArena {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    #[inline]
    pub fn alloc(&mut self, expr: CrvExpr) -> ExprId {
        let id = ExprId(self.nodes.len() as u32);
        self.nodes.push(expr);
        id
    }

    #[inline]
    pub fn get(&self, id: ExprId) -> &CrvExpr {
        &self.nodes[id.0 as usize]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Single hard or soft constraint clause within a [`ConstraintBlock`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintClause {
    pub expr: ExprId,
    pub is_soft: bool,
    pub soft_weight: Option<u32>,
}

/// Named constraint block (`constraint c_valid { ... }` <-> `crv_constraint c_valid { ... }`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintBlock {
    pub name: String,
    pub clauses: Vec<ConstraintClause>,
}

// ============================================================================
// 4. `cov` Dialect (UCIS 1.0 Isomorphic Coverage)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinKind {
    Values(Vec<u64>),
    Range { low: u64, high: u64 },
    IgnoreRange { low: u64, high: u64 },
    IllegalValues(Vec<u64>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CovBin {
    pub name: String,
    pub kind: BinKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverpointDecl {
    pub name: String,
    pub target_expr: String,
    pub iff_guard: Option<String>,
    pub bins: Vec<CovBin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossDecl {
    pub name: String,
    pub coverpoints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CovergroupDecl {
    pub name: String,
    pub coverpoints: Vec<CoverpointDecl>,
    pub crosses: Vec<CrossDecl>,
}

// ============================================================================
// 5. `sched` Dialect (Process Concurrency & Clocking Barriers)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinKind {
    JoinAll,
    JoinAny,
    JoinNone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedOp {
    WaitPosEdge {
        signal: String,
    },
    WaitDelayNs {
        ns: u64,
    },
    /// Maps SV `clocking cb @(posedge clk); input #1step ...` <-> SystemC 3.0.3 `SC_PRE_TIMESTEP`.
    SamplePreTimestep {
        clocking_name: String,
    },
    Fork {
        join: JoinKind,
        branches: Vec<Vec<SchedOp>>,
        cancel_on_exit: bool,
    },
    Assign {
        lhs: String,
        rhs: String,
        nonblocking: bool,
    },
    Call {
        stmt: String,
    },
}

// ============================================================================
// 6. `sva` Dialect (Temporal Assertions)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvaPropertyDecl {
    pub name: String,
    pub clock_signal: String,
    pub disable_iff: Option<String>,
    pub antecedent: String,
    pub min_delay: u32,
    pub max_delay: Option<u32>,
    pub consequent: String,
    pub is_cover: bool,
}

// ============================================================================
// Top-Level `UvirModule` & Extension Architecture
// ============================================================================

/// Complete compilation unit in `UVIR`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UvirModule {
    pub name: String,
    pub crv_arena: CrvArena,
    pub classes: Vec<ClassDecl>,
    pub standalone_covergroups: Vec<CovergroupDecl>,
    pub properties: Vec<SvaPropertyDecl>,
    /// Vendor/research dialect extensions keyed by namespace (e.g. `"ams"`, `"formal"`, `"mcp"`).
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl UvirModule {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn find_class(&self, name: &str) -> Option<&ClassDecl> {
        self.classes.iter().find(|c| c.name == name)
    }
}

/// Structured error across front-ends, passes, equivalence checkers, and emitters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvirError {
    pub stage: &'static str,
    pub message: String,
}

impl UvirError {
    pub fn new(stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

impl fmt::Display for UvirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.stage, self.message)
    }
}

impl std::error::Error for UvirError {}

/// Timing and size telemetry from a front-end parse + lower run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FrontendMetrics {
    pub frontend: String,
    pub parse_us: u64,
    pub lower_us: u64,
    pub classes_lowered: usize,
    pub constraints_lowered: usize,
    pub covergroups_lowered: usize,
}

/// Per-pass execution telemetry.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PassStats {
    pub pass_name: String,
    pub elapsed_us: u64,
    pub items_modified: usize,
}

/// Pluggable language front-end lowering source text into [`UvirModule`].
pub trait Frontend: Send + Sync {
    fn name(&self) -> &'static str;
    fn lower_str(
        &self,
        filename: &str,
        source: &str,
    ) -> Result<(UvirModule, FrontendMetrics), UvirError>;
}

/// Pluggable IR transformation / normalization pass.
pub trait TransformPass: Send + Sync {
    fn name(&self) -> &'static str;
    fn run(&self, module: &mut UvirModule) -> Result<PassStats, UvirError>;
}

/// Pluggable target code emitter (`SystemC/UVM`, `SV/UVM`, `Python/pyuvm`, `PSS`, `Rust`, etc.).
pub trait BackendEmitter: Send + Sync {
    fn target_name(&self) -> &'static str;
    fn capabilities(&self) -> TargetCapabilities;
    fn emit(&self, module: &UvirModule) -> Result<String, UvirError>;
}

/// Pluggable custom dialect extension validator for [`UvirModule::extensions`].
pub trait DialectExtension: Send + Sync {
    fn namespace(&self) -> &'static str;
    fn validate(&self, payload: &serde_json::Value) -> Result<(), String>;
}

// ============================================================================
// Multi-Language Hub-and-Spoke Capability & Legalization Model
// ============================================================================

/// Target language memory/lifetime model used by `TargetLegalizerPass` & `OwnershipPass`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryModel {
    /// Garbage-collected handles (`SystemVerilog`, `Python/pyuvm`, `Chisel/Scala`).
    GarbageCollected,
    /// Deterministic RAII + explicit value/smart-pointer semantics (`C++17/SystemC`, `Rust`).
    ValueAndRaii,
    /// Declarative action/flow graph without heap pointers (`IEEE 1800.1 PSS`).
    DeclarativeGraph,
    /// Statically elaborated processes & shared variables/protected types (`VHDL-2019/OSVVM`).
    StaticElaborated,
}

/// Target concurrency model for lowering `sched` dialect primitives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConcurrencyModel {
    /// Native `fork` / `join_any` / `disable fork` (`SystemVerilog`).
    StructuredForkJoin,
    /// Dynamic process spawn + cooperative cancel tokens (`SystemC sc_spawn`).
    SpawnWithCancelToken,
    /// Stackless `async`/`await` coroutines (`Python cocotb`, `Rust async`).
    AsyncCoroutines,
    /// Declarative parallel/schedule blocks (`IEEE 1800.1 PSS`).
    ActivitySchedule,
    /// Static `process` blocks + barrier synchronization (`VHDL OSVVM`).
    StaticProcesses,
}

/// How the target language executes `crv` constraints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverModel {
    /// Language-native solver (`SystemVerilog randomize() with`).
    LanguageBuiltin,
    /// Embedded expression-template / decorator library (`C++ CRAVE`, `Python PyVSC`).
    EmbeddedLibraryAst,
    /// Direct SMT-LIB2 / C-API solver call synthesized by `uvir` (`Rust`, `VHDL`, bare `C++`).
    SynthesizedSmtCall,
}

/// How the target language checks `sva` temporal assertions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemporalModel {
    /// Native IEEE 1800 SVA (`assert property`).
    NativeSva,
    /// Native IEEE 1850 PSL (`VHDL-2008/2019`).
    NativePsl,
    /// Lowered by `uvir` into a cycle-accurate observer state machine in `sched` (`SystemC`, `Python`, `Rust`).
    LoweredAutomaton,
}

/// Declarative capability profile enabling $O(N)$ multi-language target legalization.
///
/// Instead of writing $N \times (N-1)$ pairwise converters, every target backend declares
/// its [`TargetCapabilities`]. The shared middle-end legalizer lowers unsupported high-level
/// `UVIR` constructs (e.g., 4-state `logic`, tolerance `inside`, SVA properties, `disable fork`,
/// GC handles) into primitives the target natively supports prior to code emission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCapabilities {
    pub language_id: String,
    pub memory_model: MemoryModel,
    pub concurrency_model: ConcurrencyModel,
    pub solver_model: SolverModel,
    pub temporal_model: TemporalModel,
    pub native_four_state: bool,
    pub native_inside_tolerance: bool,
    pub native_covergroups: bool,
    pub ieee_1800_context_widths: bool,
}

impl TargetCapabilities {
    /// IEEE 1800-2023 SystemVerilog + IEEE 1800.2-2020 UVM.
    pub fn sv_uvm_1800_2023() -> Self {
        Self {
            language_id: "sv-uvm".into(),
            memory_model: MemoryModel::GarbageCollected,
            concurrency_model: ConcurrencyModel::StructuredForkJoin,
            solver_model: SolverModel::LanguageBuiltin,
            temporal_model: TemporalModel::NativeSva,
            native_four_state: true,
            native_inside_tolerance: true,
            native_covergroups: true,
            ieee_1800_context_widths: true,
        }
    }

    /// IEEE 1666-2023 SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC.
    pub fn sc_uvm_crave_fc4sc() -> Self {
        Self {
            language_id: "sc-uvm".into(),
            memory_model: MemoryModel::ValueAndRaii,
            concurrency_model: ConcurrencyModel::SpawnWithCancelToken,
            solver_model: SolverModel::EmbeddedLibraryAst,
            temporal_model: TemporalModel::LoweredAutomaton,
            native_four_state: true,
            native_inside_tolerance: false,
            native_covergroups: true,
            ieee_1800_context_widths: false,
        }
    }

    /// Future spoke: Python `cocotb 2.0` + `pyuvm` + `PyVSC`.
    pub fn python_cocotb_pyuvm() -> Self {
        Self {
            language_id: "py-uvm".into(),
            memory_model: MemoryModel::GarbageCollected,
            concurrency_model: ConcurrencyModel::AsyncCoroutines,
            solver_model: SolverModel::EmbeddedLibraryAst,
            temporal_model: TemporalModel::LoweredAutomaton,
            native_four_state: false,
            native_inside_tolerance: false,
            native_covergroups: true,
            ieee_1800_context_widths: false,
        }
    }

    /// Future spoke: Accellera IEEE 1800.1 Portable Stimulus (`PSS 3.0`).
    pub fn pss_1800_1() -> Self {
        Self {
            language_id: "pss".into(),
            memory_model: MemoryModel::DeclarativeGraph,
            concurrency_model: ConcurrencyModel::ActivitySchedule,
            solver_model: SolverModel::LanguageBuiltin,
            temporal_model: TemporalModel::LoweredAutomaton,
            native_four_state: false,
            native_inside_tolerance: false,
            native_covergroups: true,
            ieee_1800_context_widths: false,
        }
    }

    /// Future spoke: Rust native verification / Verilator / Arcilator testbench.
    pub fn rust_verilator() -> Self {
        Self {
            language_id: "rust-dv".into(),
            memory_model: MemoryModel::ValueAndRaii,
            concurrency_model: ConcurrencyModel::AsyncCoroutines,
            solver_model: SolverModel::SynthesizedSmtCall,
            temporal_model: TemporalModel::LoweredAutomaton,
            native_four_state: false,
            native_inside_tolerance: false,
            native_covergroups: false,
            ieee_1800_context_widths: false,
        }
    }
}

/// Dynamic hub-and-spoke registry of language front-ends, emitters, and dialect extensions.
#[derive(Default)]
pub struct LanguageRegistry {
    frontends: BTreeMap<String, Box<dyn Frontend>>,
    backends: BTreeMap<String, Box<dyn BackendEmitter>>,
    extensions: BTreeMap<String, Box<dyn DialectExtension>>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_frontend(&mut self, id: impl Into<String>, fe: Box<dyn Frontend>) {
        self.frontends.insert(id.into(), fe);
    }

    pub fn register_backend(&mut self, id: impl Into<String>, be: Box<dyn BackendEmitter>) {
        self.backends.insert(id.into(), be);
    }

    pub fn register_extension(&mut self, ext: Box<dyn DialectExtension>) {
        self.extensions.insert(ext.namespace().to_string(), ext);
    }

    pub fn frontend(&self, id: &str) -> Option<&dyn Frontend> {
        self.frontends.get(id).map(Box::as_ref)
    }

    pub fn backend(&self, id: &str) -> Option<&dyn BackendEmitter> {
        self.backends.get(id).map(Box::as_ref)
    }

    pub fn registered_frontends(&self) -> Vec<&str> {
        self.frontends.keys().map(String::as_str).collect()
    }

    pub fn registered_backends(&self) -> Vec<&str> {
        self.backends.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crv_arena_roundtrips_json_cleanly() {
        let mut m = UvirModule::new("unit");
        let v = m.crv_arena.alloc(CrvExpr::Var("addr".into()));
        let lo = m.crv_arena.alloc(CrvExpr::Const {
            value: 0x1000,
            width: 32,
            signed: false,
        });
        let hi = m.crv_arena.alloc(CrvExpr::Const {
            value: 0x1FFF,
            width: 32,
            signed: false,
        });
        let inside = m.crv_arena.alloc(CrvExpr::Inside {
            target: v,
            elems: vec![InsideElem::Range { low: lo, high: hi }],
        });
        assert_eq!(m.crv_arena.len(), 4);
        let json = serde_json::to_string(&m).expect("serialize");
        let back: UvirModule = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.crv_arena.get(inside), m.crv_arena.get(inside));
    }
}
