//! C++ and SystemC integer semantics for the subset parsed by [`crate::cxx`], made explicit
//! as typed `svcc_ir::hdl` expressions.
//!
//! This is the model of "what the generated C++ computes" that the formal gates rest on, so
//! every rule is written down here and nothing is inherited from the code that generated the
//! C++:
//!
//! - **Built-in arithmetic.** `bool` and the 8- and 16-bit integer types promote to `int`. An `sc_uint<W>` operand converts to
//!   `unsigned long long` and an `sc_int<W>` operand to `long long` (LP64: `int` is 32 bits,
//!   `long` and `long long` 64). Two operands are then brought to a common type by the usual
//!   arithmetic conversions, and the operation is carried out *in that type*: `a + b` on two
//!   `sc_uint<8>` values is a 64-bit addition.
//! - **Truncation happens only at a construction or assignment**: `sc_uint<8>(x)` keeps the
//!   low 8 bits of `x`.
//! - **`sc_biguint` / `sc_bigint`** arithmetic is exact (the result is as wide as it needs to
//!   be), as in SystemC's `sc_unsigned` / `sc_signed`.
//! - **Undefined or erroring operations** are not given a value. Each one contributes a
//!   condition to the value's `ub` flag: signed overflow, division by zero, a shift by a
//!   negative amount or by the width or more, a left shift of a negative value, a bit or
//!   range select outside the vector, an array index outside the array. Conditions that an
//!   interval analysis proves false are dropped; the rest are left for the solver, guarded
//!   by the `?:`, `&&` and `||` they sit under.
//! - Implementation-defined behaviour is fixed as every supported compiler has it (and as
//!   C++20 defines it): conversion to a narrower signed type wraps, `>>` of a negative value
//!   is an arithmetic shift.
//!
//! The rules for SystemC types were checked against SystemC 3.0.3 with a probe program; see
//! `docs/formal_gates.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::cxx::{err, Ast, CTy, SAst};
use svcc_ir::hdl::{BinOp, CaseArm, Expr, LocalVar, LvBase, LvSel, Lval, Pat, Stmt, Ty, UnOp};
use svcc_ir::SvccError;

/// Widest exact intermediate the reader will build for `sc_big*` arithmetic.
const MAX_EXACT_WIDTH: u32 = 8192;

/// Name of the process- or function-local that collects undefined-behaviour conditions.
pub(crate) const UB_LOCAL: &str = "svcc__ubl";

/// Static type of a C++ value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CT {
    Bool,
    Int {
        w: u32,
        s: bool,
    },
    /// `sc_uint` / `sc_int` object, or a bit or range of one (unsigned).
    Sc {
        w: u32,
        s: bool,
    },
    /// `sc_biguint` / `sc_bigint` object.
    Big {
        w: u32,
        s: bool,
    },
    /// `sc_unsigned` / `sc_signed` temporary holding an exact result in `w` bits.
    BigVal {
        w: u32,
        s: bool,
    },
    /// Range of an `sc_big*` object.
    BigSub {
        w: u32,
    },
    /// `sc_concatref`: only meaningful as the argument of a construction.
    Concat {
        w: u32,
    },
}

impl From<CTy> for CT {
    fn from(t: CTy) -> CT {
        match t {
            CTy::Bool => CT::Bool,
            CTy::Int { w, s } => CT::Int { w, s },
            CTy::Sc { w, s } => CT::Sc { w, s },
            CTy::Big { w, s } => CT::Big { w, s },
        }
    }
}

pub(crate) fn cty_ty(t: CTy) -> Ty {
    match t {
        CTy::Bool => Ty::BIT,
        CTy::Int { w, s } | CTy::Sc { w, s } | CTy::Big { w, s } => Ty::new(w, s),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Val {
    pub e: Expr,
    pub ct: CT,
    /// 1-bit condition under which evaluating this value is undefined in C++.
    pub ub: Option<Expr>,
}

// ----------------------------------------------------------------------
// Expression building blocks
// ----------------------------------------------------------------------

pub(crate) fn konst(ty: Ty, v: u64) -> Expr {
    Expr::konst(ty, v)
}

fn const_words(e: &Expr) -> Option<(&Vec<u64>, Ty)> {
    match e {
        Expr::Const { ty, words } => Some((words, *ty)),
        _ => None,
    }
}

/// Converts to `to` the way a C++ conversion or a SystemC construction does: the value is
/// extended by its own signedness or truncated, then reinterpreted.
pub(crate) fn conv(e: Expr, to: Ty) -> Expr {
    let from = e.ty();
    if from == to {
        return e;
    }
    if let Some((words, ty)) = const_words(&e) {
        let n = to.w.div_ceil(64) as usize;
        let neg = ty.s && (words[((ty.w - 1) / 64) as usize] >> ((ty.w - 1) % 64)) & 1 == 1;
        let mut out = vec![0u64; n];
        for (i, o) in out.iter_mut().enumerate() {
            let bit = i as u32 * 64;
            let mut w = if bit < ty.w {
                words.get(i).copied().unwrap_or(0)
            } else {
                0
            };
            if neg {
                if bit >= ty.w {
                    w = u64::MAX;
                } else if ty.w - bit < 64 {
                    w |= u64::MAX << (ty.w - bit);
                }
            }
            *o = w;
        }
        let top = to.w - 64 * (n as u32 - 1);
        if top < 64 {
            out[n - 1] &= (1u64 << top) - 1;
        }
        return Expr::Const { ty: to, words: out };
    }
    let mut x = e;
    if from.w != to.w {
        x = Expr::Resize {
            arg: Box::new(x),
            ty: Ty::new(to.w, from.s),
        };
    }
    if x.ty().s != to.s {
        x = Expr::Cast {
            arg: Box::new(x),
            ty: to,
        };
    }
    x
}

fn bin(op: BinOp, a: Expr, b: Expr, ty: Ty) -> Expr {
    Expr::Binary {
        op,
        lhs: Box::new(a),
        rhs: Box::new(b),
        ty,
    }
}

fn un(op: UnOp, a: Expr, ty: Ty) -> Expr {
    Expr::Unary {
        op,
        arg: Box::new(a),
        ty,
    }
}

pub(crate) fn truth(e: Expr) -> Expr {
    if e.ty() == Ty::BIT {
        e
    } else {
        un(UnOp::RedOr, e, Ty::BIT)
    }
}

fn not(e: Expr) -> Expr {
    un(UnOp::LogNot, e, Ty::BIT)
}

fn and(a: Expr, b: Expr) -> Expr {
    bin(BinOp::LogAnd, a, b, Ty::BIT)
}

pub(crate) fn or(a: Expr, b: Expr) -> Expr {
    bin(BinOp::LogOr, a, b, Ty::BIT)
}

fn or_opt(a: Option<Expr>, b: Option<Expr>) -> Option<Expr> {
    match (a, b) {
        (Some(a), Some(b)) => Some(or(a, b)),
        (a, None) => a,
        (None, b) => b,
    }
}

fn guard(g: Expr, ub: Option<Expr>) -> Option<Expr> {
    ub.map(|u| and(g, u))
}

fn is_false(e: &Expr) -> bool {
    e.as_u64() == Some(0)
}

// ----------------------------------------------------------------------
// Interval analysis
// ----------------------------------------------------------------------

fn ty_range(ty: Ty) -> Option<(i128, i128)> {
    if ty.w > 120 {
        return None;
    }
    if ty.s {
        let h = 1i128 << (ty.w - 1);
        Some((-h, h - 1))
    } else {
        Some((0, (1i128 << ty.w) - 1))
    }
}

fn const_i128(e: &Expr) -> Option<i128> {
    let (words, ty) = const_words(e)?;
    if ty.w > 64 {
        if words.iter().skip(1).all(|w| *w == 0) && !ty.s {
            return Some(words[0] as i128);
        }
        return None;
    }
    let v = words.first().copied().unwrap_or(0);
    Some(if ty.s && ty.w < 64 && (v >> (ty.w - 1)) & 1 == 1 {
        v as i128 - (1i128 << ty.w)
    } else if ty.s {
        v as i64 as i128
    } else {
        v as i128
    })
}

/// The expression under value-preserving conversions.
fn core(e: &Expr) -> &Expr {
    match e {
        Expr::Resize { arg, ty } if ty.w >= arg.ty().w => core(arg),
        Expr::Cast { arg, .. } => core(arg),
        _ => e,
    }
}

pub(crate) type Ranges = HashMap<String, (i128, i128)>;

/// Interval that provably contains the value of `e`.
pub(crate) fn rng(e: &Expr, env: &Ranges) -> Option<(i128, i128)> {
    let ty = e.ty();
    let tr = ty_range(ty);
    let fit = |r: Option<(i128, i128)>| -> Option<(i128, i128)> {
        match (r, tr) {
            (Some(r), Some(t)) if r.0 >= t.0 && r.1 <= t.1 => Some(r),
            _ => tr,
        }
    };
    match e {
        Expr::Const { .. } => const_i128(e).map(|v| (v, v)).or(tr),
        Expr::Local { name, .. } => fit(env.get(name).copied()),
        Expr::Resize { arg, .. } | Expr::Cast { arg, .. } => fit(rng(arg, env)),
        Expr::Unary {
            op: UnOp::Neg, arg, ..
        } => fit(rng(arg, env).map(|(l, h)| (-h, -l))),
        Expr::Unary { .. } => tr,
        Expr::Binary { op, lhs, rhs, .. } => {
            let (a, b) = (rng(lhs, env), rng(rhs, env));
            let big = |r: (i128, i128)| r.0.abs() > (1i128 << 100) || r.1.abs() > (1i128 << 100);
            let r = match (op, a, b) {
                (
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::LogAnd
                    | BinOp::LogOr,
                    _,
                    _,
                ) => Some((0, 1)),
                (BinOp::Add, Some(a), Some(b)) if !big(a) && !big(b) => {
                    Some((a.0 + b.0, a.1 + b.1))
                }
                (BinOp::Sub, Some(a), Some(b)) if !big(a) && !big(b) => {
                    Some((a.0 - b.1, a.1 - b.0))
                }
                (BinOp::Mul, Some(a), Some(b))
                    if [a.0, a.1, b.0, b.1].iter().all(|v| v.abs() < (1i128 << 60)) =>
                {
                    let c = [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1];
                    Some((*c.iter().min()?, *c.iter().max()?))
                }
                (BinOp::BitAnd, Some(a), Some(b)) if a.0 >= 0 && b.0 >= 0 => {
                    Some((0, a.1.min(b.1)))
                }
                (BinOp::BitAnd, Some(a), _) if a.0 >= 0 => Some((0, a.1)),
                (BinOp::BitAnd, _, Some(b)) if b.0 >= 0 => Some((0, b.1)),
                (BinOp::BitOr | BinOp::BitXor, Some(a), Some(b)) if a.0 >= 0 && b.0 >= 0 => {
                    let m = a.1.max(b.1) as u128;
                    let bits = 128 - m.leading_zeros();
                    Some((0, ((1u128 << bits.min(126)) - 1) as i128))
                }
                (BinOp::LShr | BinOp::AShr, Some(a), _) if a.0 >= 0 => Some((0, a.1)),
                (BinOp::Div, Some(a), Some(b)) if a.0 >= 0 && b.0 >= 0 => Some((0, a.1)),
                (BinOp::Mod, Some(a), Some(b)) if a.0 >= 0 && b.0 >= 0 => {
                    Some((0, a.1.min((b.1 - 1).max(0))))
                }
                _ => None,
            };
            fit(r)
        }
        Expr::Cond {
            cond,
            then_e,
            else_e,
            ..
        } => {
            let mut t = rng(then_e, env);
            let b = rng(else_e, env);
            // `x < c ? x : k`: the then-branch is below `c`. This is a statement about the
            // integer value of `x`, so it needs `x` to have that same value both where it is
            // compared and where it is selected.
            if let Expr::Binary {
                op: op @ (BinOp::Lt | BinOp::Le),
                lhs,
                rhs,
                ..
            } = cond.as_ref()
            {
                if core(lhs) == core(then_e) {
                    let x = rng(core(lhs), env);
                    let holds_in = |ty: Ty| match (x, ty_range(ty)) {
                        (Some(x), Some(r)) => x.0 >= r.0 && x.1 <= r.1,
                        _ => false,
                    };
                    if let (Some(c), Some(x)) = (const_i128(rhs), x) {
                        if holds_in(lhs.ty()) && holds_in(then_e.ty()) {
                            let hi = if *op == BinOp::Lt { c - 1 } else { c };
                            t = Some((x.0, x.1.min(hi)));
                        }
                    }
                }
            }
            match (t, b) {
                (Some(t), Some(b)) => fit(Some((t.0.min(b.0), t.1.max(b.1)))),
                _ => tr,
            }
        }
        Expr::CountOnes { arg, .. } => fit(Some((0, arg.ty().w as i128))),
        _ => tr,
    }
}

// ----------------------------------------------------------------------
// Loop headers in their plain form
// ----------------------------------------------------------------------
//
// C++ evaluates `i < 16` and `i = i + 1` on an `sc_int<32>` in 64 bits. A netlist can only
// unroll a loop whose header it can evaluate, so the two rewrites below put a header back
// in the width of its variable. Both hold for every value, not only for loop variables.

/// An expression with the same integer value as `e`, without the widening around it.
fn int_core<'e>(e: &'e Expr, env: &Ranges) -> &'e Expr {
    match e {
        Expr::Resize { arg, ty } if ty.w >= arg.ty().w => int_core(arg, env),
        Expr::Cast { arg, ty } => {
            let fits = match (rng(arg, env), ty_range(*ty)) {
                (Some(r), Some(t)) => r.0 >= t.0 && r.1 <= t.1,
                _ => false,
            };
            if fits {
                int_core(arg, env)
            } else {
                e
            }
        }
        _ => e,
    }
}

