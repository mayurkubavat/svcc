//! Typed, elaborated RTL design IR (`hdl` dialect).
//!
//! This is the hub representation for synthesizable design conversion. A front end
//! (the SystemVerilog one lowers `svlang`'s elaborated design and bound expressions)
//! produces one [`Module`] per specialization, with every expression fully typed:
//!
//! - Every [`Expr`] node carries its own [`Ty`] (bit width and signedness). All
//!   width and sign conversions required by the source language are explicit
//!   [`Expr::Resize`] and [`Expr::Cast`] nodes, so a back end never applies
//!   context rules of its own.
//! - Values are two-state. Unknown (`x`) and high-impedance (`z`) bits are mapped
//!   to `0` by the front end.
//! - Bit positions are normalized: position `0` is the least significant bit of
//!   the operand regardless of the declared range direction, and unpacked array
//!   indices are zero-based element offsets.
//! - Parameters and generate constructs are resolved. Generate scopes survive only
//!   as name prefixes.

use serde::{Deserialize, Serialize};

/// Integral type: bit width (`>= 1`) and signedness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ty {
    pub w: u32,
    pub s: bool,
}

impl Ty {
    pub const BIT: Ty = Ty { w: 1, s: false };
    pub const INT: Ty = Ty { w: 32, s: true };

    pub fn new(w: u32, s: bool) -> Self {
        Ty { w: w.max(1), s }
    }

    pub fn unsigned(w: u32) -> Self {
        Ty::new(w, false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    /// Two's-complement negation, result truncated to the node type.
    Neg,
    /// Bitwise complement within the node type.
    BitNot,
    /// Logical negation of a truth value (operand of any type, result 1 bit).
    LogNot,
    /// Reduction operators (operand of any type, result 1 bit).
    RedAnd,
    RedOr,
    RedXor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    // Operands and result share the node type.
    Add,
    Sub,
    Mul,
    /// Division by zero yields `0`.
    Div,
    /// Modulo by zero yields `0`.
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    BitXnor,
    // Result has the type of the left operand; the right operand is an unsigned amount.
    Shl,
    /// Logical right shift (zero fill) regardless of signedness.
    LShr,
    /// Arithmetic right shift (sign fill when the node type is signed).
    AShr,
    // Operands share one type; result is 1 bit.
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Operands are truth values of any type; result is 1 bit.
    LogAnd,
    LogOr,
}

/// Typed expression tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    /// Constant. `words` holds the value bits, least significant word first, masked to `ty.w`.
    Const {
        ty: Ty,
        words: Vec<u64>,
    },
    /// Whole module signal or port.
    Sig {
        name: String,
        ty: Ty,
    },
    /// Element of an unpacked array signal. `idx` are zero-based element offsets, one per dimension.
    Elem {
        name: String,
        idx: Vec<Expr>,
        ty: Ty,
    },
    /// Process-local or function-local variable (including loop variables and arguments).
    Local {
        name: String,
        ty: Ty,
    },
    Unary {
        op: UnOp,
        arg: Box<Expr>,
        ty: Ty,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        ty: Ty,
    },
    Cond {
        cond: Box<Expr>,
        then_e: Box<Expr>,
        else_e: Box<Expr>,
        ty: Ty,
    },
    /// Concatenation, most significant part first. Parts are taken as raw bits.
    Concat {
        parts: Vec<Expr>,
        ty: Ty,
    },
    /// Replication of the raw bits of `arg`.
    Repl {
        count: u32,
        arg: Box<Expr>,
        ty: Ty,
    },
    /// `ty.w` bits of `base` starting at bit `lo + dynamic lo_dyn`. Bits outside `base` read as `0`.
    Slice {
        base: Box<Expr>,
        lo: u32,
        lo_dyn: Option<Box<Expr>>,
        ty: Ty,
    },
    /// Width change. Widening extends by the signedness of `arg`; narrowing truncates.
    /// `ty.s` equals the signedness of `arg`.
    Resize {
        arg: Box<Expr>,
        ty: Ty,
    },
    /// Reinterpretation of the same bits with another signedness (`ty.w` equals the width of `arg`).
    Cast {
        arg: Box<Expr>,
        ty: Ty,
    },
    /// Call of a module [`Function`].
    Call {
        func: String,
        args: Vec<Expr>,
        ty: Ty,
    },
    /// Number of set bits in the raw bits of `arg`.
    CountOnes {
        arg: Box<Expr>,
        ty: Ty,
    },
}

impl Expr {
    pub fn ty(&self) -> Ty {
        match self {
            Expr::Const { ty, .. }
            | Expr::Sig { ty, .. }
            | Expr::Elem { ty, .. }
            | Expr::Local { ty, .. }
            | Expr::Unary { ty, .. }
            | Expr::Binary { ty, .. }
            | Expr::Cond { ty, .. }
            | Expr::Concat { ty, .. }
            | Expr::Repl { ty, .. }
            | Expr::Slice { ty, .. }
            | Expr::Resize { ty, .. }
            | Expr::Cast { ty, .. }
            | Expr::Call { ty, .. }
            | Expr::CountOnes { ty, .. } => *ty,
        }
    }

