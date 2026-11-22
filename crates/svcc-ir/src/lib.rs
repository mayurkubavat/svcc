//! Core Universal Verification Intermediate Representation (`svcc-ir`) for SV/UVM <-> C++17/SystemC.
//!
//! `svcc-ir` models IEEE 1800-2023 SystemVerilog + IEEE 1800.2-2020 UVM and
//! IEEE 1666-2023 SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC across six dialects
//! inside an arena-indexed [`IrModule`]:
//!
//! 1. **`types`**: Bit-accurate 2-state/4-state vectors, enums (`EnumDecl`), signedness,
//!    and GC-handle vs. C++17 RAII/TLM value-copy ownership (`OwnershipMode`).
//! 2. **`uvm`**: Roles, IEEE 1800.2 field policies, RAL registers & `uvm_reg_block` maps,
//!    component hierarchy, TLM ports/connections, and phases.
//! 3. **`crv`**: Arena-allocated constraint expressions (`CrvArena`), `soft` weights,
//!    weighted distributions (`dist { := , :/ }`), `randc` cyclic variables, implication,
//!    `unique`, and IEEE 1800-2023 `inside` set/range/tolerance clauses.
//! 4. **`cov`**: UCIS 1.0-isomorphic functional coverage (`CovergroupDecl`, `CoverpointDecl`,
//!    value/range/ignore/illegal `CovBin`s, and `CrossDecl`).
//! 5. **`sched`**: Procedural phases, sequence dispatch, `fork`/`join_any`/`disable fork`
//!    cancel-token concurrency, and `clocking #1step` <-> `SC_PRE_TIMESTEP` sampling.
//! 6. **`sva`**: Clocked temporal properties (`|-> ##[m:n]`, `disable iff`, `assert`/`cover`)
//!    with cycle-accurate C++17 bitmask pipeline observer lowering.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Compact 32-bit index into [`CrvArena`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExprId(pub u32);

/// Source language of a lowered [`IrModule`].
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

/// Strongly typed enumeration declaration (`typedef enum bit [W-1:0] { ... } name_e` <-> `enum class`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumDecl {
    pub name: String,
    pub width: u32,
    pub variants: Vec<(String, u64)>,
}

/// Fundamental verification data category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScalarKind {
    /// 2-state packed vector (`bit [W-1:0]`, `sc_uint<W>`, `uint*_t`, `int*_t`).
    BitVector,
    /// 4-state packed vector (`logic [W-1:0]`, `reg [W-1:0]`, `sc_lv<W>`).
    LogicVector,
    /// Strongly typed enumeration referencing an [`EnumDecl`] by name.
    Enum(String),
    /// Boolean predicate (`bit` / `bool`).
    Bool,
    /// Dynamic string (`string` / `std::string`).
    String,
    /// Real floating-point (`real` / `double`).
    Real,
    /// Handle or pointer to another UVM/OOP class.
    ClassRef(String),
}

/// Lifetime & memory ownership lowered by `OwnershipPass`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnershipMode {
    #[default]
    Unspecified,
    /// SystemVerilog GC reference semantics (`pkt_h = new()`).
    SvGcHandle,
    /// SystemC/UVM TLM-2.0 & `uvm_sequence_item` by-value copy semantics.
    ScValueCopy,
    /// UVM-SystemC factory heap component/object (`type_id::create(...)`).
    ScFactoryRawPtr,
}

/// Canonical width-, sign-, and state-resolved type descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrType {
    pub kind: ScalarKind,
    pub width: u32,
    pub signed: bool,
    pub is_four_state: bool,
}

impl IrType {
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

    pub fn enum_ty(name: impl Into<String>, width: u32) -> Self {
        Self {
            kind: ScalarKind::Enum(name.into()),
            width: width.max(1),
            signed: false,
            is_four_state: false,
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
// 2. `uvm` Dialect (Roles, Field Ops, RAL Maps, Components, TLM)
// ============================================================================

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

/// IEEE 1800.2 field operation policies (`UVM_ALL_ON`, `UVM_NOCOMPARE`, `UVM_NOCOPY`, etc.).
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

/// UVM RAL field slice metadata (`uvm_reg_field::configure` <-> `uvm_randomized_reg::bind_field`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalFieldMeta {
    pub lsb: u32,
    pub width: u32,
    pub access: String,
    pub volatile: bool,
    pub reset: u64,
    pub has_reset: bool,
}

/// Register instance mapped inside a `uvm_reg_block` (`default_map.add_reg(...)`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalRegInstance {
    pub inst_name: String,
    pub class_name: String,
    pub offset: u64,
    pub rights: String,
}

/// Address map specification on a `UvmRole::RegBlock` class.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalMapSpec {
    pub map_name: String,
    pub base_addr: u64,
    pub addr_unit_bytes: u32,
    pub registers: Vec<RalRegInstance>,
}