/// `a <rel> c` in the type of `a`, when the comparison is one of integer values and the
/// constant `c` has the same value in that type.
fn plain_compare(e: &Expr, env: &Ranges) -> Option<Expr> {
    let Expr::Binary {
        op: op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne),
        lhs,
        rhs,
        ..
    } = e
    else {
        return None;
    };
    // The comparison as written compares integer values only if neither side was
    // reinterpreted on the way to the common type.
    let wide = lhs.ty();
    let value_of = |x: &Expr| -> bool {
        let c = int_core(x, env);
        match (rng(c, env), ty_range(wide)) {
            (Some(r), Some(t)) => r.0 >= t.0 && r.1 <= t.1,
            _ => false,
        }
    };
    if !value_of(lhs) || !value_of(rhs) {
        return None;
    }
    let a = int_core(lhs, env);
    let c = const_i128(int_core(rhs, env))?;
    let (lo, hi) = ty_range(a.ty())?;
    if c < lo || c > hi || a.ty().w > 64 {
        return None;
    }
    Some(bin(*op, a.clone(), konst(a.ty(), c as u64), Ty::BIT))
}

/// The low `w` bits of `e`, computed in `w` bits. Sums, differences, products and bitwise
/// operations commute with truncation.
fn narrow(e: &Expr, w: u32) -> Option<Expr> {
    let ty = e.ty();
    if ty.w == w {
        return Some(e.clone());
    }
    if ty.w < w {
        return None;
    }
    let to = Ty::new(w, ty.s);
    match e {
        Expr::Const { .. } => Some(conv(e.clone(), to)),
        Expr::Resize { arg, .. } | Expr::Cast { arg, .. } => {
            let at = arg.ty();
            if at.w >= w {
                narrow(arg, w).map(|x| conv(x, to))
            } else {
                // Low bits of a widened value: the value widened less far.
                Some(conv(conv((**arg).clone(), Ty::new(w, at.s)), to))
            }
        }
        Expr::Binary {
            op:
                op @ (BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor),
            lhs,
            rhs,
            ..
        } => {
            let (a, b) = (narrow(lhs, w)?, narrow(rhs, w)?);
            Some(bin(*op, conv(a, to), conv(b, to), to))
        }
        Expr::Unary {
            op: op @ (UnOp::Neg | UnOp::BitNot),
            arg,
            ..
        } => Some(un(*op, conv(narrow(arg, w)?, to), to)),
        _ => None,
    }
}

/// `v = narrow(...)` for a header assignment whose right side is a truncated wider result.
fn plain_assign(s: Stmt) -> Stmt {
    match s {
        Stmt::Assign {
            lhs,
            rhs,
            nonblocking,
        } if lhs.sel.is_none() => {
            let ty = lhs.base.ty();
            let rhs = match &rhs {
                Expr::Resize { arg, .. } | Expr::Cast { arg, .. } if arg.ty().w >= ty.w => {
                    match narrow(arg, ty.w) {
                        Some(x) => conv(x, ty),
                        None => rhs,
                    }
                }
                _ => rhs,
            };
            Stmt::Assign {
                lhs,
                rhs,
                nonblocking,
            }
        }
        other => other,
    }
}

// ----------------------------------------------------------------------
// Scope
// ----------------------------------------------------------------------

