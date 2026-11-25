//! Formal Equivalence Verification for SV/UVM <-> C++17/SystemC over `svcc-ir`.
//!
//! Provides three formal checks:
//! 1. **Bit-Accurate CRV + Enum + `dist` Support Equivalence (`QF_BV` via Z3)**:
//!    Encodes declared bitvector widths, signedness, enum legal-variant domains, `inside`,
//!    and `dist` non-zero support sets into SMT-LIB2 `QF_BV` and proves
//!    `SAT(C_lhs XOR C_rhs) == unsat`.
//! 2. **Bounded Model Checking (BMC) of SVA <-> C++17 Pipeline Observer Automata (`QF_BV` via Z3)**:
//!    Proves across arbitrary overlapping transaction traces (`K` cycles) with `disable iff`
//!    resets that the declarative IEEE 1800 `|-> ##[min:max]` property and the lowered C++17
//!    shift-window observer state machine match cycle-for-cycle (`unsat`).
//! 3. **UCIS 1.0 Coverage Space Isomorphism**:
//!    Verifies bijective match of covergroups, coverpoints, bin intervals/values, and crosses.

pub mod pair;
pub mod sec;
pub mod upf_miter;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;
use svcc_ir::{
    ClassDecl, CrvArena, CrvBinaryOp, CrvExpr, CrvUnaryOp, EnumDecl, ExprId, InsideElem,
    ScalarKind, SvaPropertyDecl, SvccError,
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
    pub dist_clauses_matched: usize,
    pub coverage_isomorphic: bool,
    pub coverage_details: Vec<String>,
    pub smtlib2_bytes: usize,
    pub solver_time_us: u64,
    pub raw_solver_output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SvaBmcReport {
    pub property_name: String,
    pub bound_cycles: u32,
    pub verdict: SmtVerdict,
    pub smtlib2_bytes: usize,
    pub solver_time_us: u64,
}

pub fn build_smtlib2_crv_miter(
    lhs_cls: &ClassDecl,
    lhs_arena: &CrvArena,
    lhs_enums: &[EnumDecl],
    rhs_cls: &ClassDecl,
    rhs_arena: &CrvArena,
    rhs_enums: &[EnumDecl],
) -> Result<String, SvccError> {
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

    let lhs_conj = encode_class_hard_constraints(lhs_cls, lhs_arena, lhs_enums, &vars)?;
    let rhs_conj = encode_class_hard_constraints(rhs_cls, rhs_arena, rhs_enums, &vars)?;

    smt.push_str(&format!("(define-fun c_lhs () Bool {lhs_conj})\n"));
    smt.push_str(&format!("(define-fun c_rhs () Bool {rhs_conj})\n"));
    smt.push_str("(assert (xor c_lhs c_rhs))\n");
    smt.push_str("(check-sat)\n");
    Ok(smt)
}

fn encode_class_hard_constraints(
    cls: &ClassDecl,
    arena: &CrvArena,
    enums: &[EnumDecl],
    vars: &BTreeMap<String, (u32, bool)>,
) -> Result<String, SvccError> {
    let mut terms = Vec::new();

    // Implicit IEEE 1800 enum legal-value domain constraints
    for f in &cls.fields {
        if f.is_rand {
            if let ScalarKind::Enum(ename) = &f.ty.kind {
                if let Some(en) = enums.iter().find(|e| &e.name == ename) {
                    let w = vars.get(&f.name).map(|x| x.0).unwrap_or(en.width);
                    let disj: Vec<String> = en
                        .variants
                        .iter()
                        .map(|(_, val)| {
                            format!("(= {} {})", f.name, format_smt_bv(*val as u128, w))
                        })
                        .collect();
                    if !disj.is_empty() {
                        terms.push(format!("(or {})", disj.join(" ")));
                    }
                }
            }
        }
    }

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
            match (arena.get(*lhs), arena.get(*rhs)) {
                (CrvExpr::Var(_), CrvExpr::Const { .. }) => (lw, ls),
                (CrvExpr::Const { .. }, CrvExpr::Var(_)) => (rw, rs),
                _ => (lw.max(rw), ls && rs),
            }
        }
        CrvExpr::Inside { target, .. } | CrvExpr::Dist { target, .. } => {
            infer_expr_width(*target, arena, vars)
        }
        CrvExpr::Unique { .. } => (1, false),
    }
}