/// Child UVM component instantiated via `type_id::create` in `build_phase`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubComponentDecl {
    pub name: String,
    pub class_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TlmPortKind {
    AnalysisPort,
    AnalysisExport,
    SeqItemPullPort,
}

/// TLM port/export member on a UVM component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlmPortDecl {
    pub name: String,
    pub kind: TlmPortKind,
    pub item_type: String,
}

/// TLM `connect_phase` binding (`initiator.connect(target)`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlmConnection {
    pub initiator: String,
    pub target: String,
}

/// A class member property / `rand` / `randc` variable / `crv_variable<T>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecl {
    pub name: String,
    pub ty: IrType,
    pub is_rand: bool,
    pub is_randc: bool,
    pub ops: FieldOps,
    pub ral_meta: Option<RalFieldMeta>,
}

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodDecl {
    pub name: String,
    pub phase: PhaseKind,
    pub is_task: bool,
    pub is_virtual: bool,
    pub body_ops: Vec<SchedOp>,
}

/// Canonical UVM/OOP class representation in `svcc-ir`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassDecl {
    pub name: String,
    pub package_or_namespace: String,
    pub role: UvmRole,
    pub base_class: Option<String>,
    /// Sequence/Driver parameterized item type when applicable (`uvm_driver #(apb_item)`).
    pub item_type: Option<String>,
    pub ownership: OwnershipMode,
    pub sv_signedness: bool,
    pub needs_value_copy_ctor: bool,
    pub synthesize_field_methods: bool,
    pub reg_width_bits: Option<u32>,
    pub ral_map: Option<RalMapSpec>,
    pub subcomponents: Vec<SubComponentDecl>,
    pub tlm_ports: Vec<TlmPortDecl>,
    pub tlm_connections: Vec<TlmConnection>,
    pub fields: Vec<FieldDecl>,
    pub constraints: Vec<ConstraintBlock>,
    pub covergroups: Vec<CovergroupDecl>,
    pub methods: Vec<MethodDecl>,
    pub provenance: Provenance,
}

// ============================================================================
// 3. `crv` Dialect (Arena-Allocated Constraint & Distribution AST)
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

/// Element of an IEEE 1800-2023 `inside { ... }` or `dist { ... }` set.
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

/// IEEE 1800 `:=` (weight per value in range) vs `:/` (weight divided across range).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistWeightKind {
    /// `:=` — each value in the range receives `weight`.
    PerValue,
    /// `:/` — `weight` is divided across the range cardinality.
    SplitRange,
}

/// Weighted distribution item inside `CrvExpr::Dist`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistItem {
    pub elem: InsideElem,
    pub kind: DistWeightKind,
    pub weight: u32,
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
    Dist {
        target: ExprId,
        items: Vec<DistItem>,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintClause {
    pub expr: ExprId,
    pub is_soft: bool,
    pub soft_weight: Option<u32>,
}

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
// 5. `sched` Dialect (Clocking `#1step`, `fork`/`disable fork`, & Phases)
// ============================================================================

/// Clocking block (`clocking cb @(posedge clk); default input #1step; ... endclocking`)
/// mapped to SystemC 3.0.3 `SC_PRE_TIMESTEP` pre-edge snapshot sampler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockingDecl {
    pub name: String,
    pub clock_signal: String,
    pub input_1step_signals: Vec<String>,
    pub output_signals: Vec<String>,
}

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
    /// `@(cb)` — advances to clocking event and latches `#1step` preponed inputs (`SC_PRE_TIMESTEP`).
    SamplePreTimestep {
        clocking_name: String,
    },
    Fork {
        join: JoinKind,
        branches: Vec<Vec<SchedOp>>,
        cancel_on_exit: bool,
    },
    /// Standalone `disable fork;` cancelling sibling spawned processes in enclosing scope.
    DisableFork,
    /// `wait fork;` barrier waiting on all spawned children in current scope.
    WaitFork,
    StartItem {
        item_var: String,
    },
    RandomizeItem {
        item_var: String,
    },
    FinishItem {
        item_var: String,
    },
    AnalysisWrite {
        port: String,
        expr: String,
    },
    Assign {
        lhs: String,
        rhs: String,
        nonblocking: bool,
    },
    Repeat {
        count: u32,
        body: Vec<SchedOp>,
    },
    Call {
        stmt: String,
    },
}