/// A module signal or port as C++ sees it.
#[derive(Clone, Debug)]
pub(crate) struct SigInfo {
    /// Name in the design (the string literal the object was constructed with).
    pub name: String,
    pub ty: CTy,
    /// Element count for an `sc_vector`.
    pub dim: Option<u32>,
    /// An input port cannot be written.
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct FnSig {
    pub name: String,
    pub ret: CTy,
    pub args: Vec<CTy>,
}

#[derive(Clone, Debug)]
struct LocalInfo {
    hdl: String,
    ty: CTy,
    /// The local stands for this signal (copy-in at the top, copy-out at the bottom).
    alias: Option<String>,
    array: bool,
}

/// Lifting state of one process or function body.
pub(crate) struct Body<'a> {
    pub sigs: &'a HashMap<String, SigInfo>,
    pub funcs: &'a HashMap<String, FnSig>,
    scopes: Vec<HashMap<String, LocalInfo>>,
    pub locals: Vec<LocalVar>,
    used: BTreeSet<String>,
    ranges: Ranges,
    /// `.write` is a nonblocking assignment (edge-triggered process).
    pub seq: bool,
    /// Return type when lifting a function.
    pub ret: Option<CTy>,
    /// Some statement can raise the undefined-behaviour flag.
    pub ub_used: bool,
    /// Signals written through `.write` (not through an alias).
    pub written: BTreeSet<String>,
}

impl<'a> Body<'a> {
    pub(crate) fn new(
        sigs: &'a HashMap<String, SigInfo>,
        funcs: &'a HashMap<String, FnSig>,
        seq: bool,
        ret: Option<CTy>,
    ) -> Self {
        let mut used: BTreeSet<String> = sigs.values().map(|s| s.name.clone()).collect();
        used.extend(funcs.values().map(|f| f.name.clone()));
        used.insert("_ret".to_string());
        used.insert(UB_LOCAL.to_string());
        Body {
            sigs,
            funcs,
            scopes: vec![HashMap::new()],
            locals: Vec::new(),
            used,
            ranges: HashMap::new(),
            seq,
            ret,
            ub_used: false,
            written: BTreeSet::new(),
        }
    }

    fn lookup(&self, name: &str) -> Option<&LocalInfo> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// Declares a local and returns its name in the design.
    pub(crate) fn declare(&mut self, name: &str, ty: CTy) -> String {
        let mut hdl = if name.starts_with('_') {
            format!("svcc__c{name}")
        } else {
            name.to_string()
        };
        let base = hdl.clone();
        let mut k = 2;
        while !self.used.insert(hdl.clone()) {
            hdl = format!("{base}__{k}");
            k += 1;
        }
        self.locals.push(LocalVar {
            name: hdl.clone(),
            ty: cty_ty(ty),
        });
        self.scopes.last_mut().expect("scope").insert(
            name.to_string(),
            LocalInfo {
                hdl: hdl.clone(),
                ty,
                alias: None,
                array: false,
            },
        );
        hdl
    }

    /// Makes local `name` stand for signal `sig` (C++ identifier of the signal).
    pub(crate) fn alias(&mut self, name: &str, sig: &str, array: bool) -> Result<(), SvccError> {
        let Some(si) = self.sigs.get(sig) else {
            return err(format!("'{sig}' is not a signal"));
        };
        self.scopes.last_mut().expect("scope").insert(
            name.to_string(),
            LocalInfo {
                hdl: si.name.clone(),
                ty: si.ty,
                alias: Some(sig.to_string()),
                array,
            },
        );
        Ok(())
    }

    fn ub_stmt(&mut self, out: &mut Vec<Stmt>, ub: Option<Expr>) {
        let Some(u) = ub else { return };
        if is_false(&u) {
            return;
        }
        self.ub_used = true;
        let l = Expr::Local {
            name: UB_LOCAL.to_string(),
            ty: Ty::BIT,
        };
        out.push(Stmt::Assign {
            lhs: Lval {
                base: LvBase::Local {
                    name: UB_LOCAL.to_string(),
                    ty: Ty::BIT,
                },
                sel: None,
            },
            rhs: or(l, u),
            nonblocking: false,
        });
    }

    // ------------------------------------------------------------------
    // Values
    // ------------------------------------------------------------------

    /// Operand of a built-in operator after promotion, or `None` for an `sc_big*` value.
    fn builtin(&self, v: &Val) -> Result<Option<Expr>, SvccError> {
        Ok(Some(match v.ct {
            CT::Bool => conv(v.e.clone(), Ty::INT),
            // Integer promotion: every type narrower than `int` becomes `int`.
            CT::Int { w, .. } if w < 32 => conv(v.e.clone(), Ty::INT),
            CT::Int { .. } => v.e.clone(),
            CT::Sc { s, .. } => conv(v.e.clone(), Ty::new(64, s)),
            CT::Big { .. } | CT::BigVal { .. } | CT::BigSub { .. } => return Ok(None),
            CT::Concat { .. } => {
                return err("a concatenation used outside a construction");
            }
        }))
    }

    /// Exact value of any operand: expression, width, signedness.
    fn exact(&self, v: &Val) -> Result<(Expr, u32, bool), SvccError> {
        Ok(match v.ct {
            CT::Bool => (v.e.clone(), 1, false),
            CT::Int { w, s } | CT::Sc { w, s } | CT::Big { w, s } | CT::BigVal { w, s } => {
                (v.e.clone(), w, s)
            }
            CT::BigSub { w } => (v.e.clone(), w, false),
            CT::Concat { .. } => return err("a concatenation used outside a construction"),
        })
    }

    fn need_width(&self, w: u32) -> Result<u32, SvccError> {
        if w > MAX_EXACT_WIDTH {
            err(format!(
                "exact sc_big arithmetic needs {w} bits (limit {MAX_EXACT_WIDTH})"
            ))
        } else {
            Ok(w)
        }
    }

    /// 1-bit condition `e` is outside `[0, n)`, or `None` when the range analysis rules it out.
    fn outside(&self, e: &Expr, n: u64) -> Option<Expr> {
        if let Some((lo, hi)) = rng(e, &self.ranges) {
            if lo >= 0 && hi < n as i128 {
                return None;
            }
        }
        // As raw bits a negative value is a large unsigned one.
        let w = e.ty().w.max(64);
        let u = conv(conv(e.clone(), Ty::new(w, e.ty().s)), Ty::unsigned(w));
        Some(bin(BinOp::Ge, u, konst(Ty::unsigned(w), n), Ty::BIT))
    }

    fn val(e: Expr, ct: CT, ub: Option<Expr>) -> Val {
        Val { e, ct, ub }
    }

    pub(crate) fn expr(&mut self, a: &Ast) -> Result<Val, SvccError> {
        match a {
            Ast::Num { v, ty } => Ok(Self::val(konst(cty_ty(*ty), *v), (*ty).into(), None)),
            Ast::Bool(b) => Ok(Self::val(konst(Ty::BIT, *b as u64), CT::Bool, None)),
            Ast::Id(name) => {
                let Some(l) = self.lookup(name).cloned() else {
                    if self.sigs.contains_key(name) {
                        return err(format!("signal '{name}' used without .read()"));
                    }
                    return err(format!("unknown identifier '{name}'"));
                };
                if l.array {
                    return err(format!("array '{name}' used as a value"));
                }
                let ty = cty_ty(l.ty);
                let e = match l.alias {
                    Some(_) => Expr::Sig { name: l.hdl, ty },
                    None => Expr::Local { name: l.hdl, ty },
                };
                Ok(Self::val(e, l.ty.into(), None))
            }
            Ast::Un(op, x) => {
                let v = self.expr(x)?;
                self.unary(op, v)
            }
            Ast::Bin(op, x, y) => {
                let a = self.expr(x)?;
                let b = self.expr(y)?;
                self.binary(op, a, b)
            }
            Ast::Cond(c, x, y) => {
                let c = self.expr(c)?;
                let a = self.expr(x)?;
                let b = self.expr(y)?;
                self.cond(c, a, b)
            }
            Ast::Cast(ty, x) => {
                let v = self.expr(x)?;
                self.cast(*ty, v)
            }
            Ast::Comma(parts) => {
                let mut es = Vec::new();
                let mut ub = None;
                let mut total = 0u32;
                for p in parts {
                    let v = self.expr(p)?;
                    let w = match v.ct {
                        CT::Sc { w, .. } | CT::Big { w, .. } | CT::BigSub { w } => w,
                        _ => {
                            return err(
                                "concatenation part that is not an sc_uint/sc_int/sc_big value (a built-in operand would make this the C++ comma operator)",
                            )
                        }
                    };
                    total += w;
                    ub = or_opt(ub, v.ub);
                    es.push(conv(v.e, Ty::unsigned(w)));
                }
                let ty = Ty::unsigned(self.need_width(total)?);
                Ok(Self::val(
                    Expr::Concat { parts: es, ty },
                    CT::Concat { w: total },
                    ub,
                ))
            }
            Ast::Call(name, args) => self.call(name, args),
            Ast::Method(obj, name, args) => self.method(obj, name, args),
            Ast::Index(obj, idx) => {
                // Element of an aliased array, else a bit of a value.
                if let Ast::Id(n) = obj.as_ref() {
                    if let Some(l) = self.lookup(n).cloned() {
                        if l.array {
                            let (i, ub) = self.elem_index(&l.alias.unwrap_or_default(), idx)?;
                            let ty = cty_ty(l.ty);
                            return Ok(Self::val(
                                Expr::Elem {
                                    name: l.hdl,
                                    idx: vec![i],
                                    ty,
                                },
                                l.ty.into(),
                                ub,
                            ));
                        }
                    }
                }
                let base = self.expr(obj)?;
                let i = self.expr(idx)?;
                let w = match base.ct {
                    CT::Sc { w, .. } | CT::Big { w, .. } => w,
                    _ => {
                        return err(
                            "bit select of a value that is not an sc_uint/sc_int/sc_big object",
                        )
                    }
                };
                let Some(ie) = self.builtin(&i)? else {
                    return err("bit index that is not a built-in integer");
                };
                let ub = or_opt(or_opt(base.ub, i.ub), self.outside(&ie, w as u64));
                Ok(Self::val(
                    Expr::Slice {
                        base: Box::new(conv(base.e, Ty::unsigned(w))),
                        lo: 0,
                        lo_dyn: Some(Box::new(ie)),
                        ty: Ty::BIT,
                    },
                    CT::Sc { w: 1, s: false },
                    ub,
                ))
            }
        }
    }

