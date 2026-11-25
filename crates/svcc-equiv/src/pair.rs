//! Equivalence of two Btor2 transition systems that share their primary inputs.
//!
//! This is the proof engine behind the typed RTL gates. Each side is the `svlang` netlist of
//! one design, written as word-level Btor2 on its own (no textual miter, so the two sides may
//! define modules and packages with the same names). The product is built here:
//!
//! - inputs named `<top>.<port>` are the same free value on both sides in every step;
//! - outputs are found by the same naming (`output` lines of combinational ports, `state`
//!   lines of registered ones) and compared in every step;
//! - an optional flag output of the implementation side must stay 0.
//!
//! Two-state reference semantics, stated once here because they are part of what a verdict
//! means: an X or Z bit (the netlist writes it as a free input named `<prefix>_x_<n>`) reads
//! as 0, and division or modulo by zero yields 0. This is the model `docs/rtl_subset.md`
//! defines for the conversion.
//!
//! Two proof methods:
//!
//! - **Induction over paired state.** When every state of one side has a state of the same
//!   name and sort on the other, both sides start from the same arbitrary state. If the
//!   outputs agree and the next states agree again, the two designs agree in every step of
//!   every run that starts from equal states, with no bound.
//! - **Bounded check.** Otherwise, or when induction finds a difference that may come from an
//!   unreachable state, both sides start from their initial state (a state without one
//!   starts at 0) and the outputs are compared for a fixed number of steps.
//!
//! The parser is strict: a Btor2 line it does not model is an error, never skipped.

use crate::{run_z3_stdin, SmtVerdict};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::Instant;
use svcc_ir::SvccError;