// ============================================================================
// 6. `sva` Dialect (Clocked Temporal Assertions & Covers)
// ============================================================================

/// Clocked bounded-delay implication property:
/// `assert/cover property (@(posedge clk) [disable iff (rst)] antecedent |-> ##[min:max] consequent)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvaPropertyDecl {
    pub name: String,
    pub clock_signal: String,
    /// Active-high reset disable expression (e.g. `!rst_n` or `rst`).
    pub disable_iff: Option<String>,
    pub antecedent: String,
    /// `0` for overlapping `|->`, `1` for `|=>`, or `m` in `|-> ##[m:n]`.
    pub min_delay: u32,
    /// `max_delay >= min_delay` (bounded window up to 63 cycles for 64-bit O(1) shift pipeline).
    pub max_delay: u32,
    pub consequent: String,
    pub is_cover: bool,
}

// ============================================================================
// Top-Level `IrModule` & SV <-> C++17 Capability Architecture
// ============================================================================

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrModule {
    pub name: String,
    pub crv_arena: CrvArena,
    pub enums: Vec<EnumDecl>,
    pub clockings: Vec<ClockingDecl>,
    pub classes: Vec<ClassDecl>,
    pub standalone_covergroups: Vec<CovergroupDecl>,
    pub properties: Vec<SvaPropertyDecl>,
    pub rtl_modules: Vec<RtlModule>,
    pub extensions: BTreeMap<String, serde_json::Value>,
}

// ============================================================================
// 7. `rtl` Dialect (Synthesizable IEEE 1800 <-> IEEE 1666.1 RTL & C2RTL SSA)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RtlPortDir {
    Input,
    Output,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtlPort {
    pub name: String,
    pub dir: RtlPortDir,
    pub width: u32,
    pub signed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtlSignal {
    pub name: String,
    pub width: u32,
    pub signed: bool,
    pub is_reg: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RtlStmt {
    Assign {
        lhs: String,
        rhs: String,
        nonblocking: bool,
    },
    IfElse {
        cond: String,
        then_body: Vec<RtlStmt>,
        else_body: Vec<RtlStmt>,
    },
}

/// Combinational process (`always_comb` / `assign` <-> `SC_METHOD` with complete `sensitive << ...`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtlCombBlock {
    pub name: String,
    pub sensitivity: Vec<String>,
    pub stmts: Vec<RtlStmt>,
}

/// Synchronous single-clock process (`always_ff @(posedge clk [or negedge rst_n])` <-> `SC_CTHREAD` / clocked `SC_METHOD`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtlSyncBlock {
    pub name: String,
    pub clock: String,
    pub reset: Option<String>,
    pub reset_active_low: bool,
    pub reset_stmts: Vec<RtlStmt>,
    pub body_stmts: Vec<RtlStmt>,
}

/// Synthesizable RTL module or SSA-lowered untimed C++17 golden model (`C2RTL`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtlModule {
    pub name: String,
    pub ports: Vec<RtlPort>,
    pub signals: Vec<RtlSignal>,
    pub comb_blocks: Vec<RtlCombBlock>,
    pub sync_blocks: Vec<RtlSyncBlock>,
    pub provenance: Provenance,
}

impl IrModule {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn find_class(&self, name: &str) -> Option<&ClassDecl> {
        self.classes.iter().find(|c| c.name == name)
    }

    pub fn find_enum(&self, name: &str) -> Option<&EnumDecl> {
        self.enums.iter().find(|e| e.name == name)
    }