    pub fn konst(ty: Ty, v: u64) -> Expr {
        let mut words = vec![0u64; ty.w.div_ceil(64) as usize];
        words[0] = if ty.w >= 64 {
            v
        } else {
            v & ((1u64 << ty.w) - 1)
        };
        Expr::Const { ty, words }
    }

    /// Constant value when the expression is a constant that fits 64 bits.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Expr::Const { words, .. } if words.iter().skip(1).all(|w| *w == 0) => {
                Some(words.first().copied().unwrap_or(0))
            }
            _ => None,
        }
    }

    /// Visits this node and all nodes below it.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        match self {
            Expr::Const { .. } | Expr::Sig { .. } | Expr::Local { .. } => {}
            Expr::Elem { idx, .. } => idx.iter().for_each(|e| e.walk(f)),
            Expr::Unary { arg, .. }
            | Expr::Repl { arg, .. }
            | Expr::Resize { arg, .. }
            | Expr::Cast { arg, .. }
            | Expr::CountOnes { arg, .. } => arg.walk(f),
            Expr::Binary { lhs, rhs, .. } => {
                lhs.walk(f);
                rhs.walk(f);
            }
            Expr::Cond {
                cond,
                then_e,
                else_e,
                ..
            } => {
                cond.walk(f);
                then_e.walk(f);
                else_e.walk(f);
            }
            Expr::Concat { parts, .. } => parts.iter().for_each(|e| e.walk(f)),
            Expr::Slice { base, lo_dyn, .. } => {
                base.walk(f);
                if let Some(d) = lo_dyn {
                    d.walk(f);
                }
            }
            Expr::Call { args, .. } => args.iter().for_each(|e| e.walk(f)),
        }
    }
}

/// Storage an assignment writes to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LvBase {
    Sig {
        name: String,
        ty: Ty,
    },
    Elem {
        name: String,
        idx: Vec<Expr>,
        ty: Ty,
    },
    Local {
        name: String,
        ty: Ty,
    },
}

impl LvBase {
    pub fn ty(&self) -> Ty {
        match self {
            LvBase::Sig { ty, .. } | LvBase::Elem { ty, .. } | LvBase::Local { ty, .. } => *ty,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            LvBase::Sig { name, .. } | LvBase::Elem { name, .. } | LvBase::Local { name, .. } => {
                name
            }
        }
    }
}

/// Bit range of the base that a partial assignment writes: `w` bits from `lo + lo_dyn`.
/// Bits that fall outside the base are not written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LvSel {
    pub lo: u32,
    pub lo_dyn: Option<Expr>,
    pub w: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lval {
    pub base: LvBase,
    /// `None` writes the whole base.
    pub sel: Option<LvSel>,
}