const STAGE: &str = "svcc-equiv/pair";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairVerdict {
    /// Proved for every run from equal starting states, any length.
    ProvedInductive,
    /// No difference within `bound_steps` steps from the initial state. Not a full proof.
    BoundedNoDifference,
    /// A concrete input sequence makes the two sides differ (or raises the flag).
    Mismatch,
    /// The solver gave no answer (missing, timed out, or an error).
    Inconclusive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairReport {
    pub verdict: PairVerdict,
    pub spec_top: String,
    pub imp_top: String,
    pub outputs_compared: usize,
    pub states_paired: usize,
    /// Steps unrolled by the query that decided the verdict.
    pub bound_steps: u32,
    pub latency: u32,
    pub solver_time_us: u64,
    pub smtlib2_bytes: usize,
    /// What limited the method, and the counterexample when there is one.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PairOptions {
    /// Steps of the bounded check.
    pub bound: u32,
    /// The implementation's outputs are compared `latency` steps after the specification's.
    pub latency: u32,
    /// Inputs held at a constant in every step (a reset kept inactive, for a latency check).
    pub hold: Vec<(String, u64)>,
    /// Output of the implementation that must be 0 in every step.
    pub imp_flag: Option<String>,
    /// Output of the specification that must be 0 in every step.
    pub spec_flag: Option<String>,
    /// Try the check from an arbitrary state of both sides first (every register free at
    /// step 0). A design whose outputs depend only on a bounded window of past inputs passes
    /// it, and then the result holds at every step of every run.
    pub arbitrary_start: bool,
    /// Solver time limit per query, in milliseconds.
    pub timeout_ms: u32,
}

impl Default for PairOptions {
    fn default() -> Self {
        PairOptions {
            bound: 8,
            latency: 0,
            hold: Vec::new(),
            imp_flag: None,
            spec_flag: None,
            arbitrary_start: false,
            timeout_ms: 120_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sort {
    Bv(u32),
    Arr(u32, u32),
}

impl Sort {
    pub(crate) fn smt(self) -> String {
        match self {
            Sort::Bv(w) => format!("(_ BitVec {w})"),
            Sort::Arr(i, e) => format!("(Array (_ BitVec {i}) (_ BitVec {e}))"),
        }
    }
    pub(crate) fn zero(self) -> String {
        match self {
            Sort::Bv(w) => format!("(_ bv0 {w})"),
            Sort::Arr(i, e) => {
                format!("((as const (Array (_ BitVec {i}) (_ BitVec {e}))) (_ bv0 {e}))")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Node {
    Input {
        name: String,
    },
    State {
        name: String,
    },
    Const(String),
    Op {
        op: String,
        args: Vec<i64>,
        imm: Vec<u32>,
    },
}

/// One side of the product: Btor2 text, its top module name, and the Btor2 symbol of each
/// output port (a port missing here is looked up as `<top>.<port>`).
#[derive(Clone, Debug)]
pub struct PairSide<'a> {
    pub btor2: &'a str,
    pub top: &'a str,
    pub outputs: &'a [(String, String)],
}

pub(crate) struct Model {
    pub(crate) top: String,
    pub(crate) port_symbols: BTreeMap<String, String>,
    pub(crate) sorts: BTreeMap<u32, Sort>,
    pub(crate) nodes: BTreeMap<u32, (Sort, Node)>,
    pub(crate) init: BTreeMap<u32, i64>,
    pub(crate) next: BTreeMap<u32, i64>,
    pub(crate) outputs: BTreeMap<String, i64>,
    pub(crate) constraints: Vec<i64>,
    pub(crate) has_arrays: bool,
}

fn bad_line(line: &str, why: &str) -> SvccError {
    SvccError::new(STAGE, format!("btor2 line not modelled ({why}): {line}"))
}

/// Decimal or hex digits as an SMT bit-vector literal of `w` bits (value taken modulo `2^w`).
fn bv_literal(digits: &str, radix: u32, w: u32) -> Option<String> {
    let (neg, digits) = match digits.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, digits),
    };
    // Little-endian base-2^32 limbs.
    let mut limbs: Vec<u32> = vec![0];
    for c in digits.chars() {
        let d = c.to_digit(radix)?;
        let mut carry = d as u64;
        for l in limbs.iter_mut() {
            let v = *l as u64 * radix as u64 + carry;
            *l = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            limbs.push(carry as u32);
        }
    }
    let mut bits = String::with_capacity(w as usize);
    for i in (0..w).rev() {
        let limb = limbs.get((i / 32) as usize).copied().unwrap_or(0);
        bits.push(if (limb >> (i % 32)) & 1 == 1 {
            '1'
        } else {
            '0'
        });
    }
    let lit = format!("#b{bits}");
    Some(if neg { format!("(bvneg {lit})") } else { lit })
}

pub(crate) fn parse(side: &PairSide) -> Result<Model, SvccError> {
    let (btor2, top) = (side.btor2, side.top);
    let mut m = Model {
        top: top.to_string(),
        port_symbols: side.outputs.iter().cloned().collect(),
        sorts: BTreeMap::new(),
        nodes: BTreeMap::new(),
        init: BTreeMap::new(),
        next: BTreeMap::new(),
        outputs: BTreeMap::new(),
        constraints: Vec::new(),
        has_arrays: false,
    };
    for raw in btor2.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| -> Result<i64, SvccError> {
            t.get(i)
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| bad_line(line, "missing number"))
        };
        let id = num(0)? as u32;
        let op = *t.get(1).ok_or_else(|| bad_line(line, "no operator"))?;
        let sort_of = |m: &Model, i: usize| -> Result<Sort, SvccError> {
            let s = num(i)? as u32;
            m.sorts
                .get(&s)
                .copied()
                .ok_or_else(|| bad_line(line, "unknown sort"))
        };
        match op {
            "sort" => {
                let s = match t.get(2).copied() {
                    Some("bitvec") => Sort::Bv(num(3)? as u32),
                    Some("array") => {
                        let (i, e) = (sort_of(&m, 3)?, sort_of(&m, 4)?);
                        match (i, e) {
                            (Sort::Bv(i), Sort::Bv(e)) => {
                                m.has_arrays = true;
                                Sort::Arr(i, e)
                            }
                            _ => return Err(bad_line(line, "nested array sort")),
                        }
                    }
                    _ => return Err(bad_line(line, "sort kind")),
                };
                m.sorts.insert(id, s);
            }
            "input" | "state" => {
                let s = sort_of(&m, 2)?;
                let name = t.get(3).map(|s| s.to_string()).unwrap_or_default();
                let n = if op == "input" {
                    Node::Input { name }
                } else {
                    Node::State { name }
                };
                m.nodes.insert(id, (s, n));
            }
            "init" => {
                m.init.insert(num(3)? as u32, num(4)?);
            }
            "next" => {
                m.next.insert(num(3)? as u32, num(4)?);
            }
            "output" => {
                if let Some(name) = t.get(3) {
                    m.outputs.insert(name.to_string(), num(2)?);
                }
            }
            // Assertions of the design itself are not what is being checked here.
            "bad" => {}
            "constraint" => m.constraints.push(num(2)?),
            "fair" | "justice" => return Err(bad_line(line, "liveness")),
            "const" | "constd" | "consth" => {
                let s = sort_of(&m, 2)?;
                let Sort::Bv(w) = s else {
                    return Err(bad_line(line, "array constant"));
                };
                let digits = t.get(3).ok_or_else(|| bad_line(line, "no value"))?;
                let radix = match op {
                    "const" => 2,
                    "constd" => 10,
                    _ => 16,
                };
                let lit = bv_literal(digits, radix, w).ok_or_else(|| bad_line(line, "digits"))?;
                m.nodes.insert(id, (s, Node::Const(lit)));
            }
            "zero" | "one" | "ones" => {
                let s = sort_of(&m, 2)?;
                let Sort::Bv(w) = s else {
                    return Err(bad_line(line, "array constant"));
                };
                let lit = match op {
                    "zero" => format!("(_ bv0 {w})"),
                    "one" => format!("(_ bv1 {w})"),
                    _ => format!("(bvnot (_ bv0 {w}))"),
                };
                m.nodes.insert(id, (s, Node::Const(lit)));
            }
            _ => {
                let s = sort_of(&m, 2)?;
                let (nargs, nimm) = match op {
                    "not" | "neg" | "redand" | "redor" | "redxor" | "inc" | "dec" => (1, 0),
                    "slice" => (1, 2),
                    "uext" | "sext" => (1, 1),
                    "add" | "sub" | "mul" | "udiv" | "sdiv" | "urem" | "srem" | "smod" | "and"
                    | "or" | "xor" | "xnor" | "nand" | "nor" | "sll" | "srl" | "sra" | "concat"
                    | "eq" | "neq" | "ult" | "ulte" | "ugt" | "ugte" | "slt" | "slte" | "sgt"
                    | "sgte" | "implies" | "iff" | "read" => (2, 0),
                    "ite" | "write" => (3, 0),
                    _ => return Err(bad_line(line, "operator")),
                };
                let mut args = Vec::new();
                for i in 0..nargs {
                    args.push(num(3 + i)?);
                }
                let mut imm = Vec::new();
                for i in 0..nimm {
                    imm.push(num(3 + nargs + i)? as u32);
                }
                m.nodes.insert(
                    id,
                    (
                        s,
                        Node::Op {
                            op: op.to_string(),
                            args,
                            imm,
                        },
                    ),
                );
            }
        }
    }
    Ok(m)
}

impl Model {
    pub(crate) fn sort(&self, r: i64) -> Result<Sort, SvccError> {
        self.nodes
            .get(&(r.unsigned_abs() as u32))
            .map(|n| n.0)
            .ok_or_else(|| SvccError::new(STAGE, format!("btor2 reference to unknown node {r}")))
    }

    pub(crate) fn width(&self, r: i64) -> Result<u32, SvccError> {
        match self.sort(r)? {
            Sort::Bv(w) => Ok(w),
            Sort::Arr(..) => Err(SvccError::new(
                STAGE,
                format!("btor2 node {r} is an array where a vector is needed"),
            )),
        }
    }

    /// Port name of a top-level symbol (`<top>.<port>`), if it is one.
    pub(crate) fn port<'a>(&self, name: &'a str) -> Option<&'a str> {
        let rest = name.strip_prefix(self.top.as_str())?.strip_prefix('.')?;
        if rest.contains('.') {
            None
        } else {
            Some(rest)
        }
    }

    /// Name of a state without the top prefix: the key states are paired by.
    pub(crate) fn state_key(&self, name: &str) -> String {
        match name
            .strip_prefix(self.top.as_str())
            .and_then(|r| r.strip_prefix('.'))
        {
            Some(r) => r.to_string(),
            None => name.to_string(),
        }
    }

    pub(crate) fn states(&self) -> Vec<(u32, Sort, String)> {
        self.nodes
            .iter()
            .filter_map(|(id, (s, n))| match n {
                Node::State { name } => Some((*id, *s, name.clone())),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn output_ref(&self, port: &str) -> Option<i64> {
        let full = match self.port_symbols.get(port) {
            Some(s) => s.clone(),
            None => format!("{}.{}", self.top, port),
        };
        if let Some(r) = self.outputs.get(&full) {
            return Some(*r);
        }
        self.nodes.iter().find_map(|(id, (_, n))| match n {
            Node::State { name } | Node::Input { name } if *name == full => Some(*id as i64),
            _ => None,
        })
    }
}

/// True for the free input the netlist writes for an X or Z bit.
pub(crate) fn is_x_input(name: &str) -> bool {
    match name.rfind("_x_") {
        Some(i) => {
            !name.contains('.')
                && name[i + 3..].chars().all(|c| c.is_ascii_digit())
                && !name[i + 3..].is_empty()
        }
        None => false,
    }
}

fn is_flag_state(name: &str) -> bool {
    name.contains("svcc__ub")
}

pub(crate) struct Side<'a> {
    pub(crate) m: &'a Model,
    pub(crate) tag: char,
}

impl Side<'_> {
    pub(crate) fn r(&self, r: i64, step: u32) -> Result<String, SvccError> {
        let id = r.unsigned_abs();
        if r < 0 {
            self.m.width(r)?;
            Ok(format!("(bvnot {}{}_{})", self.tag, step, id))
        } else {
            Ok(format!("{}{}_{}", self.tag, step, id))
        }
    }

    /// Defines every node of this side at `step`. `state0` gives the step-0 term of a state.
    pub(crate) fn define_step(
        &self,
        smt: &mut String,
        step: u32,
        state0: &BTreeMap<u32, String>,
        hold: &BTreeMap<String, u64>,
    ) -> Result<(), SvccError> {
        let m = self.m;
        for (&id, (sort, node)) in &m.nodes {
            let me = format!("{}{}_{}", self.tag, step, id);
            let body = match node {
                Node::Input { name } => {
                    if is_x_input(name) {
                        sort.zero()
                    } else if let Some(p) = m.port(name) {
                        match (hold.get(p), sort) {
                            (Some(v), Sort::Bv(w)) => {
                                bv_literal(&v.to_string(), 10, *w).unwrap_or_else(|| sort.zero())
                            }
                            _ => {
                                // Declared once per step by the caller.
                                format!("|in{step}_{p}|")
                            }
                        }
                    } else {
                        let _ = writeln!(smt, "(declare-const {me}_free {})", sort.smt());
                        format!("{me}_free")
                    }
                }
                Node::State { .. } => {
                    if step == 0 {
                        state0.get(&id).cloned().unwrap_or_else(|| sort.zero())
                    } else {
                        match m.next.get(&id) {
                            Some(n) => self.r(*n, step - 1)?,
                            None => format!("{}{}_{}", self.tag, step - 1, id),
                        }
                    }
                }
                Node::Const(lit) => lit.clone(),
                Node::Op { op, args, imm } => self.op(op, args, imm, *sort, step)?,
            };
            let _ = writeln!(smt, "(define-fun {me} () {} {body})", sort.smt());
        }
        for c in &m.constraints {
            let _ = writeln!(smt, "(assert (= {} #b1))", self.r(*c, step)?);
        }
        Ok(())
    }

    pub(crate) fn op(
        &self,
        op: &str,
        args: &[i64],
        imm: &[u32],
        sort: Sort,
        step: u32,
    ) -> Result<String, SvccError> {
        let a = self.r(args[0], step)?;
        let b = match args.get(1) {
            Some(x) => self.r(*x, step)?,
            None => String::new(),
        };
        let bit = |c: String| format!("(ite {c} #b1 #b0)");
        let aw = || self.m.width(args[0]);
        Ok(match op {
            "not" => format!("(bvnot {a})"),
            "neg" => format!("(bvneg {a})"),
            "inc" => format!("(bvadd {a} (_ bv1 {}))", aw()?),
            "dec" => format!("(bvsub {a} (_ bv1 {}))", aw()?),
            "redor" => bit(format!("(distinct {a} (_ bv0 {}))", aw()?)),
            "redand" => bit(format!("(= {a} (bvnot (_ bv0 {})))", aw()?)),
            "redxor" => {
                let w = aw()?;
                let mut acc = format!("((_ extract 0 0) {a})");
                for i in 1..w {
                    acc = format!("(bvxor {acc} ((_ extract {i} {i}) {a}))");
                }
                acc
            }
            "slice" => format!("((_ extract {} {}) {a})", imm[0], imm[1]),
            "uext" => format!("((_ zero_extend {}) {a})", imm[0]),
            "sext" => format!("((_ sign_extend {}) {a})", imm[0]),
            "add" => format!("(bvadd {a} {b})"),
            "sub" => format!("(bvsub {a} {b})"),
            "mul" => format!("(bvmul {a} {b})"),
            // Two-state rule: division and modulo by zero yield 0.
            "udiv" | "sdiv" | "urem" | "srem" | "smod" => {
                let w = aw()?;
                format!("(ite (= {b} (_ bv0 {w})) (_ bv0 {w}) (bv{op} {a} {b}))")
            }
            "and" => format!("(bvand {a} {b})"),
            "or" => format!("(bvor {a} {b})"),
            "xor" => format!("(bvxor {a} {b})"),
            "xnor" => format!("(bvxnor {a} {b})"),
            "nand" => format!("(bvnand {a} {b})"),
            "nor" => format!("(bvnor {a} {b})"),
            "sll" => format!("(bvshl {a} {b})"),
            "srl" => format!("(bvlshr {a} {b})"),
            "sra" => format!("(bvashr {a} {b})"),
            "concat" => format!("(concat {a} {b})"),
            "eq" | "iff" => bit(format!("(= {a} {b})")),
            "neq" => bit(format!("(distinct {a} {b})")),
            "ult" => bit(format!("(bvult {a} {b})")),
            "ulte" => bit(format!("(bvule {a} {b})")),
            "ugt" => bit(format!("(bvugt {a} {b})")),
            "ugte" => bit(format!("(bvuge {a} {b})")),
            "slt" => bit(format!("(bvslt {a} {b})")),
            "slte" => bit(format!("(bvsle {a} {b})")),
            "sgt" => bit(format!("(bvsgt {a} {b})")),
            "sgte" => bit(format!("(bvsge {a} {b})")),
            "implies" => format!("(bvor (bvnot {a}) {b})"),
            "read" => format!("(select {a} {b})"),
            "write" => format!("(store {a} {b} {})", self.r(args[2], step)?),
            "ite" => format!("(ite (= {a} #b1) {b} {})", self.r(args[2], step)?),
            other => {
                return Err(SvccError::new(
                    STAGE,
                    format!("btor2 operator '{other}' of sort {sort:?} not modelled"),
                ))
            }
        })
    }
}

/// Step-0 term of every state that has a constant initial value; error for any other kind.
pub(crate) fn init_terms(m: &Model) -> Result<BTreeMap<u32, String>, SvccError> {
    let mut out = BTreeMap::new();
    for (st, v) in &m.init {
        let term = match m.nodes.get(&(v.unsigned_abs() as u32)) {
            Some((_, Node::Const(lit))) if *v > 0 => lit.clone(),
            Some((_, Node::Const(lit))) => format!("(bvnot {lit})"),
            _ => {
                return Err(SvccError::new(
                    STAGE,
                    format!("state {st} has an initial value that is not a constant"),
                ))
            }
        };
        let term = match m.nodes.get(st).map(|n| n.0) {
            Some(Sort::Arr(i, e)) => {
                format!("((as const (Array (_ BitVec {i}) (_ BitVec {e}))) {term})")
            }
            _ => term,
        };
        out.insert(*st, term);
    }
    Ok(out)
}

pub(crate) struct Query {
    pub(crate) smt: String,
    /// `(name, term)` pairs printed when the query is satisfiable.
    pub(crate) watch: Vec<(String, String)>,
}

#[allow(clippy::too_many_arguments)]
fn build_query(
    spec: &Model,
    imp: &Model,
    outputs: &[String],
    opts: &PairOptions,
    steps: u32,
    spec0: &BTreeMap<u32, String>,
    imp0: &BTreeMap<u32, String>,
    paired_next: &[(u32, u32)],
    decls: &str,
) -> Result<Query, SvccError> {
    let mut smt = String::with_capacity(1 << 16);
    let _ = writeln!(smt, "(set-option :timeout {})", opts.timeout_ms);
    smt.push_str("(set-option :produce-models true)\n");
    let _ = writeln!(
        smt,
        "(set-logic {})",
        if spec.has_arrays || imp.has_arrays {
            "QF_ABV"
        } else {
            "QF_BV"
        }
    );
    smt.push_str(decls);
    let hold: BTreeMap<String, u64> = opts.hold.iter().cloned().collect();

    // Shared primary inputs: the union of both sides' ports, by name.
    let mut ports: BTreeMap<String, Sort> = BTreeMap::new();
    for m in [spec, imp] {
        for (sort, node) in m.nodes.values() {
            if let Node::Input { name } = node {
                if let Some(p) = m.port(name) {
                    match ports.get(p) {
                        Some(s) if s != sort => {
                            return Err(SvccError::new(
                                STAGE,
                                format!("input '{p}' has different widths on the two sides"),
                            ))
                        }
                        _ => {
                            ports.insert(p.to_string(), *sort);
                        }
                    }
                }
            }
        }
    }
    let mut watch = Vec::new();
    let s = Side { m: spec, tag: 's' };
    let i = Side { m: imp, tag: 'i' };
    let mut bad = Vec::new();
    for step in 0..steps {
        for (p, sort) in &ports {
            if !hold.contains_key(p) {
                let _ = writeln!(smt, "(declare-const |in{step}_{p}| {})", sort.smt());
                watch.push((format!("step {step} {p}"), format!("|in{step}_{p}|")));
            }
        }
        s.define_step(&mut smt, step, spec0, &hold)?;
        i.define_step(&mut smt, step, imp0, &hold)?;
        if let Some(f) = &opts.imp_flag {
            let r = imp.output_ref(f).ok_or_else(|| {
                SvccError::new(STAGE, format!("implementation has no output '{f}'"))
            })?;
            let t = i.r(r, step)?;
            bad.push(format!("(distinct {t} (_ bv0 {}))", imp.width(r)?));
            watch.push((format!("step {step} imp  {f}"), t));
        }
        if let Some(f) = &opts.spec_flag {
            let r = spec.output_ref(f).ok_or_else(|| {
                SvccError::new(STAGE, format!("specification has no output '{f}'"))
            })?;
            let t = s.r(r, step)?;
            bad.push(format!("(distinct {t} (_ bv0 {}))", spec.width(r)?));
            watch.push((format!("step {step} spec {f}"), t));
        }
    }
    let compare_steps = if paired_next.is_empty() {
        steps
    } else {
        steps - 1
    };
    for step in 0..compare_steps {
        if step < opts.latency {
            continue;
        }
        for o in outputs {
            let (rs, ri) = (
                spec.output_ref(o).ok_or_else(|| {
                    SvccError::new(STAGE, format!("specification has no output '{o}'"))
                })?,
                imp.output_ref(o).ok_or_else(|| {
                    SvccError::new(STAGE, format!("implementation has no output '{o}'"))
                })?,
            );
            if spec.sort(rs)? != imp.sort(ri)? {
                return Err(SvccError::new(
                    STAGE,
                    format!("output '{o}' has different widths on the two sides"),
                ));
            }
            let (a, b) = (s.r(rs, step - opts.latency)?, i.r(ri, step)?);
            bad.push(format!("(distinct {a} {b})"));
            watch.push((format!("step {} spec {o}", step - opts.latency), a));
            watch.push((format!("step {step} imp  {o}"), b));
        }
    }
    // Induction: the paired states must be equal again after the last compared step.
    for (a, b) in paired_next {
        let last = steps - 1;
        bad.push(format!("(distinct s{last}_{a} i{last}_{b})"));
    }
    if bad.is_empty() {
        smt.push_str("(assert false)\n");
    } else {
        let _ = writeln!(smt, "(assert (or {}))", bad.join("\n  "));
    }
    smt.push_str("(check-sat)\n");
    Ok(Query { smt, watch })
}

/// Runs a query; on `sat` asks for the watched values and returns them as text.
pub(crate) fn solve(q: &Query) -> (SmtVerdict, Vec<String>, u64, usize) {
    let t0 = Instant::now();
    let (v, _) = run_z3_stdin(&q.smt);
    let mut us = t0.elapsed().as_micros() as u64;
    let mut cex = Vec::new();
    if v == SmtVerdict::CounterexampleSat && !q.watch.is_empty() {
        let mut smt2 = q.smt.clone();
        let terms: Vec<&str> = q.watch.iter().map(|w| w.1.as_str()).collect();
        let _ = writeln!(smt2, "(get-value ({}))", terms.join(" "));
        let t1 = Instant::now();
        let (_, text) = run_z3_stdin(&smt2);
        us += t1.elapsed().as_micros() as u64;
        // One `(term value)` pair per watched term, in order.
        let body = text.split_once('\n').map(|x| x.1).unwrap_or("");
        let vals: Vec<&str> = body
            .split('\n')
            .map(|l| l.trim().trim_start_matches('(').trim_end_matches(')'))
            .filter(|l| !l.is_empty())
            .collect();
        for (k, (name, term)) in q.watch.iter().enumerate() {
            if let Some(l) = vals.get(k) {
                let val = l.strip_prefix(term.as_str()).unwrap_or(l).trim();
                if !val.contains("Array") && !val.contains("lambda") && !val.contains("store") {
                    cex.push(format!("{name} = {val}"));
                }
            }
        }
    }
    (v, cex, us, q.smt.len())
}

/// Proves that `imp` computes the same outputs as `spec`. `outputs` are port names present on
/// both sides. See the module documentation for what each verdict means.
pub fn prove_btor2_pair(
    spec_side: &PairSide,
    imp_side: &PairSide,
    outputs: &[String],
    opts: &PairOptions,
) -> Result<PairReport, SvccError> {
    let (spec_top, imp_top) = (spec_side.top, imp_side.top);
    let spec = parse(spec_side)?;
    let imp = parse(imp_side)?;
    if outputs.is_empty() {
        return Err(SvccError::new(STAGE, "no outputs to compare"));
    }
    let mut notes = Vec::new();
    let mut report = PairReport {
        verdict: PairVerdict::Inconclusive,
        spec_top: spec_top.to_string(),
        imp_top: imp_top.to_string(),
        outputs_compared: outputs.len(),
        states_paired: 0,
        bound_steps: 0,
        latency: opts.latency,
        solver_time_us: 0,
        smtlib2_bytes: 0,
        notes: Vec::new(),
    };

    // Pair states by name.
    let ss = spec.states();
    let is = imp.states();
    let imp_by_key: BTreeMap<String, (u32, Sort)> = is
        .iter()
        .filter(|(_, _, n)| !is_flag_state(n))
        .map(|(id, s, n)| (imp.state_key(n), (*id, *s)))
        .collect();
    let mut pairs: Vec<(u32, u32, Sort, String)> = Vec::new();
    let mut unpaired = Vec::new();
    for (id, sort, name) in &ss {
        let key = spec.state_key(name);
        match imp_by_key.get(&key) {
            Some((iid, isort)) if isort == sort => pairs.push((*id, *iid, *sort, key)),
            _ => unpaired.push(format!("spec {key}")),
        }
    }
    for key in imp_by_key.keys() {
        if !pairs.iter().any(|p| p.3 == *key) {
            unpaired.push(format!("imp {key}"));
        }
    }
    report.states_paired = pairs.len();
    let spec_init = init_terms(&spec)?;
    let imp_init = init_terms(&imp)?;

    let can_induct = opts.latency == 0 && unpaired.is_empty();
    if can_induct {
        // Start values that both sides declare must agree, or equal starting states is not
        // what the two designs do.
        let mut init_diff = Vec::new();
        for (a, b, _, key) in &pairs {
            match (spec_init.get(a), imp_init.get(b)) {
                (Some(x), Some(y)) if x != y => init_diff.push(key.clone()),
                (Some(x), None) | (None, Some(x)) if !is_zero_term(x) => {
                    init_diff.push(key.clone())
                }
                _ => {}
            }
        }
        if !init_diff.is_empty() {
            report.verdict = PairVerdict::Mismatch;
            report.notes.push(format!(
                "start values differ for state(s): {}",
                init_diff.join(", ")
            ));
            return Ok(report);
        }
        let mut decls = String::new();
        let mut s0 = BTreeMap::new();
        let mut i0 = BTreeMap::new();
        for (k, (a, b, sort, _)) in pairs.iter().enumerate() {
            let _ = writeln!(decls, "(declare-const st_{k} {})", sort.smt());
            s0.insert(*a, format!("st_{k}"));
            i0.insert(*b, format!("st_{k}"));
        }
        // Flag registers of the implementation start clear.
        let two_phase = ss.iter().any(|(_, _, n)| n.contains("clock_phase"));
        let steps = if two_phase { 3 } else { 2 };
        let next: Vec<(u32, u32)> = pairs.iter().map(|p| (p.0, p.1)).collect();
        // A design without state still needs one compared step: no next-state obligations.
        let q = if next.is_empty() {
            build_query(&spec, &imp, outputs, opts, 2, &s0, &i0, &[], &decls)?
        } else {
            build_query(&spec, &imp, outputs, opts, steps, &s0, &i0, &next, &decls)?
        };
        let (v, cex, us, bytes) = solve(&q);
        report.solver_time_us += us;
        report.smtlib2_bytes = bytes;
        report.bound_steps = steps;
        match v {
            SmtVerdict::EquivalentUnsat => {
                report.verdict = PairVerdict::ProvedInductive;
                report.notes = notes;
                return Ok(report);
            }
            SmtVerdict::CounterexampleSat => {
                if next.is_empty() {
                    report.verdict = PairVerdict::Mismatch;
                    if cex.iter().any(|l| l.contains("svcc__ub = #b1")) {
                        notes.push(
                            "the C++ side executes an operation with undefined behaviour (flag svcc__ub is 1 below)"
                                .to_string(),
                        );
                    }
                    notes.extend(cex);
                    report.notes = notes;
                    return Ok(report);
                }
                notes.push(
                    "induction from an arbitrary common state found a difference; checking from the initial state"
                        .to_string(),
                );
            }
            SmtVerdict::SolverUnavailable => {
                notes.push("induction query: solver gave no answer".to_string());
            }
        }
    } else if opts.latency == 0 {
        notes.push(format!(
            "no induction: states without a partner: {}",
            unpaired.join(", ")
        ));
    }

    if opts.arbitrary_start {
        // Every register of both sides is free at step 0, except the flag registers.
        let mut decls = String::new();
        let mut s0 = BTreeMap::new();
        let mut i0 = BTreeMap::new();
        for (tag, list, map) in [('s', &ss, &mut s0), ('i', &is, &mut i0)] {
            for (id, sort, name) in list.iter() {
                if is_flag_state(name) {
                    continue;
                }
                let _ = writeln!(decls, "(declare-const any_{tag}{id} {})", sort.smt());
                map.insert(*id, format!("any_{tag}{id}"));
            }
        }
        let steps = opts.latency + 2;
        let q = build_query(&spec, &imp, outputs, opts, steps, &s0, &i0, &[], &decls)?;
        let (v, _, us, bytes) = solve(&q);
        report.solver_time_us += us;
        report.smtlib2_bytes = bytes;
        report.bound_steps = steps;
        match v {
            SmtVerdict::EquivalentUnsat => {
                report.verdict = PairVerdict::ProvedInductive;
                notes.push(format!(
                    "holds from an arbitrary state: outputs agree in every step after the first {}",
                    opts.latency
                ));
                report.notes = notes;
                return Ok(report);
            }
            SmtVerdict::CounterexampleSat => notes.push(
                "from an arbitrary state the outputs can differ; checking from the initial state"
                    .to_string(),
            ),
            SmtVerdict::SolverUnavailable => {
                notes.push("arbitrary-state query: solver gave no answer".to_string())
            }
        }
    }

    let steps = opts.bound.max(1) + opts.latency;
    let q = build_query(
        &spec,
        &imp,
        outputs,
        opts,
        steps,
        &spec_init,
        &imp_init,
        &[],
        "",
    )?;
    let (v, cex, us, bytes) = solve(&q);
    report.solver_time_us += us;
    report.smtlib2_bytes = bytes;
    report.bound_steps = steps;
    report.verdict = match v {
        SmtVerdict::EquivalentUnsat => PairVerdict::BoundedNoDifference,
        SmtVerdict::CounterexampleSat => {
            if cex.iter().any(|l| l.contains("svcc__ub = #b1")) {
                notes.push(
                    "the C++ side executes an operation with undefined behaviour (flag svcc__ub is 1 below)"
                        .to_string(),
                );
            }
            notes.extend(cex);
            PairVerdict::Mismatch
        }
        SmtVerdict::SolverUnavailable => {
            notes.push("bounded query: solver gave no answer".to_string());
            PairVerdict::Inconclusive
        }
    };
    report.notes = notes;
    Ok(report)
}

fn is_zero_term(t: &str) -> bool {
    let t = t.trim();
    if let Some(bits) = t.strip_prefix("#b") {
        return bits.chars().all(|c| c == '0');
    }
    t.starts_with("(_ bv0 ")
        || (t.starts_with("((as const") && t.contains("(_ bv0 ") && !t.contains("#b"))
        || (t.starts_with("((as const")
            && t.rsplit("#b")
                .next()
                .is_some_and(|b| b.trim_end_matches(')').chars().all(|c| c == '0')))
}