    /// Index into signal array `sig` (C++ identifier), with its out-of-range condition.
    fn elem_index(&mut self, sig: &str, idx: &Ast) -> Result<(Expr, Option<Expr>), SvccError> {
        let n = self.sigs.get(sig).and_then(|s| s.dim).ok_or_else(|| {
            SvccError::new("svcc-front-sc/cxx", format!("'{sig}' is not an array"))
        })?;
        let i = self.expr(idx)?;
        let Some(ie) = self.builtin(&i)? else {
            return err("array index that is not a built-in integer");
        };
        let ub = or_opt(i.ub, self.outside(&ie, n as u64));
        Ok((ie, ub))
    }

    fn unary(&mut self, op: &str, v: Val) -> Result<Val, SvccError> {
        if op == "!" {
            return Ok(Self::val(not(truth(self.scalar(&v)?)), CT::Bool, v.ub));
        }
        match self.builtin(&v)? {
            Some(e) => {
                let ty = e.ty();
                let ct = CT::Int { w: ty.w, s: ty.s };
                match op {
                    "~" => Ok(Self::val(un(UnOp::BitNot, e, ty), ct, v.ub)),
                    _ => {
                        let mut ub = v.ub;
                        if ty.s {
                            let min = -(1i128 << (ty.w - 1));
                            if rng(&e, &self.ranges).is_none_or(|r| r.0 <= min) {
                                let m = konst(ty, 1u64 << (ty.w - 1));
                                ub = or_opt(ub, Some(bin(BinOp::Eq, e.clone(), m, Ty::BIT)));
                            }
                        }
                        Ok(Self::val(un(UnOp::Neg, e, ty), ct, ub))
                    }
                }
            }
            None => {
                let (e, w, s) = self.exact(&v)?;
                match (op, v.ct) {
                    // Complement over the declared width of an object.
                    ("~", CT::Big { .. }) => Ok(Self::val(
                        un(UnOp::BitNot, e, Ty::new(w, s)),
                        CT::BigVal { w, s },
                        v.ub,
                    )),
                    ("~", _) => {
                        err("`~` of an sc_big temporary (its width is not the reader's to guess)")
                    }
                    _ => {
                        let r = self.need_width(w + 2)?;
                        let x = conv(e, Ty::new(r, true));
                        Ok(Self::val(
                            un(UnOp::Neg, x, Ty::new(r, true)),
                            CT::BigVal { w: r, s: true },
                            v.ub,
                        ))
                    }
                }
            }
        }
    }

    /// Value usable as a truth value or converted to `bool`.
    fn scalar(&self, v: &Val) -> Result<Expr, SvccError> {
        match v.ct {
            CT::Concat { .. } => err("a concatenation used outside a construction"),
            _ => Ok(v.e.clone()),
        }
    }

    fn binary(&mut self, op: &str, a: Val, b: Val) -> Result<Val, SvccError> {
        if op == "&&" || op == "||" {
            let x = truth(self.scalar(&a)?);
            let y = truth(self.scalar(&b)?);
            // The right operand is evaluated only when the left one does not decide.
            let g = if op == "&&" {
                x.clone()
            } else {
                not(x.clone())
            };
            let ub = or_opt(a.ub, guard(g, b.ub));
            let e = if op == "&&" { and(x, y) } else { or(x, y) };
            return Ok(Self::val(e, CT::Bool, ub));
        }
        let ub0 = or_opt(a.ub.clone(), b.ub.clone());
        let (ba, bb) = (self.builtin(&a)?, self.builtin(&b)?);
        let (Some(x), Some(y)) = (ba, bb) else {
            return self.big_binary(op, &a, &b, ub0);
        };
        let (tx, ty) = (x.ty(), y.ty());
        // Usual arithmetic conversions.
        let common = if tx.w == ty.w {
            Ty::new(tx.w, tx.s && ty.s)
        } else if tx.w > ty.w {
            tx
        } else {
            ty
        };
        let cct = CT::Int {
            w: common.w,
            s: common.s,
        };
        match op {
            "+" | "-" | "*" => {
                let bop = match op {
                    "+" => BinOp::Add,
                    "-" => BinOp::Sub,
                    _ => BinOp::Mul,
                };
                let (cx, cy) = (conv(x, common), conv(y, common));
                let res = bin(bop, cx.clone(), cy.clone(), common);
                let mut ub = ub0;
                if common.s {
                    let exact = match (rng(&cx, &self.ranges), rng(&cy, &self.ranges)) {
                        (Some(p), Some(q)) => {
                            let c = match bop {
                                BinOp::Add => vec![p.0 + q.0, p.1 + q.1],
                                BinOp::Sub => vec![p.0 - q.1, p.1 - q.0],
                                _ => vec![p.0 * q.0, p.0 * q.1, p.1 * q.0, p.1 * q.1],
                            };
                            c.iter().min().copied().zip(c.iter().max().copied())
                        }
                        _ => None,
                    };
                    let lim = 1i128 << (common.w - 1);
                    if !exact.is_some_and(|(lo, hi)| lo >= -lim && hi < lim) {
                        // Signed overflow: the result in twice the width differs.
                        let wide = Ty::new(common.w * 2, true);
                        let full = bin(bop, conv(cx, wide), conv(cy, wide), wide);
                        ub = or_opt(
                            ub,
                            Some(bin(BinOp::Ne, full, conv(res.clone(), wide), Ty::BIT)),
                        );
                    }
                }
                Ok(Self::val(res, cct, ub))
            }
            "/" | "%" => {
                let bop = if op == "/" { BinOp::Div } else { BinOp::Mod };
                let (cx, cy) = (conv(x, common), conv(y, common));
                let mut ub = ub0;
                let ry = rng(&cy, &self.ranges);
                if !ry.is_some_and(|r| r.0 > 0 || r.1 < 0) {
                    ub = or_opt(
                        ub,
                        Some(bin(BinOp::Eq, cy.clone(), konst(common, 0), Ty::BIT)),
                    );
                }
                if common.s {
                    let min = -(1i128 << (common.w - 1));
                    let rx = rng(&cx, &self.ranges);
                    let safe =
                        rx.is_some_and(|r| r.0 > min) || ry.is_some_and(|r| r.0 > -1 || r.1 < -1);
                    if !safe {
                        let m = konst(common, 1u64 << (common.w - 1));
                        let ones = conv(konst(Ty::new(1, true), 1), common);
                        ub = or_opt(
                            ub,
                            Some(and(
                                bin(BinOp::Eq, cx.clone(), m, Ty::BIT),
                                bin(BinOp::Eq, cy.clone(), ones, Ty::BIT),
                            )),
                        );
                    }
                }
                Ok(Self::val(bin(bop, cx, cy, common), cct, ub))
            }
            "&" | "|" | "^" => {
                let bop = match op {
                    "&" => BinOp::BitAnd,
                    "|" => BinOp::BitOr,
                    _ => BinOp::BitXor,
                };
                Ok(Self::val(
                    bin(bop, conv(x, common), conv(y, common), common),
                    cct,
                    ub0,
                ))
            }
            "<<" | ">>" => {
                // The result has the promoted type of the left operand.
                let lct = CT::Int { w: tx.w, s: tx.s };
                let mut ub = or_opt(ub0, self.outside(&y, tx.w as u64));
                if op == ">>" {
                    let bop = if tx.s { BinOp::AShr } else { BinOp::LShr };
                    return Ok(Self::val(bin(bop, x, y, tx), lct, ub));
                }
                let res = bin(BinOp::Shl, x.clone(), y.clone(), tx);
                if tx.s {
                    // Left shift of a negative value, or one that loses bits, is undefined.
                    let rx = rng(&x, &self.ranges);
                    let ry = rng(&y, &self.ranges);
                    let safe = match (rx, ry) {
                        (Some(p), Some(q)) => {
                            p.0 >= 0
                                && q.0 >= 0
                                && q.1 < tx.w as i128
                                && (p.1 as u128) << q.1 < (1u128 << (tx.w - 1))
                        }
                        _ => false,
                    };
                    if !safe {
                        let neg = bin(BinOp::Lt, x.clone(), konst(tx, 0), Ty::BIT);
                        let back = bin(BinOp::AShr, res.clone(), y, tx);
                        ub = or_opt(ub, Some(or(neg, bin(BinOp::Ne, back, x, Ty::BIT))));
                    }
                }
                Ok(Self::val(res, lct, ub))
            }
            "==" | "!=" | "<" | "<=" | ">" | ">=" => Ok(Self::val(
                bin(cmp_op(op), conv(x, common), conv(y, common), Ty::BIT),
                CT::Bool,
                ub0,
            )),
            _ => err(format!("operator `{op}`")),
        }
    }