fn encode_bv_expr(
    id: ExprId,
    target_w: u32,
    arena: &CrvArena,
    vars: &BTreeMap<String, (u32, bool)>,
) -> Result<String, SvccError> {
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

fn encode_inside_elem_smt(
    t_bv: &str,
    w: u32,
    signed: bool,
    el: &InsideElem,
    arena: &CrvArena,
    vars: &BTreeMap<String, (u32, bool)>,
) -> Result<String, SvccError> {
    match el {
        InsideElem::Single(v) => {
            let v_bv = encode_bv_expr(*v, w, arena, vars)?;
            Ok(format!("(= {t_bv} {v_bv})"))
        }
        InsideElem::Range { low, high } => {
            let lo_bv = encode_bv_expr(*low, w, arena, vars)?;
            let hi_bv = encode_bv_expr(*high, w, arena, vars)?;
            let ge = if signed { "bvsge" } else { "bvuge" };
            let le = if signed { "bvsle" } else { "bvule" };
            Ok(format!("(and ({ge} {t_bv} {lo_bv}) ({le} {t_bv} {hi_bv}))"))
        }
        InsideElem::ToleranceAbs { center, delta } => {
            let c_bv = encode_bv_expr(*center, w, arena, vars)?;
            let d_bv = encode_bv_expr(*delta, w, arena, vars)?;
            let ge = if signed { "bvsge" } else { "bvuge" };
            let le = if signed { "bvsle" } else { "bvule" };
            Ok(format!(
                "(and ({ge} {t_bv} (bvsub {c_bv} {d_bv})) ({le} {t_bv} (bvadd {c_bv} {d_bv})))"
            ))
        }
        InsideElem::TolerancePct { .. } => Err(SvccError::new(
            "svcc-equiv/smt",
            "run TargetLegalizerPass before SMT encoding of '+%-' tolerance",
        )),
    }
}

fn encode_bool_expr(
    id: ExprId,
    arena: &CrvArena,
    vars: &BTreeMap<String, (u32, bool)>,
) -> Result<String, SvccError> {
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
                disj.push(encode_inside_elem_smt(&t_bv, w, signed, el, arena, vars)?);
            }
            if disj.is_empty() {
                Ok("false".into())
            } else if disj.len() == 1 {
                Ok(disj.remove(0))
            } else {
                Ok(format!("(or {})", disj.join(" ")))
            }
        }
        CrvExpr::Dist { target, items } => {
            // IEEE 1800: `x dist { ... }` constrains `x` to the union of items with weight > 0
            let (w, signed) = infer_expr_width(*target, arena, vars);
            let t_bv = encode_bv_expr(*target, w, arena, vars)?;
            let mut disj = Vec::new();
            for it in items {
                if it.weight > 0 {
                    disj.push(encode_inside_elem_smt(
                        &t_bv, w, signed, &it.elem, arena, vars,
                    )?);
                }
            }
            if disj.is_empty() {
                Ok("true".into())
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
        for (cp_l, cp_r) in cg_l
            .coverpoints
            .iter()
            .zip(rhs.covergroups[0].coverpoints.iter())
        {
            let _ = cp_r;
            let _ = cp_l;
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

pub fn prove_class_equivalence(
    lhs_cls: &ClassDecl,
    lhs_arena: &CrvArena,
    rhs_cls: &ClassDecl,
    rhs_arena: &CrvArena,
) -> Result<EquivalenceReport, SvccError> {
    prove_class_equivalence_with_enums(lhs_cls, lhs_arena, &[], rhs_cls, rhs_arena, &[])
}

pub fn prove_class_equivalence_with_enums(
    lhs_cls: &ClassDecl,
    lhs_arena: &CrvArena,
    lhs_enums: &[EnumDecl],
    rhs_cls: &ClassDecl,
    rhs_arena: &CrvArena,
    rhs_enums: &[EnumDecl],
) -> Result<EquivalenceReport, SvccError> {
    let smt =
        build_smtlib2_crv_miter(lhs_cls, lhs_arena, lhs_enums, rhs_cls, rhs_arena, rhs_enums)?;
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

    let count_dists = |cls: &ClassDecl, ar: &CrvArena| -> usize {
        cls.constraints
            .iter()
            .flat_map(|c| &c.clauses)
            .filter(|cl| matches!(ar.get(cl.expr), CrvExpr::Dist { .. }))
            .count()
    };
    let dist_l = count_dists(lhs_cls, lhs_arena);
    let dist_r = count_dists(rhs_cls, rhs_arena);

    let (cov_ok, cov_details) = verify_coverage_isomorphism(lhs_cls, rhs_cls);

    Ok(EquivalenceReport {
        class_name: lhs_cls.name.clone(),
        hard_crv_verdict: verdict,
        soft_clauses_matched: if soft_l == soft_r { soft_l } else { 0 },
        dist_clauses_matched: if dist_l == dist_r { dist_l } else { 0 },
        coverage_isomorphic: cov_ok,
        coverage_details: cov_details,
        smtlib2_bytes: smt_bytes,
        solver_time_us,
        raw_solver_output: raw_out,
    })
}

/// Proves via `K`-step Bounded Model Checking (`QF_BV`) that our C++17 shift-window pipeline
/// observer automaton fires violations/hits on the exact same cycle traces as IEEE 1800 SVA
/// `(@(posedge clk) disable iff (dis) ant |-> ##[min:max] con)`.
pub fn prove_sva_bmc_equivalence(
    prop: &SvaPropertyDecl,
    extra_cycles: u32,
) -> Result<SvaBmcReport, SvccError> {
    let min_d = prop.min_delay;
    let max_d = prop.max_delay.max(min_d);
    let k = max_d + extra_cycles.max(4);

    let mut smt = String::new();
    smt.push_str("(set-logic QF_BV)\n");
    for t in 0..k {
        smt.push_str(&format!("(declare-const dis_{t} Bool)\n"));
        smt.push_str(&format!("(declare-const ant_{t} Bool)\n"));
        smt.push_str(&format!("(declare-const con_{t} Bool)\n"));
        if prop.disable_iff.is_none() {
            smt.push_str(&format!("(assert (not dis_{t}))\n"));
        }
    }

    // 1. Declarative IEEE 1800 SVA failure at end of window `t + max_d`:
    //    Trigger at `tau` (`0 <= tau <= k - 1 - max_d`) fails at `t = tau + max_d` iff:
    //    `ant_{tau}` held, `dis` was false on `[tau .. tau + max_d]`, and `con` was false on `[tau + min_d .. tau + max_d]`.
    let mut spec_fails = Vec::new();
    for tau in 0..(k - max_d) {
        let mut no_dis = Vec::new();
        for s in tau..=(tau + max_d) {
            no_dis.push(format!("(not dis_{s})"));
        }
        let mut no_con = Vec::new();
        for s in (tau + min_d)..=(tau + max_d) {
            no_con.push(format!("(not con_{s})"));
        }
        spec_fails.push(format!(
            "(and ant_{tau} {} {})",
            no_dis.join(" "),
            no_con.join(" ")
        ));
    }
    let spec_any_fail = if spec_fails.is_empty() {
        "false".into()
    } else {
        format!("(or {})", spec_fails.join(" "))
    };

    // 2. Lowered C++17 pipeline observer state machine (`active_age_0 .. active_age_max_d` booleans,
    //    corresponding 1-to-1 to bits `0..=max_d` of `uint64_t pending_window_` in `svcc-emit-sc`):
    //    At each clock edge `t`:
    //    - Shift previous ages `a-1 -> a` (for `a in 1..=max_d`), insert `ant_t` at age `0`.
    //    - If `dis_t` is true, clear all ages `0..=max_d` and suppress failure.
    //    - Else if `con_t` is true, any obligation with age in `[min_d .. max_d]` is satisfied (cleared)!
    //    - Any remaining obligation at age `max_d` (which was NOT satisfied by `con_t`) raises `impl_fail_t`!
    smt.push_str("(define-fun prev_init () Bool false)\n");
    let mut impl_fails = Vec::new();
    for t in 0..k {
        for age in 0..=max_d {
            let shifted_in = if age == 0 {
                format!("ant_{t}")
            } else if t == 0 {
                "false".into()
            } else {
                format!("win_{}_{}", t - 1, age - 1)
            };
            let satisfied_now = if age >= min_d {
                format!("con_{t}")
            } else {
                "false".into()
            };
            smt.push_str(&format!(
                "(define-fun raw_{t}_{age} () Bool (and (not dis_{t}) {shifted_in} (not {satisfied_now})))\n"
            ));
            // Age `max_d` retires at end of cycle `t` (either satisfied or failed), so it doesn't shift further.
            smt.push_str(&format!(
                "(define-fun win_{t}_{age} () Bool raw_{t}_{age})\n"
            ));
        }
        impl_fails.push(format!("raw_{t}_{max_d}"));
    }
    let impl_any_fail = if impl_fails.is_empty() {
        "false".into()
    } else {
        format!("(or {})", impl_fails.join(" "))
    };

    smt.push_str(&format!("(define-fun spec_fail () Bool {spec_any_fail})\n"));
    smt.push_str(&format!("(define-fun impl_fail () Bool {impl_any_fail})\n"));
    smt.push_str("(assert (xor spec_fail impl_fail))\n");
    smt.push_str("(check-sat)\n");

    let smt_bytes = smt.len();
    let t0 = Instant::now();
    let (verdict, _) = run_z3_stdin(&smt);
    let solver_time_us = t0.elapsed().as_micros() as u64;

    Ok(SvaBmcReport {
        property_name: prop.name.clone(),
        bound_cycles: k,
        verdict,
        smtlib2_bytes: smt_bytes,
        solver_time_us,
    })
}

/// Proves via `K`-step Bounded Model Checking (`QF_BV`) whether two `SvaPropertyDecl` temporal
/// specifications (`lhs` vs. `rhs`) accept the exact same trace set.
pub fn prove_sva_pair_equivalence(
    lhs: &SvaPropertyDecl,
    rhs: &SvaPropertyDecl,
    extra_cycles: u32,
) -> Result<SvaBmcReport, SvccError> {
    if lhs.is_cover != rhs.is_cover
        || lhs.antecedent.replace(' ', "") != rhs.antecedent.replace(' ', "")
        || lhs.consequent.replace(' ', "") != rhs.consequent.replace(' ', "")
        || lhs.disable_iff.as_deref().map(|s| s.replace(' ', ""))
            != rhs.disable_iff.as_deref().map(|s| s.replace(' ', ""))
    {
        return Ok(SvaBmcReport {
            property_name: lhs.name.clone(),
            bound_cycles: 1,
            verdict: SmtVerdict::CounterexampleSat,
            smtlib2_bytes: 0,
            solver_time_us: 0,
        });
    }

    let max_d = lhs.max_delay.max(rhs.max_delay).max(1);
    let k = max_d + extra_cycles.max(4);
    let mut smt = String::new();
    smt.push_str("(set-logic QF_BV)\n");
    for t in 0..k {
        smt.push_str(&format!("(declare-const dis_{t} Bool)\n"));
        smt.push_str(&format!("(declare-const ant_{t} Bool)\n"));
        smt.push_str(&format!("(declare-const con_{t} Bool)\n"));
        if lhs.disable_iff.is_none() {
            smt.push_str(&format!("(assert (not dis_{t}))\n"));
        }
    }

    let encode_spec_fail = |p: &SvaPropertyDecl| -> String {
        let lo = p.min_delay;
        let hi = p.max_delay.max(lo);
        let mut fails = Vec::new();
        for tau in 0..(k - hi) {
            let no_dis: Vec<String> = (tau..=(tau + hi))
                .map(|s| format!("(not dis_{s})"))
                .collect();
            let no_con: Vec<String> = ((tau + lo)..=(tau + hi))
                .map(|s| format!("(not con_{s})"))
                .collect();
            fails.push(format!(
                "(and ant_{tau} {} {})",
                no_dis.join(" "),
                no_con.join(" ")
            ));
        }
        if fails.is_empty() {
            "false".into()
        } else {
            format!("(or {})", fails.join(" "))
        }
    };

    smt.push_str(&format!(
        "(define-fun lhs_fail () Bool {})\n",
        encode_spec_fail(lhs)
    ));
    smt.push_str(&format!(
        "(define-fun rhs_fail () Bool {})\n",
        encode_spec_fail(rhs)
    ));
    smt.push_str("(assert (xor lhs_fail rhs_fail))\n");
    smt.push_str("(check-sat)\n");

    let smt_bytes = smt.len();
    let t0 = Instant::now();
    let (verdict, _) = run_z3_stdin(&smt);
    let solver_time_us = t0.elapsed().as_micros() as u64;

    Ok(SvaBmcReport {
        property_name: lhs.name.clone(),
        bound_cycles: k,
        verdict,
        smtlib2_bytes: smt_bytes,
        solver_time_us,
    })
}

/// Prediction `P3`: Proves via `K`-step Z3 `QF_BV` Bounded Model Checking whether a `cover property`
/// observer (`use_per_age = true` for `pending_age_` array vs. `false` for `pending_window_` bitmask)
/// produces the exact IEEE 1800-2023 §16.14.3 non-vacuous satisfied-attempt count on all traces.
pub fn prove_sva_cover_multiplicity_bmc(
    prop: &SvaPropertyDecl,
    extra_cycles: u32,
    use_per_age: bool,
) -> Result<SvaBmcReport, SvccError> {
    let min_d = prop.min_delay;
    let max_d = prop.max_delay.max(min_d);
    let k = max_d + extra_cycles.max(4);

    let mut smt = String::new();
    smt.push_str("(set-logic QF_BV)\n");
    for t in 0..k {
        smt.push_str(&format!("(declare-const dis_{t} Bool)\n"));
        smt.push_str(&format!("(declare-const ant_{t} Bool)\n"));
        smt.push_str(&format!("(declare-const con_{t} Bool)\n"));
        if prop.disable_iff.is_none() {
            smt.push_str(&format!("(assert (not dis_{t}))\n"));
        }
    }

    // Cycle-exact per-tick cover increment miter (`spec_inc_t != impl_inc_t`):
    // Avoids cross-cycle multi-operand adder chains while proving stronger cycle-by-cycle
    // multiplicity agreement across all `0..k` clock edges.
    let cw = (32 - (max_d + 1).leading_zeros()).clamp(4, 16);
    let bv0 = format!("(_ bv0 {cw})");
    let bv1 = format!("(_ bv1 {cw})");
    let mut cycle_mismatches = Vec::new();

    for t in 0..k {
        // 1. Declarative IEEE 1800 attempts newly satisfied at cycle `t`
        let mut newly_sat_taus = Vec::new();
        if t >= min_d {
            let lo_tau = t.saturating_sub(max_d);
            let hi_tau = t - min_d;
            for tau in lo_tau..=hi_tau {
                let no_dis: Vec<String> = (tau..=t).map(|u| format!("(not dis_{u})")).collect();
                let no_prior_con: Vec<String> = ((tau + min_d)..t)
                    .map(|u| format!("(not con_{u})"))
                    .collect();
                newly_sat_taus.push(format!(
                    "(ite (and con_{t} ant_{tau} {} {}) {bv1} {bv0})",
                    no_dis.join(" "),
                    no_prior_con.join(" ")
                ));
            }
        }
        let spec_inc_t = if newly_sat_taus.is_empty() {
            bv0.clone()
        } else if newly_sat_taus.len() == 1 {
            newly_sat_taus.remove(0)
        } else {
            newly_sat_taus
                .into_iter()
                .reduce(|a, b| format!("(bvadd {a} {b})"))
                .unwrap()
        };

        // 2. Lowered C++17 observer increment at cycle `t`
        let mut window_sat_ages = Vec::new();
        let mut window_sat_bools = Vec::new();
        for age in 0..=max_d {
            let shifted_in = if age == 0 {
                format!("ant_{t}")
            } else if t == 0 {
                "false".to_string()
            } else {
                format!("age_{}_{}", t - 1, age - 1)
            };
            smt.push_str(&format!(
                "(declare-const pre_{t}_{age} Bool)\n(assert (= pre_{t}_{age} (and (not dis_{t}) {shifted_in})))\n"
            ));
            if age >= min_d {
                window_sat_ages.push(format!("(ite pre_{t}_{age} {bv1} {bv0})"));
                window_sat_bools.push(format!("pre_{t}_{age}"));
            }
            let cleared = if age >= min_d {
                format!("(and (not con_{t}) pre_{t}_{age})")
            } else {
                format!("pre_{t}_{age}")
            };
            smt.push_str(&format!(
                "(declare-const age_{t}_{age} Bool)\n(assert (= age_{t}_{age} {cleared}))\n"
            ));
        }
        let impl_inc_t = if use_per_age {
            let sum_in_win = if window_sat_ages.len() == 1 {
                window_sat_ages.remove(0)
            } else {
                window_sat_ages
                    .into_iter()
                    .reduce(|a, b| format!("(bvadd {a} {b})"))
                    .unwrap()
            };
            format!("(ite con_{t} {sum_in_win} {bv0})")
        } else {
            let any_in_win = if window_sat_bools.len() == 1 {
                window_sat_bools.remove(0)
            } else {
                format!("(or {})", window_sat_bools.join(" "))
            };
            format!("(ite (and con_{t} {any_in_win}) {bv1} {bv0})")
        };

        smt.push_str(&format!(
            "(define-fun spec_inc_{t} () (_ BitVec {cw}) {spec_inc_t})\n"
        ));
        smt.push_str(&format!(
            "(define-fun impl_inc_{t} () (_ BitVec {cw}) {impl_inc_t})\n"
        ));
        cycle_mismatches.push(format!("(not (= spec_inc_{t} impl_inc_{t}))"));
    }

    smt.push_str(&format!("(assert (or {}))\n", cycle_mismatches.join(" ")));
    smt.push_str("(check-sat)\n");

    let smt_bytes = smt.len();
    let t0 = Instant::now();
    let (verdict, _) = run_z3_stdin(&smt);
    let solver_time_us = t0.elapsed().as_micros() as u64;

    Ok(SvaBmcReport {
        property_name: prop.name.clone(),
        bound_cycles: k,
        verdict,
        smtlib2_bytes: smt_bytes,
        solver_time_us,
    })
}

pub(crate) fn run_z3_stdin(smt: &str) -> (SmtVerdict, String) {
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