    pub fn find_rtl_module(&self, name: &str) -> Option<&RtlModule> {
        self.rtl_modules.iter().find(|m| m.name == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvccError {
    pub stage: &'static str,
    pub message: String,
}

impl SvccError {
    pub fn new(stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

impl fmt::Display for SvccError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.stage, self.message)
    }
}

impl std::error::Error for SvccError {}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FrontendMetrics {
    pub frontend: String,
    pub parse_us: u64,
    pub lower_us: u64,
    pub classes_lowered: usize,
    pub constraints_lowered: usize,
    pub covergroups_lowered: usize,
    pub properties_lowered: usize,
    pub rtl_modules_lowered: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PassStats {
    pub pass_name: String,
    pub elapsed_us: u64,
    pub items_modified: usize,
}

pub trait Frontend: Send + Sync {
    fn name(&self) -> &'static str;
    fn lower_str(
        &self,
        filename: &str,
        source: &str,
    ) -> Result<(IrModule, FrontendMetrics), SvccError>;
}

pub trait TransformPass: Send + Sync {
    fn name(&self) -> &'static str;
    fn run(&self, module: &mut IrModule) -> Result<PassStats, SvccError>;
}

pub trait BackendEmitter: Send + Sync {
    fn target_name(&self) -> &'static str;
    fn capabilities(&self) -> TargetCapabilities;
    fn emit(&self, module: &IrModule) -> Result<String, SvccError>;
}

pub trait DialectExtension: Send + Sync {
    fn namespace(&self) -> &'static str;
    fn validate(&self, payload: &serde_json::Value) -> Result<(), String>;
}

// ============================================================================
// Capability & Legalization Model (Scoped to SV/UVM & C++17/SystemC)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryModel {
    /// SystemVerilog garbage-collected class handles.
    GarbageCollected,
    /// C++17 RAII + explicit TLM value-copy / factory pointer ownership.
    ValueAndRaii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConcurrencyModel {
    /// Native IEEE 1800 `fork` / `join_any` / `disable fork`.
    StructuredForkJoin,
    /// IEEE 1666 `sc_core::sc_spawn` + `sc_event_or_list` + cooperative cancel token.
    SpawnWithCancelToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverModel {
    /// IEEE 1800 built-in `randomize()` constraint solver.
    LanguageBuiltin,
    /// C++17 CRAVE expression-template + SMT backend (`crv_constraint`, `crv_soft_constraint`, `dist`).
    EmbeddedLibraryAst,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemporalModel {
    /// Native IEEE 1800 SVA (`assert property` / `cover property`).
    NativeSva,
    /// Cycle-accurate C++17 `sc_module` bitmask shift-window observer automaton.
    LoweredAutomaton,
}

/// Declarative capability profile driving `PassPipeline::for_target`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCapabilities {
    pub language_id: String,
    pub memory_model: MemoryModel,
    pub concurrency_model: ConcurrencyModel,
    pub solver_model: SolverModel,
    pub temporal_model: TemporalModel,
    pub native_four_state: bool,
    pub native_inside_tolerance: bool,
    pub native_randc: bool,
    pub native_covergroups: bool,
    pub ieee_1800_context_widths: bool,
    pub exact_dist_split_weights: bool,
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
            native_randc: true,
            native_covergroups: true,
            ieee_1800_context_widths: true,
            exact_dist_split_weights: true,
        }
    }

    /// IEEE 1666-2023 SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC (C++17).
    pub fn sc_uvm_crave_fc4sc() -> Self {
        Self {
            language_id: "sc-uvm".into(),
            memory_model: MemoryModel::ValueAndRaii,
            concurrency_model: ConcurrencyModel::SpawnWithCancelToken,
            solver_model: SolverModel::EmbeddedLibraryAst,
            temporal_model: TemporalModel::LoweredAutomaton,
            native_four_state: true,
            native_inside_tolerance: false,
            native_randc: false,
            native_covergroups: true,
            ieee_1800_context_widths: false,
            exact_dist_split_weights: false,
        }
    }
}

/// Registry of active language front-ends, emitters, and dialect extensions.
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
    fn crv_arena_and_dist_roundtrip_json_cleanly() {
        let mut m = IrModule::new("unit");
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
        let dist = m.crv_arena.alloc(CrvExpr::Dist {
            target: v,
            items: vec![DistItem {
                elem: InsideElem::Range { low: lo, high: hi },
                kind: DistWeightKind::SplitRange,
                weight: 80,
            }],
        });
        let json = serde_json::to_string(&m).expect("serialize");
        let back: IrModule = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.crv_arena.get(dist), m.crv_arena.get(dist));
    }
}