    fn big_binary(
        &mut self,
        op: &str,
        a: &Val,
        b: &Val,
        ub0: Option<Expr>,
    ) -> Result<Val, SvccError> {
        let (ea, wa, sa) = self.exact(a)?;
        let (eb, wb, sb) = self.exact(b)?;
        // Width of each operand as a signed number.
        let (swa, swb) = (wa + !sa as u32, wb + !sb as u32);
        let any_signed = sa || sb;
        let ext = |e: Expr, w: u32, s: bool| conv(e, Ty::new(w, s));
        match op {
            "<<" | ">>" => {
                let amt_big = !matches!(b.ct, CT::Bool | CT::Int { .. } | CT::Sc { .. });
                if amt_big {
                    return err("sc_big shift by an sc_big amount");
                }
                let Some((lo, hi)) = rng(&eb, &self.ranges) else {
                    return err("sc_big shift by an unbounded amount");
                };
                if lo < 0 {
                    return err("sc_big shift by an amount that may be negative");
                }
                if op == ">>" {
                    let bop = if sa { BinOp::AShr } else { BinOp::LShr };
                    let ty = Ty::new(wa, sa);
                    return Ok(Self::val(
                        bin(bop, ea, eb, ty),
                        CT::BigVal { w: wa, s: sa },
                        ub0,
                    ));
                }
                if hi > MAX_EXACT_WIDTH as i128 {
                    return err("sc_big left shift by an amount without a small bound");
                }
                let r = self.need_width(wa + hi as u32)?;
                let ty = Ty::new(r, sa);
                Ok(Self::val(
                    bin(BinOp::Shl, ext(ea, r, sa), eb, ty),
                    CT::BigVal { w: r, s: sa },
                    ub0,
                ))
            }
            "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                let (r, s) = if any_signed {
                    (swa.max(swb), true)
                } else {
                    (wa.max(wb), false)
                };
                let r = self.need_width(r)?;
                let (x, y) = if s {
                    (
                        conv(ext(ea, r, sa), Ty::new(r, true)),
                        conv(ext(eb, r, sb), Ty::new(r, true)),
                    )
                } else {
                    (ext(ea, r, false), ext(eb, r, false))
                };
                Ok(Self::val(bin(cmp_op(op), x, y, Ty::BIT), CT::Bool, ub0))
            }
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" => {
                // Subtraction always yields a signed result in SystemC.
                let s = any_signed || op == "-";
                let (ia, ib) = if s { (swa, swb) } else { (wa, wb) };
                let r = match op {
                    "+" | "-" | "/" | "%" => ia.max(ib) + 1,
                    "*" => ia + ib,
                    _ => ia.max(ib),
                };
                let r = self.need_width(r)?;
                let ty = Ty::new(r, s);
                let x = conv(ext(ea, r, sa), ty);
                let y = conv(ext(eb, r, sb), ty);
                let bop = match op {
                    "+" => BinOp::Add,
                    "-" => BinOp::Sub,
                    "*" => BinOp::Mul,
                    "/" => BinOp::Div,
                    "%" => BinOp::Mod,
                    "&" => BinOp::BitAnd,
                    "|" => BinOp::BitOr,
                    _ => BinOp::BitXor,
                };
                let mut ub = ub0;
                if matches!(bop, BinOp::Div | BinOp::Mod) {
                    // SystemC reports an error for division by zero.
                    let zero = conv(konst(Ty::BIT, 0), ty);
                    ub = or_opt(ub, Some(bin(BinOp::Eq, y.clone(), zero, Ty::BIT)));
                }
                Ok(Self::val(bin(bop, x, y, ty), CT::BigVal { w: r, s }, ub))
            }
            _ => err(format!("operator `{op}` on sc_big values")),
        }
    }

    fn cond(&mut self, c: Val, a: Val, b: Val) -> Result<Val, SvccError> {
        let ce = truth(self.scalar(&c)?);
        let ub = or_opt(
            or_opt(c.ub, guard(ce.clone(), a.ub.clone())),
            guard(not(ce.clone()), b.ub.clone()),
        );
        let mk = |x: Expr, y: Expr, ty: Ty| Expr::Cond {
            cond: Box::new(ce.clone()),
            then_e: Box::new(x),
            else_e: Box::new(y),
            ty,
        };
        let class = |ct: CT| matches!(ct, CT::Sc { .. } | CT::Big { .. });
        if a.ct == b.ct && (a.ct == CT::Bool || class(a.ct)) {
            let ty = a.e.ty();
            return Ok(Self::val(mk(a.e, b.e, ty), a.ct, ub));
        }
        if let (CT::BigVal { w: wa, s: sa }, CT::BigVal { w: wb, s: sb }) = (a.ct, b.ct) {
            if sa == sb {
                let ty = Ty::new(wa.max(wb), sa);
                return Ok(Self::val(
                    mk(conv(a.e, ty), conv(b.e, ty), ty),
                    CT::BigVal { w: ty.w, s: sa },
                    ub,
                ));
            }
        }
        let arith = |ct: CT| matches!(ct, CT::Bool | CT::Int { .. });
        if arith(a.ct) && arith(b.ct) {
            let (Some(x), Some(y)) = (self.builtin(&a)?, self.builtin(&b)?) else {
                return err("conditional operands");
            };
            let (tx, ty) = (x.ty(), y.ty());
            let common = if tx.w == ty.w {
                Ty::new(tx.w, tx.s && ty.s)
            } else if tx.w > ty.w {
                tx
            } else {
                ty
            };
            return Ok(Self::val(
                mk(conv(x, common), conv(y, common), common),
                CT::Int {
                    w: common.w,
                    s: common.s,
                },
                ub,
            ));
        }
        err("conditional operator whose branches have different class types")
    }

    pub(crate) fn cast(&mut self, ty: CTy, v: Val) -> Result<Val, SvccError> {
        let to = cty_ty(ty);
        let e = match (ty, v.ct) {
            (CTy::Bool, _) => truth(self.scalar(&v)?),
            (CTy::Int { .. }, CT::Bool | CT::Int { .. } | CT::Sc { .. }) => conv(v.e, to),
            (CTy::Int { .. }, _) => {
                return err("built-in cast of an sc_big value (use .to_uint64())");
            }
            // Constructions keep the low bits of the value.
            (CTy::Sc { .. } | CTy::Big { .. }, _) => conv(v.e, to),
        };
        Ok(Self::val(e, ty.into(), v.ub))
    }

    /// Converts a value to the type of the storage it is assigned or passed to.
    pub(crate) fn assign_conv(&mut self, ty: CTy, v: Val) -> Result<Val, SvccError> {
        match (ty, v.ct) {
            // No implicit conversion from sc_big types to a built-in integer.
            (
                CTy::Int { .. } | CTy::Bool,
                CT::Big { .. } | CT::BigVal { .. } | CT::BigSub { .. },
            ) => err("implicit conversion of an sc_big value to a built-in type"),
            (CTy::Int { .. } | CTy::Bool, CT::Concat { .. }) => {
                err("implicit conversion of a concatenation to a built-in type")
            }
            _ => self.cast(ty, v),
        }
    }

    fn call(&mut self, name: &str, args: &[Ast]) -> Result<Val, SvccError> {
        if name == "__builtin_popcountll" {
            let [a] = args else {
                return err("__builtin_popcountll takes one argument");
            };
            let v = self.expr(a)?;
            let v = self.assign_conv(CTy::Int { w: 64, s: false }, v)?;
            return Ok(Self::val(
                Expr::CountOnes {
                    arg: Box::new(v.e),
                    ty: Ty::INT,
                },
                CT::Int { w: 32, s: true },
                v.ub,
            ));
        }
        let Some(f) = self.funcs.get(name).cloned() else {
            return err(format!("call of unknown function '{name}'"));
        };
        if f.args.len() != args.len() {
            return err(format!("call of '{name}' with a wrong number of arguments"));
        }
        let mut es = Vec::new();
        let mut ub = None;
        for (a, t) in args.iter().zip(&f.args) {
            let v = self.expr(a)?;
            let v = self.assign_conv(*t, v)?;
            ub = or_opt(ub, v.ub);
            es.push(v.e);
        }
        // A lifted function returns its undefined-behaviour flag above its value.
        let rt = cty_ty(f.ret);
        let packed = Expr::Call {
            func: f.name.clone(),
            args: es,
            ty: Ty::unsigned(rt.w + 1),
        };
        let value = conv(
            Expr::Slice {
                base: Box::new(packed.clone()),
                lo: 0,
                lo_dyn: None,
                ty: Ty::unsigned(rt.w),
            },
            rt,
        );
        let flag = Expr::Slice {
            base: Box::new(packed),
            lo: rt.w,
            lo_dyn: None,
            ty: Ty::BIT,
        };
        Ok(Self::val(value, f.ret.into(), or_opt(ub, Some(flag))))
    }

    fn method(&mut self, obj: &Ast, name: &str, args: &[Ast]) -> Result<Val, SvccError> {
        if name == "read" && args.is_empty() {
            match obj {
                Ast::Id(s) if self.sigs.contains_key(s) && self.lookup(s).is_none() => {
                    let si = &self.sigs[s];
                    if si.dim.is_some() {
                        return err(format!("array '{s}' read without an index"));
                    }
                    return Ok(Self::val(
                        Expr::Sig {
                            name: si.name.clone(),
                            ty: cty_ty(si.ty),
                        },
                        si.ty.into(),
                        None,
                    ));
                }
                Ast::Index(arr, idx) => {
                    if let Ast::Id(s) = arr.as_ref() {
                        if self.sigs.contains_key(s) && self.lookup(s).is_none() {
                            let si = self.sigs[s].clone();
                            let (i, ub) = self.elem_index(s, idx)?;
                            return Ok(Self::val(
                                Expr::Elem {
                                    name: si.name,
                                    idx: vec![i],
                                    ty: cty_ty(si.ty),
                                },
                                si.ty.into(),
                                ub,
                            ));
                        }
                    }
                }
                _ => {}
            }
            return err(".read() on something that is not a signal");
        }
        let v = self.expr(obj)?;
        match (name, args) {
            ("to_uint64" | "to_int64", []) => {
                let s = name == "to_int64";
                match v.ct {
                    CT::Sc { .. } | CT::Big { .. } | CT::BigVal { .. } | CT::BigSub { .. } => Ok(
                        Self::val(conv(v.e, Ty::new(64, s)), CT::Int { w: 64, s }, v.ub),
                    ),
                    _ => err(format!(".{name}() on a built-in value")),
                }
            }
            ("xor_reduce" | "and_reduce" | "or_reduce", []) => {
                let w = match v.ct {
                    CT::Sc { w, .. } | CT::Big { w, .. } => w,
                    _ => return err(format!(".{name}() on a value that is not an sc object")),
                };
                let op = match name {
                    "xor_reduce" => UnOp::RedXor,
                    "and_reduce" => UnOp::RedAnd,
                    _ => UnOp::RedOr,
                };
                Ok(Self::val(
                    un(op, conv(v.e, Ty::unsigned(w)), Ty::BIT),
                    CT::Bool,
                    v.ub,
                ))
            }
            ("range", [hi, lo]) => {
                let (w, big) = match v.ct {
                    CT::Sc { w, .. } => (w, false),
                    CT::Big { w, .. } => (w, true),
                    _ => return err(".range() on a value that is not an sc object"),
                };
                let (rw, lo_e, ub) = self.range_bounds(hi, lo, w)?;
                let e = Expr::Slice {
                    base: Box::new(conv(v.e, Ty::unsigned(w))),
                    lo: 0,
                    lo_dyn: Some(Box::new(lo_e)),
                    ty: Ty::unsigned(rw),
                };
                let ct = if big {
                    CT::BigSub { w: rw }
                } else {
                    CT::Sc { w: rw, s: false }
                };
                Ok(Self::val(e, ct, or_opt(v.ub, ub)))
            }
            _ => err(format!("method .{name}()")),
        }
    }

    /// Width, low index and out-of-range condition of `.range(hi, lo)` on a `w`-bit object.
    /// The width must be a constant: `hi` is a constant, or `lo` plus a constant.
    fn range_bounds(
        &mut self,
        hi: &Ast,
        lo: &Ast,
        w: u32,
    ) -> Result<(u32, Expr, Option<Expr>), SvccError> {
        let l = self.expr(lo)?;
        let Some(le) = self.builtin(&l)? else {
            return err("range bound that is not a built-in integer");
        };
        let span: u64 = match (hi, lo) {
            (Ast::Num { v: h, .. }, Ast::Num { v: l, .. }) if h >= l => h - l,
            (Ast::Bin(op, base, k), _) if op == "+" && base.as_ref() == lo => match k.as_ref() {
                Ast::Num { v, .. } => *v,
                _ => return err("range whose width is not a constant"),
            },
            _ => return err("range whose width is not a constant"),
        };
        if span >= w as u64 {
            return err("range wider than the object");
        }
        let rw = span as u32 + 1;
        // In range when 0 <= lo and lo + rw <= w. `hi = lo + span` is computed in the type
        // of `lo`; it cannot wrap when `lo` is in range.
        let ub = or_opt(l.ub, self.outside(&le, (w - rw + 1) as u64));
        Ok((rw, le, ub))
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    pub(crate) fn stmts(&mut self, ss: &[SAst]) -> Result<Vec<Stmt>, SvccError> {
        let mut out = Vec::new();
        for s in ss {
            self.stmt(s, &mut out)?;
        }
        Ok(out)
    }

    fn scoped(&mut self, ss: &[SAst]) -> Result<Vec<Stmt>, SvccError> {
        self.scopes.push(HashMap::new());
        let r = self.stmts(ss);
        self.scopes.pop();
        r
    }

    fn assign_local(&mut self, hdl: String, ty: CTy, v: Val, out: &mut Vec<Stmt>) {
        self.ub_stmt(out, v.ub);
        out.push(Stmt::Assign {
            lhs: Lval {
                base: LvBase::Local {
                    name: hdl,
                    ty: cty_ty(ty),
                },
                sel: None,
            },
            rhs: v.e,
            nonblocking: false,
        });
    }

    fn stmt(&mut self, s: &SAst, out: &mut Vec<Stmt>) -> Result<(), SvccError> {
        match s {
            SAst::Decl {
                ty,
                name,
                array,
                init,
            } => {
                if array.is_some() {
                    return err(format!(
                        "local array '{name}' that is not a copy of a signal array"
                    ));
                }
                // The initializer is evaluated before the name is in scope.
                let v = match init {
                    Some(i) => {
                        let v = self.expr(i)?;
                        self.assign_conv(*ty, v)?
                    }
                    None => match ty {
                        // SystemC objects start at 0; a built-in local has no value yet.
                        CTy::Sc { .. } | CTy::Big { .. } => Val {
                            e: conv(konst(Ty::BIT, 0), cty_ty(*ty)),
                            ct: (*ty).into(),
                            ub: None,
                        },
                        _ => return err(format!("built-in local '{name}' without an initializer")),
                    },
                };
                let hdl = self.declare(name, *ty);
                self.assign_local(hdl, *ty, v, out);
                Ok(())
            }
            SAst::Assign { lhs, rhs } => self.assign(lhs, rhs, out),
            SAst::Write { target, value } => {
                let v = self.expr(value)?;
                let (base, sty, ub) = match target {
                    Ast::Id(s) if self.sigs.contains_key(s) && self.lookup(s).is_none() => {
                        let si = self.sigs[s].clone();
                        if si.dim.is_some() {
                            return err(format!("array '{s}' written without an index"));
                        }
                        self.check_writable(s, &si)?;
                        (
                            LvBase::Sig {
                                name: si.name,
                                ty: cty_ty(si.ty),
                            },
                            si.ty,
                            None,
                        )
                    }
                    Ast::Index(arr, idx) => {
                        let Ast::Id(s) = arr.as_ref() else {
                            return err(".write() target");
                        };
                        if !self.sigs.contains_key(s) || self.lookup(s).is_some() {
                            return err(".write() on something that is not a signal");
                        }
                        let si = self.sigs[s].clone();
                        self.check_writable(s, &si)?;
                        let (i, ub) = self.elem_index(s, idx)?;
                        (
                            LvBase::Elem {
                                name: si.name,
                                idx: vec![i],
                                ty: cty_ty(si.ty),
                            },
                            si.ty,
                            ub,
                        )
                    }
                    _ => return err(".write() on something that is not a signal"),
                };
                let v = self.assign_conv(sty, v)?;
                self.ub_stmt(out, or_opt(ub, v.ub));
                out.push(Stmt::Assign {
                    lhs: Lval { base, sel: None },
                    rhs: v.e,
                    nonblocking: self.seq,
                });
                Ok(())
            }
            SAst::If {
                cond,
                then_s,
                else_s,
            } => {
                let c = self.expr(cond)?;
                let ce = truth(self.scalar(&c)?);
                self.ub_stmt(out, c.ub);
                let t = self.scoped(then_s)?;
                let e = self.scoped(else_s)?;
                out.push(Stmt::If {
                    cond: ce,
                    then_s: t,
                    else_s: e,
                });
                Ok(())
            }
            SAst::For {
                init,
                cond,
                step,
                body,
            } => {
                self.scopes.push(HashMap::new());
                let r = self.for_loop(init, cond, step, body, out);
                self.scopes.pop();
                r
            }
            SAst::Switch { sel, arms, default } => {
                let v = self.expr(sel)?;
                let Some(se) = self.builtin(&v)? else {
                    return err("switch on an sc_big value");
                };
                self.ub_stmt(out, v.ub);
                let w = se.ty().w;
                let sel_e = conv(se, Ty::unsigned(w));
                let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                let mut seen = BTreeSet::new();
                let mut harms = Vec::new();
                for (labels, body) in arms {
                    let mut pats = Vec::new();
                    for l in labels {
                        if !seen.insert(*l) {
                            return err("duplicate case label");
                        }
                        if *l & !mask != 0 {
                            return err("case label wider than the selector");
                        }
                        pats.push(Pat::Const {
                            value: vec![*l],
                            care: vec![mask],
                        });
                    }
                    harms.push(CaseArm {
                        pats,
                        body: self.scoped(body)?,
                    });
                }
                let d = self.scoped(default)?;
                out.push(Stmt::Case {
                    sel: sel_e,
                    arms: harms,
                    default: Some(d),
                });
                Ok(())
            }
            SAst::Block(b) => {
                let body = self.scoped(b)?;
                out.extend(body);
                Ok(())
            }
            SAst::Return(e) => {
                let Some(rt) = self.ret else {
                    return err("return in a process");
                };
                let v = self.expr(e)?;
                let v = self.assign_conv(rt, v)?;
                self.ub_stmt(out, v.ub);
                let w = cty_ty(rt).w;
                out.push(Stmt::Return(Expr::Concat {
                    parts: vec![
                        Expr::Local {
                            name: UB_LOCAL.to_string(),
                            ty: Ty::BIT,
                        },
                        conv(v.e, Ty::unsigned(w)),
                    ],
                    ty: Ty::unsigned(w + 1),
                }));
                Ok(())
            }
        }
    }

    fn check_writable(&mut self, c_name: &str, si: &SigInfo) -> Result<(), SvccError> {
        if !si.writable {
            return err(format!("write to input port '{}'", si.name));
        }
        self.written.insert(c_name.to_string());
        Ok(())
    }

    fn for_loop(
        &mut self,
        init: &[SAst],
        cond: &Ast,
        step: &[SAst],
        body: &[SAst],
        out: &mut Vec<Stmt>,
    ) -> Result<(), SvccError> {
        // Declarations of the header are locals of the enclosing process; their start
        // values are the loop's init assignments.
        let mut hinit = Vec::new();
        for s in init {
            self.stmt(s, &mut hinit)?;
        }
        // `for (v = c0; v < c1; v = v + k)` with a body that leaves `v` alone bounds `v`.
        let bound = loop_bound(init, cond, step, body);
        let mut bound_name = None;
        if let Some((var, lo, hi, _)) = &bound {
            if let Some(l) = self.lookup(var) {
                if l.alias.is_none() {
                    bound_name = Some(l.hdl.clone());
                    self.ranges.insert(l.hdl.clone(), (*lo, *hi));
                }
            }
        }
        let result = (|| -> Result<Stmt, SvccError> {
            let c = self.expr(cond)?;
            if c.ub.as_ref().is_some_and(|u| !is_false(u)) {
                return err("loop condition that may be undefined");
            }
            let ce = truth(self.scalar(&c)?);
            let ce = plain_compare(&ce, &self.ranges).unwrap_or(ce);
            let hbody = self.scoped(body)?;
            // The step sees the variable one past the loop range.
            if let (Some(n), Some((_, lo, hi, d))) = (&bound_name, &bound) {
                self.ranges
                    .insert(n.clone(), (*lo.min(&(*lo + *d)), *hi.max(&(*hi + *d))));
            }
            let mut hstep = Vec::new();
            for s in step {
                self.stmt(s, &mut hstep)?;
            }
            Ok(Stmt::For {
                init: Vec::new(),
                cond: ce,
                step: hstep,
                body: hbody,
            })
        })();
        if let Some(n) = &bound_name {
            self.ranges.remove(n);
        }
        let Stmt::For {
            cond: ce,
            step: hstep,
            body: hbody,
            ..
        } = result?
        else {
            return err("loop");
        };
        let plain = |ss: &[Stmt]| {
            ss.iter().all(|s| {
                matches!(
                    s,
                    Stmt::Assign {
                        lhs: Lval { sel: None, base: LvBase::Local { name, .. } },
                        ..
                    } if name != UB_LOCAL
                )
            })
        };
        if !plain(&hinit) || !plain(&hstep) {
            return err(
                "loop header that is not plain assignments to locals (or may be undefined)",
            );
        }
        if hinit.is_empty() || hstep.is_empty() {
            out.extend(hinit);
            let mut b = hbody;
            b.extend(hstep);
            // `v = c; for (; cond; ) { ...; v = next; }` is the loop
            // `for (v = c; cond; v = next) { ... }` written out: put it back in that
            // shape, which is the one a netlist can unroll.
            fn target(s: &Stmt) -> Option<&str> {
                match s {
                    Stmt::Assign {
                        lhs:
                            Lval {
                                sel: None,
                                base: LvBase::Local { name, .. } | LvBase::Sig { name, .. },
                            },
                        nonblocking: false,
                        ..
                    } => Some(name),
                    _ => None,
                }
            }
            let var = b.last().and_then(target).map(str::to_string);
            let init_ok = match (&var, out.last()) {
                (Some(v), Some(prev)) => target(prev) == Some(v.as_str()),
                _ => false,
            };
            if init_ok {
                let step = vec![plain_assign(b.pop().expect("step"))];
                let init = vec![out.pop().expect("init")];
                out.push(Stmt::For {
                    init,
                    cond: ce,
                    step,
                    body: b,
                });
            } else {
                out.push(Stmt::For {
                    init: Vec::new(),
                    cond: ce,
                    step: Vec::new(),
                    body: b,
                });
            }
        } else {
            out.push(Stmt::For {
                init: hinit,
                cond: ce,
                step: hstep.into_iter().map(plain_assign).collect(),
                body: hbody,
            });
        }
        Ok(())
    }

    /// Storage named by an assignment target: base, element type, out-of-range condition.
    fn storage(&mut self, a: &Ast) -> Result<(LvBase, CTy, Option<Expr>), SvccError> {
        match a {
            Ast::Id(n) => {
                let Some(l) = self.lookup(n).cloned() else {
                    return err(format!(
                        "assignment to '{n}', which is not a local variable"
                    ));
                };
                if l.array {
                    return err(format!("assignment to whole array '{n}'"));
                }
                let ty = cty_ty(l.ty);
                let base = match l.alias {
                    Some(_) => LvBase::Sig { name: l.hdl, ty },
                    None => LvBase::Local { name: l.hdl, ty },
                };
                Ok((base, l.ty, None))
            }
            Ast::Index(arr, idx) => {
                let Ast::Id(n) = arr.as_ref() else {
                    return err("assignment target");
                };
                let Some(l) = self.lookup(n).cloned() else {
                    return err(format!(
                        "assignment to '{n}', which is not a local variable"
                    ));
                };
                if !l.array {
                    return err("not an array element");
                }
                let (i, ub) = self.elem_index(&l.alias.unwrap_or_default(), idx)?;
                Ok((
                    LvBase::Elem {
                        name: l.hdl,
                        idx: vec![i],
                        ty: cty_ty(l.ty),
                    },
                    l.ty,
                    ub,
                ))
            }
            _ => err("assignment target"),
        }
    }

    fn assign(&mut self, lhs: &Ast, rhs: &Ast, out: &mut Vec<Stmt>) -> Result<(), SvccError> {
        let v = self.expr(rhs)?;
        let nb = false;
        // Whole variable or array element.
        let whole = match lhs {
            Ast::Id(_) => true,
            Ast::Index(arr, _) => {
                matches!(arr.as_ref(), Ast::Id(n) if self.lookup(n).is_some_and(|l| l.array))
            }
            _ => false,
        };
        if whole {
            let (base, ty, ub) = self.storage(lhs)?;
            let v = self.assign_conv(ty, v)?;
            self.ub_stmt(out, or_opt(ub, v.ub));
            out.push(Stmt::Assign {
                lhs: Lval { base, sel: None },
                rhs: v.e,
                nonblocking: nb,
            });
            return Ok(());
        }
        match lhs {
            // One bit: `x[i] = b` assigns `b != 0`.
            Ast::Index(obj, idx) => {
                let (base, ty, ub0) = self.storage(obj)?;
                let w = match ty {
                    CTy::Sc { w, .. } | CTy::Big { w, .. } => w,
                    _ => return err("bit assignment to a built-in variable"),
                };
                let i = self.expr(idx)?;
                let Some(ie) = self.builtin(&i)? else {
                    return err("bit index that is not a built-in integer");
                };
                let ub = or_opt(or_opt(ub0, i.ub), self.outside(&ie, w as u64));
                let b = truth(self.scalar(&v)?);
                self.ub_stmt(out, or_opt(ub, v.ub));
                out.push(Stmt::Assign {
                    lhs: Lval {
                        base,
                        sel: Some(LvSel {
                            lo: 0,
                            lo_dyn: Some(ie),
                            w: 1,
                        }),
                    },
                    rhs: b,
                    nonblocking: nb,
                });
                Ok(())
            }
            // `x.range(hi, lo) = v` keeps the low bits of `v`.
            Ast::Method(obj, name, args) if name == "range" && args.len() == 2 => {
                let (base, ty, ub0) = self.storage(obj)?;
                let w = match ty {
                    CTy::Sc { w, .. } | CTy::Big { w, .. } => w,
                    _ => return err("range assignment to a built-in variable"),
                };
                let (rw, lo_e, ub1) = self.range_bounds(&args[0], &args[1], w)?;
                let src = match v.ct {
                    CT::Concat { .. } => return err("range assigned from a bare concatenation"),
                    _ => conv(v.e, Ty::unsigned(rw)),
                };
                self.ub_stmt(out, or_opt(or_opt(ub0, ub1), v.ub));
                out.push(Stmt::Assign {
                    lhs: Lval {
                        base,
                        sel: Some(LvSel {
                            lo: 0,
                            lo_dyn: Some(lo_e),
                            w: rw,
                        }),
                    },
                    rhs: src,
                    nonblocking: nb,
                });
                Ok(())
            }
            _ => err("assignment target"),
        }
    }
}