/// One `case` item pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pat {
    /// Matches when `(sel & care) == value`. A plain constant has every `care` bit set.
    Const { value: Vec<u64>, care: Vec<u64> },
    /// Matches when `sel == expr` (expression typed like the selector).
    Expr(Expr),
    /// Matches when `lo <= sel <= hi` (both typed like the selector).
    Range(Expr, Expr),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseArm {
    pub pats: Vec<Pat>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stmt {
    /// `rhs` already has the type of the written target.
    Assign {
        lhs: Lval,
        rhs: Expr,
        nonblocking: bool,
    },
    If {
        cond: Expr,
        then_s: Vec<Stmt>,
        else_s: Vec<Stmt>,
    },
    /// First matching arm wins.
    Case {
        sel: Expr,
        arms: Vec<CaseArm>,
        default: Option<Vec<Stmt>>,
    },
    /// `for (init; cond; step) body`. `init` and `step` are blocking assignments to locals.
    For {
        init: Vec<Stmt>,
        cond: Expr,
        step: Vec<Stmt>,
        body: Vec<Stmt>,
    },
    /// Return from a [`Function`]. The value already has the return type.
    Return(Expr),
    /// Nested scope with its own locals (one merged source block).
    Block {
        locals: Vec<LocalVar>,
        body: Vec<Stmt>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalVar {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub sig: String,
    pub posedge: bool,
    /// The edge signal is also read by the process body (asynchronous set or reset).
    pub is_async_ctrl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcKind {
    /// Combinational: sensitive to every signal it reads and does not write.
    Comb,
    /// Edge triggered.
    Seq { edges: Vec<Edge> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub name: String,
    pub kind: ProcKind,
    pub locals: Vec<LocalVar>,
    pub body: Vec<Stmt>,
    /// Number of source blocks merged into this process (more than one when several
    /// blocks drive parts of the same signal).
    pub merged_from: u32,
    /// Source line of the (first) originating block, `0` when synthetic.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub ret: Ty,
    pub args: Vec<LocalVar>,
    pub locals: Vec<LocalVar>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    In,
    Out,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    pub name: String,
    pub dir: Dir,
    pub ty: Ty,
    /// Declared without a packed range (maps to a plain boolean when 1 bit unsigned).
    pub scalar: bool,
    /// Unpacked dimension sizes; empty for a plain port.
    pub dims: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    pub name: String,
    pub ty: Ty,
    pub scalar: bool,
    /// Unpacked dimension sizes; empty for a plain signal.
    pub dims: Vec<u32>,
    /// Constant start value (value bits, least significant word first).
    pub init: Option<Vec<u64>>,
}

/// What an instance port is bound to: a signal or port of the parent, or one element of an
/// unpacked array signal. An array port is bound to a whole array of the same shape (`elem`
/// empty).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conn {
    pub port: String,
    pub sig: String,
    pub elem: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub module: String,
    pub conns: Vec<Conn>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// Unique specialization name.
    pub name: String,
    /// Source definition name.
    pub definition: String,
    /// Resolved parameter values of this specialization, for documentation.
    pub params: Vec<(String, String)>,
    pub ports: Vec<Port>,
    pub signals: Vec<Signal>,
    pub functions: Vec<Function>,
    pub processes: Vec<Process>,
    pub instances: Vec<Instance>,
    pub file: String,
    pub line: u32,
}

/// Elaborated design: modules in dependency order (leaf modules first).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Design {
    pub modules: Vec<Module>,
    /// Names of the top-level modules.
    pub tops: Vec<String>,
    /// Source constructs that were skipped on purpose (assertions, `initial` blocks, ...).
    pub notes: Vec<String>,
}

impl Design {
    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.name == name)
    }
}

impl Module {
    /// Names of the signals, ports and arrays that `stmts` read, including reads made by the
    /// module functions they call.
    pub fn reads(&self, stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
        let on_expr = |e: &Expr, out: &mut std::collections::BTreeSet<String>| {
            let mut calls = Vec::new();
            e.walk(&mut |x| match x {
                Expr::Sig { name, .. } | Expr::Elem { name, .. } => {
                    out.insert(name.clone());
                }
                Expr::Call { func, .. } => calls.push(func.clone()),
                _ => {}
            });
            for c in calls {
                if let Some(f) = self.functions.iter().find(|f| f.name == c) {
                    // Functions cannot recurse, so this terminates.
                    self.reads(&f.body, out);
                }
            }
        };
        for s in stmts {
            match s {
                Stmt::Assign { lhs, rhs, .. } => {
                    on_expr(rhs, out);
                    if let Some(LvSel {
                        lo_dyn: Some(d), ..
                    }) = &lhs.sel
                    {
                        on_expr(d, out);
                    }
                    if let LvBase::Elem { idx, .. } = &lhs.base {
                        idx.iter().for_each(|e| on_expr(e, out));
                    }
                }
                Stmt::If {
                    cond,
                    then_s,
                    else_s,
                } => {
                    on_expr(cond, out);
                    self.reads(then_s, out);
                    self.reads(else_s, out);
                }
                Stmt::Case { sel, arms, default } => {
                    on_expr(sel, out);
                    for a in arms {
                        for p in &a.pats {
                            match p {
                                Pat::Expr(e) => on_expr(e, out),
                                Pat::Range(lo, hi) => {
                                    on_expr(lo, out);
                                    on_expr(hi, out);
                                }
                                Pat::Const { .. } => {}
                            }
                        }
                        self.reads(&a.body, out);
                    }
                    if let Some(d) = default {
                        self.reads(d, out);
                    }
                }
                Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    self.reads(init, out);
                    on_expr(cond, out);
                    self.reads(step, out);
                    self.reads(body, out);
                }
                Stmt::Return(e) => on_expr(e, out),
                Stmt::Block { body, .. } => self.reads(body, out),
            }
        }
    }

    /// Names of the signals, ports and arrays that `stmts` assign.
    pub fn writes(stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
        for s in stmts {
            match s {
                Stmt::Assign { lhs, .. } => {
                    if !matches!(lhs.base, LvBase::Local { .. }) {
                        out.insert(lhs.base.name().to_string());
                    }
                }
                Stmt::If { then_s, else_s, .. } => {
                    Self::writes(then_s, out);
                    Self::writes(else_s, out);
                }
                Stmt::Case { arms, default, .. } => {
                    for a in arms {
                        Self::writes(&a.body, out);
                    }
                    if let Some(d) = default {
                        Self::writes(d, out);
                    }
                }
                Stmt::For {
                    init, step, body, ..
                } => {
                    Self::writes(init, out);
                    Self::writes(step, out);
                    Self::writes(body, out);
                }
                Stmt::Block { body, .. } => Self::writes(body, out),
                Stmt::Return(_) => {}
            }
        }
    }
}
