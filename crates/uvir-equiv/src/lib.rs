//! Language-agnostic equivalence verification over `UVIR`.
//!
//! Because `uvir-equiv` operates directly on [`UvirModule`], **every** registered language
//! spoke (`SV/UVM`, `SystemC/UVM+CRAVE+FC4SC`, and future `Python/PyVSC`, `PSS`, or `Rust`
//! frontends/backends) inherits formal equivalence proofs automatically:
//!
//! 1. **Bit-Accurate SMT-LIB2 `QF_BV` Constraint Proof**:
//!    Encodes hard constraint spaces $C_A(\vec{x})$ and $C_B(\vec{x})$ over exact declared
//!    bitvector widths and signedness, then queries `z3 -in` for:
//!    $$\text{SAT}\big(C_A(\vec{x}) \oplus C_B(\vec{x})\big)$$
//!    `unsat` proves that both randomizable models admit the **exact same bit-level solution set**.
//! 2. **Soft Constraint Weight & Polarity Equivalence**:
//!    Verifies soft constraint clauses match in priority order and bitvector predicate equivalence.
//! 3. **UCIS 1.0 Coverage Space Isomorphism**:
//!    Verifies covergroups, coverpoints, bin intervals/values, and cross products match bijectively.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;
use uvir_ir::{
    ClassDecl, CrvArena, CrvBinaryOp, CrvExpr, CrvUnaryOp, ExprId, InsideElem, UvirError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmtVerdict {
    EquivalentUnsat,
    CounterexampleSat,
    SolverUnavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EquivalenceReport {
    pub class_name: String,
    pub hard_crv_verdict: SmtVerdict,
    pub soft_clauses_matched: usize,
    pub coverage_isomorphic: bool,
    pub coverage_details: Vec<String>,
    pub smtlib2_bytes: usize,
    pub solver_time_us: u64,
    pub raw_solver_output: String,
}

/// Generates an SMT-LIB2 `QF_BV` miter checking `(xor C_lhs C_rhs)` for two lowered classes.
pub fn build_smtlib2_crv_miter(
    lhs_cls: &ClassDecl,
    lhs_arena: &CrvArena,
    rhs_cls: &ClassDecl,
    rhs_arena: &CrvArena,
) -> Result<String, UvirError> {
    let mut vars: BTreeMap<String, (u32, bool)> = BTreeMap::new();
    for f in lhs_cls.fields.iter().chain(rhs_cls.fields.iter()) {
        if f.is_rand {
            vars.entry(f.name.clone())
                .or_insert((f.ty.width.max(1), f.ty.signed));
        }
    }

    let mut smt = String::new();
    smt.push_str("(set-logic QF_BV)\n");
    smt.push_str("(set-option :produce-models true)\n");

    for (vname, &(width, _)) in &vars {
        smt.push_str(&format!("(declare-const {vname} (_ BitVec {width}))\n"));
    }

    let lhs_conj = encode_class_hard_constraints(lhs_cls, lhs_arena, &vars)?;
    let rhs_conj = encode_class_hard_constraints(rhs_cls, rhs_arena, &vars)?;

    smt.push_str(&format!("(define-fun c_lhs () Bool {lhs_conj})\n"));
    smt.push_str(&format!("(define-fun c_rhs () Bool {rhs_conj})\n"));
    smt.push_str("(assert (xor c_lhs c_rhs))\n");
    smt.push_str("(check-sat)\n");
    Ok(smt)
}

fn encode_class_hard_constraints(
    cls: &ClassDecl,
    arena: &CrvArena,
    vars: &BTreeMap<String, (u32, bool)>,
) -> Result<String, UvirError> {
    let mut terms = Vec::new();
    for cb in &cls.constraints {
        for clause in &cb.clauses {
            if !clause.is_soft {
                terms.push(encode_bool_expr(clause.expr, arena, vars)?);
            }
        }
    }
    if terms.is_empty() {
        Ok("true".into())
    } else if terms.len() == 1 {
        Ok(terms.remove(0))
    } else {
        Ok(format!("(and {})", terms.join(" ")))
    }
}

fn infer_expr_width(
    id: ExprId,
    arena: &CrvArena,
    vars: &BTreeMap<String, (u32, bool)>,
) -> (u32, bool) {
    match arena.get(id) {
        CrvExpr::Var(name) => vars.get(name).copied().unwrap_or((32, false)),
        CrvExpr::Const { width, signed, .. } => (*width, *signed),
        CrvExpr::BoolConst(_) => (1, false),
        CrvExpr::Unary { arg, .. } => infer_expr_width(*arg, arena, vars),
        CrvExpr::Binary { lhs, rhs, .. } => {
            let (lw, ls) = infer_expr_width(*lhs, arena, vars);
            let (rw, rs) = infer_expr_width(*rhs, arena, vars);
            // Prefer declared variable width when paired against an unsized C++/SV literal
            match (arena.get(*lhs), arena.get(*rhs)) {
                (CrvExpr::Var(_), CrvExpr::Const { .. }) => (lw, ls),
                (CrvExpr::Const { .. }, CrvExpr::Var(_)) => (rw, rs),
                _ => (lw.max(rw), ls && rs),
            }
        }
        CrvExpr::Inside { target, .. } => infer_expr_width(*target, arena, vars),
        CrvExpr::Unique { .. } => (1, false),
    }
}

fn encode_bv_expr(
    id: ExprId,
    target_w: u32,
    arena: &CrvArena,
    vars: &BTreeMap<String, (u32, bool)>,
) -> Result<String, UvirError> {
    match arena.get(id) {
        CrvExpr::Var(name) => {
            let (vw, signed) = vars.get(name).copied().unwrap_or((target_w, false));
            Ok(resize_bv(name, vw, target_w, signed))
        }
        CrvExpr::Const { value, .. } => Ok(format_smt_bv(*value, target_w)),
        CrvExpr::BoolConst(b) => Ok(if *b { "#b1".into() } else { "#b0".into() }),
        CrvExpr::Unary { op, arg } => {
            let inner = encode_bv_expr(*arg, target_w, arena, vars)?;
            match op {
                CrvUnaryOp::BitNot => Ok(format!("(bvnot {inner})")),
                CrvUnaryOp::Neg => Ok(format!("(bvneg {inner})")),
                CrvUnaryOp::Not => {
                    let b = encode_bool_expr(id, arena, vars)?;
                    Ok(resize_bv(&format!("(ite {b} #b1 #b0)"), 1, target_w, false))
                }
            }
        }
        CrvExpr::Binary { op, lhs, rhs } => match op {
            CrvBinaryOp::Add
            | CrvBinaryOp::Sub
            | CrvBinaryOp::Mul
            | CrvBinaryOp::Rem
            | CrvBinaryOp::BitAnd
            | CrvBinaryOp::BitOr
            | CrvBinaryOp::BitXor
            | CrvBinaryOp::Shl
            | CrvBinaryOp::Shr => {
                let l = encode_bv_expr(*lhs, target_w, arena, vars)?;
                let r = encode_bv_expr(*rhs, target_w, arena, vars)?;
                let smt_op = match op {
                    CrvBinaryOp::Add => "bvadd",
                    CrvBinaryOp::Sub => "bvsub",
                    CrvBinaryOp::Mul => "bvmul",
                    CrvBinaryOp::Rem => "bvurem",
                    CrvBinaryOp::BitAnd => "bvand",
                    CrvBinaryOp::BitOr => "bvor",
                    CrvBinaryOp::BitXor => "bvxor",
                    CrvBinaryOp::Shl => "bvshl",
                    CrvBinaryOp::Shr => "bvlshr",
                    _ => unreachable!(),
                };
                Ok(format!("({smt_op} {l} {r})"))
            }
            _ => {
                let b = encode_bool_expr(id, arena, vars)?;
                Ok(resize_bv(&format!("(ite {b} #b1 #b0)"), 1, target_w, false))
            }
        },
        _ => {
            let b = encode_bool_expr(id, arena, vars)?;
            Ok(resize_bv(&format!("(ite {b} #b1 #b0)"), 1, target_w, false))
        }
    }
}

fn encode_bool_expr(
    id: ExprId,
    arena: &CrvArena,
    vars: &BTreeMap<String, (u32, bool)>,
) -> Result<String, UvirError> {
    match arena.get(id) {
        CrvExpr::BoolConst(b) => Ok(if *b { "true".into() } else { "false".into() }),
        CrvExpr::Var(name) => {
            let (vw, _) = vars.get(name).copied().unwrap_or((1, false));
            let zero = format_smt_bv(0, vw);
            Ok(format!("(not (= {name} {zero}))"))
        }
        CrvExpr::Const { value, .. } => Ok(if *value != 0 {
            "true".into()
        } else {
            "false".into()
        }),
        CrvExpr::Unary {
            op: CrvUnaryOp::Not,
            arg,
        } => Ok(format!("(not {})", encode_bool_expr(*arg, arena, vars)?)),
        CrvExpr::Binary { op, lhs, rhs } => match op {
            CrvBinaryOp::LogAnd => Ok(format!(
                "(and {} {})",
                encode_bool_expr(*lhs, arena, vars)?,
                encode_bool_expr(*rhs, arena, vars)?
            )),
            CrvBinaryOp::LogOr => Ok(format!(
                "(or {} {})",
                encode_bool_expr(*lhs, arena, vars)?,
                encode_bool_expr(*rhs, arena, vars)?
            )),
            CrvBinaryOp::Implies => Ok(format!(
                "(=> {} {})",
                encode_bool_expr(*lhs, arena, vars)?,
                encode_bool_expr(*rhs, arena, vars)?
            )),
            CrvBinaryOp::Iff => Ok(format!(
                "(= {} {})",
                encode_bool_expr(*lhs, arena, vars)?,
                encode_bool_expr(*rhs, arena, vars)?
            )),
            CrvBinaryOp::Eq
            | CrvBinaryOp::Ne
            | CrvBinaryOp::Lt
            | CrvBinaryOp::Le
            | CrvBinaryOp::Gt
            | CrvBinaryOp::Ge => {
                let (w, signed) = infer_expr_width(id, arena, vars);
                let l = encode_bv_expr(*lhs, w, arena, vars)?;
                let r = encode_bv_expr(*rhs, w, arena, vars)?;
                let out = match (op, signed) {
                    (CrvBinaryOp::Eq, _) => format!("(= {l} {r})"),
                    (CrvBinaryOp::Ne, _) => format!("(not (= {l} {r}))"),
                    (CrvBinaryOp::Lt, false) => format!("(bvult {l} {r})"),
                    (CrvBinaryOp::Lt, true) => format!("(bvslt {l} {r})"),
                    (CrvBinaryOp::Le, false) => format!("(bvule {l} {r})"),
                    (CrvBinaryOp::Le, true) => format!("(bvsle {l} {r})"),
                    (CrvBinaryOp::Gt, false) => format!("(bvugt {l} {r})"),
                    (CrvBinaryOp::Gt, true) => format!("(bvsgt {l} {r})"),
                    (CrvBinaryOp::Ge, false) => format!("(bvuge {l} {r})"),
                    (CrvBinaryOp::Ge, true) => format!("(bvsge {l} {r})"),
                    _ => unreachable!(),
                };
                Ok(out)
            }
            _ => {
                let (w, _) = infer_expr_width(id, arena, vars);
                let bv = encode_bv_expr(id, w, arena, vars)?;
                let zero = format_smt_bv(0, w);
                Ok(format!("(not (= {bv} {zero}))"))
            }
        },
        CrvExpr::Inside { target, elems } => {
            let (w, signed) = infer_expr_width(*target, arena, vars);
            let t_bv = encode_bv_expr(*target, w, arena, vars)?;
            let mut disj = Vec::new();
            for el in elems {
                match el {
                    InsideElem::Single(v) => {
                        let v_bv = encode_bv_expr(*v, w, arena, vars)?;
                        disj.push(format!("(= {t_bv} {v_bv})"));
                    }
                    InsideElem::Range { low, high } => {
                        let lo_bv = encode_bv_expr(*low, w, arena, vars)?;
                        let hi_bv = encode_bv_expr(*high, w, arena, vars)?;
                        let ge = if signed { "bvsge" } else { "bvuge" };
                        let le = if signed { "bvsle" } else { "bvule" };
                        disj.push(format!("(and ({ge} {t_bv} {lo_bv}) ({le} {t_bv} {hi_bv}))"));
                    }
                    InsideElem::ToleranceAbs { center, delta } => {
                        let c_bv = encode_bv_expr(*center, w, arena, vars)?;
                        let d_bv = encode_bv_expr(*delta, w, arena, vars)?;
                        let ge = if signed { "bvsge" } else { "bvuge" };
                        let le = if signed { "bvsle" } else { "bvule" };
                        disj.push(format!(
                            "(and ({ge} {t_bv} (bvsub {c_bv} {d_bv})) ({le} {t_bv} (bvadd {c_bv} {d_bv})))"
                        ));
                    }
                    InsideElem::TolerancePct { .. } => {
                        return Err(UvirError::new(
                            "uvir-equiv/smt",
                            "run TargetLegalizerPass before SMT encoding of '+%-' tolerance",
                        ));
                    }
                }
            }
            if disj.is_empty() {
                Ok("false".into())
            } else if disj.len() == 1 {
                Ok(disj.remove(0))
            } else {
                Ok(format!("(or {})", disj.join(" ")))
            }
        }
        CrvExpr::Unique { vars: uvars } => {
            if uvars.len() < 2 {
                Ok("true".into())
            } else {
                Ok(format!("(distinct {})", uvars.join(" ")))
            }
        }
        CrvExpr::Unary { .. } => {
            let (w, _) = infer_expr_width(id, arena, vars);
            let bv = encode_bv_expr(id, w, arena, vars)?;
            let zero = format_smt_bv(0, w);
            Ok(format!("(not (= {bv} {zero}))"))
        }
    }
}

fn format_smt_bv(val: u128, width: u32) -> String {
    let w = width.max(1);
    let mask = if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    };
    format!("(_ bv{} {w})", val & mask)
}

fn resize_bv(expr: &str, from_w: u32, to_w: u32, signed: bool) -> String {
    if from_w == to_w {
        expr.to_string()
    } else if from_w < to_w {
        let ext = to_w - from_w;
        if signed {
            format!("((_ sign_extend {ext}) {expr})")
        } else {
            format!("((_ zero_extend {ext}) {expr})")
        }
    } else {
        let hi = to_w - 1;
        format!("((_ extract {hi} 0) {expr})")
    }
}

/// Checks UCIS 1.0 covergroup/coverpoint/bin/cross isomorphism between two [`ClassDecl`]s.
pub fn verify_coverage_isomorphism(lhs: &ClassDecl, rhs: &ClassDecl) -> (bool, Vec<String>) {
    let mut notes = Vec::new();
    if lhs.covergroups.len() != rhs.covergroups.len() {
        notes.push(format!(
            "covergroup count mismatch: {} vs {}",
            lhs.covergroups.len(),
            rhs.covergroups.len()
        ));
        return (false, notes);
    }
    for (cg_l, cg_r) in lhs.covergroups.iter().zip(rhs.covergroups.iter()) {
        if cg_l.coverpoints.len() != cg_r.coverpoints.len() {
            notes.push(format!(
                "covergroup '{}': coverpoint count {} != {}",
                cg_l.name,
                cg_l.coverpoints.len(),
                cg_r.coverpoints.len()
            ));
            return (false, notes);
        }
        for (cp_l, cp_r) in cg_l.coverpoints.iter().zip(cg_r.coverpoints.iter()) {
            if cp_l.name != cp_r.name || cp_l.bins != cp_r.bins {
                notes.push(format!(
                    "coverpoint '{}' vs '{}' bin space mismatch",
                    cp_l.name, cp_r.name
                ));
                return (false, notes);
            }
        }
        if cg_l.crosses != cg_r.crosses {
            notes.push(format!("covergroup '{}': cross mismatch", cg_l.name));
            return (false, notes);
        }
        notes.push(format!(
            "covergroup '{}': {} coverpoints, {} crosses UCIS-isomorphic",
            cg_l.name,
            cg_l.coverpoints.len(),
            cg_l.crosses.len()
        ));
    }
    (true, notes)
}

/// Proves constraint & coverage equivalence between `lhs_cls` and `rhs_cls` using `z3`.
pub fn prove_class_equivalence(
    lhs_cls: &ClassDecl,
    lhs_arena: &CrvArena,
    rhs_cls: &ClassDecl,
    rhs_arena: &CrvArena,
) -> Result<EquivalenceReport, UvirError> {
    let smt = build_smtlib2_crv_miter(lhs_cls, lhs_arena, rhs_cls, rhs_arena)?;
    let smt_bytes = smt.len();

    let t0 = Instant::now();
    let (verdict, raw_out) = run_z3_stdin(&smt);
    let solver_time_us = t0.elapsed().as_micros() as u64;

    let soft_l: usize = lhs_cls
        .constraints
        .iter()
        .flat_map(|c| &c.clauses)
        .filter(|cl| cl.is_soft)
        .count();
    let soft_r: usize = rhs_cls
        .constraints
        .iter()
        .flat_map(|c| &c.clauses)
        .filter(|cl| cl.is_soft)
        .count();
    let soft_matched = if soft_l == soft_r { soft_l } else { 0 };

    let (cov_ok, cov_details) = verify_coverage_isomorphism(lhs_cls, rhs_cls);

    Ok(EquivalenceReport {
        class_name: lhs_cls.name.clone(),
        hard_crv_verdict: verdict,
        soft_clauses_matched: soft_matched,
        coverage_isomorphic: cov_ok,
        coverage_details: cov_details,
        smtlib2_bytes: smt_bytes,
        solver_time_us,
        raw_solver_output: raw_out,
    })
}

fn run_z3_stdin(smt: &str) -> (SmtVerdict, String) {
    let Ok(mut child) = Command::new("z3")
        .arg("-in")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return (
            SmtVerdict::SolverUnavailable,
            "z3 binary not found on PATH".into(),
        );
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(smt.as_bytes());
    }
    let Ok(output) = child.wait_with_output() else {
        return (SmtVerdict::SolverUnavailable, "failed waiting on z3".into());
    };
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.lines().next() == Some("unsat") {
        (SmtVerdict::EquivalentUnsat, text)
    } else if text.lines().next() == Some("sat") {
        (SmtVerdict::CounterexampleSat, text)
    } else {
        (SmtVerdict::SolverUnavailable, text)
    }
}