fn cmp_op(op: &str) -> BinOp {
    match op {
        "==" => BinOp::Eq,
        "!=" => BinOp::Ne,
        "<" => BinOp::Lt,
        "<=" => BinOp::Le,
        ">" => BinOp::Gt,
        _ => BinOp::Ge,
    }
}

fn num(a: &Ast) -> Option<i128> {
    match a {
        Ast::Num { v, .. } => Some(*v as i128),
        Ast::Un(op, x) if op == "-" => num(x).map(|v| -v),
        Ast::Cast(_, x) => num(x),
        _ => None,
    }
}

/// `(variable, lowest, highest, step)` inside the body of a counted loop: `step` is the
/// signed amount added per iteration.
fn loop_bound(
    init: &[SAst],
    cond: &Ast,
    step: &[SAst],
    body: &[SAst],
) -> Option<(String, i128, i128, i128)> {
    let (var, start) = match init {
        [SAst::Decl {
            name,
            init: Some(i),
            array: None,
            ..
        }] => (name.clone(), num(i)?),
        [SAst::Assign {
            lhs: Ast::Id(name),
            rhs,
        }] => (name.clone(), num(rhs)?),
        _ => return None,
    };
    let [SAst::Assign {
        lhs: Ast::Id(sv),
        rhs: Ast::Bin(op, base, k),
    }] = step
    else {
        return None;
    };
    let k = num(k)?;
    if *sv != var || **base != Ast::Id(var.clone()) || k <= 0 {
        return None;
    }
    let delta = match op.as_str() {
        "+" => k,
        "-" => -k,
        _ => return None,
    };
    let Ast::Bin(rel, l, r) = cond else {
        return None;
    };
    if **l != Ast::Id(var.clone()) {
        return None;
    }
    let end = num(r)?;
    let (lo, hi) = match (rel.as_str(), delta > 0) {
        ("<", true) => (start, end - 1),
        ("<=", true) => (start, end),
        (">", false) => (end + 1, start),
        (">=", false) => (end, start),
        _ => return None,
    };
    if hi < lo || assigns(body, &var) {
        return None;
    }
    Some((var, lo, hi, delta))
}

fn lhs_base(a: &Ast) -> Option<&str> {
    match a {
        Ast::Id(n) => Some(n),
        Ast::Index(b, _) | Ast::Method(b, _, _) => lhs_base(b),
        _ => None,
    }
}

/// Whether `ss` assigns or redeclares `var`.
fn assigns(ss: &[SAst], var: &str) -> bool {
    ss.iter().any(|s| match s {
        SAst::Decl { name, .. } => name == var,
        SAst::Assign { lhs, .. } => lhs_base(lhs).is_none_or(|n| n == var),
        SAst::Write { .. } | SAst::Return(_) => false,
        SAst::If { then_s, else_s, .. } => assigns(then_s, var) || assigns(else_s, var),
        SAst::For {
            init, step, body, ..
        } => assigns(init, var) || assigns(step, var) || assigns(body, var),
        SAst::Switch { arms, default, .. } => {
            arms.iter().any(|a| assigns(&a.1, var)) || assigns(default, var)
        }
        SAst::Block(b) => assigns(b, var),
    })
}

/// Signals read by `ss`: `(design name, constant element index)`, with the reads of called
/// functions taken from `fn_reads`.
pub(crate) fn reads(
    ss: &[Stmt],
    fn_reads: &BTreeMap<String, BTreeSet<(String, Option<u64>)>>,
    out: &mut BTreeSet<(String, Option<u64>)>,
) {
    let on_expr = |e: &Expr, out: &mut BTreeSet<(String, Option<u64>)>| {
        e.walk(&mut |x| match x {
            Expr::Sig { name, .. } => {
                out.insert((name.clone(), None));
            }
            Expr::Elem { name, idx, .. } => {
                out.insert((name.clone(), idx.first().and_then(|i| i.as_u64())));
            }
            Expr::Call { func, .. } => {
                if let Some(r) = fn_reads.get(func) {
                    out.extend(r.iter().cloned());
                }
            }
            _ => {}
        });
    };
    for s in ss {
        match s {
            Stmt::Assign { lhs, rhs, .. } => {
                on_expr(rhs, out);
                if let Some(sel) = &lhs.sel {
                    if let Some(d) = &sel.lo_dyn {
                        on_expr(d, out);
                    }
                }
                if let LvBase::Elem { idx, .. } = &lhs.base {
                    idx.iter().for_each(|i| on_expr(i, out));
                }
            }
            Stmt::If {
                cond,
                then_s,
                else_s,
            } => {
                on_expr(cond, out);
                reads(then_s, fn_reads, out);
                reads(else_s, fn_reads, out);
            }
            Stmt::Case { sel, arms, default } => {
                on_expr(sel, out);
                for a in arms {
                    for p in &a.pats {
                        if let Pat::Expr(e) = p {
                            on_expr(e, out);
                        }
                    }
                    reads(&a.body, fn_reads, out);
                }
                if let Some(d) = default {
                    reads(d, fn_reads, out);
                }
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                reads(init, fn_reads, out);
                on_expr(cond, out);
                reads(step, fn_reads, out);
                reads(body, fn_reads, out);
            }
            Stmt::Return(e) => on_expr(e, out),
            Stmt::Block { body, .. } => reads(body, fn_reads, out),
        }
    }
}
