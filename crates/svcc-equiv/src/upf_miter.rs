//! Power-Aware Btor2 + Z3 Formal Miters (`svcc-equiv::upf_miter`).
//!
//! Zero-`svlang` SMT-LIB2 (`QF_BV` / `QF_ABV`) miters operating on word-level Btor2
//! models produced after `svcc_passes::upf_lower`:
//!
//! 1. **2-Copy Conditional Self-Composition Non-Interference (`prove_isolation_non_interference`)**:
//!    Instantiates two copies ($M^{(a)}, M^{(b)}$) sharing active-domain registers and primary
//!    inputs while giving powered-down domain registers and `upf__corrupt_*` / `upf__float_*`
//!    inputs independent unconstrained symbols. Proves universally (`unsat`) that under
//!    `upf__off_isolated_<D> == 1`, no top-level output can diverge between the two copies.
//! 2. **Universal Symbolic Retention Round-Trip (`prove_retention_roundtrip`)**:
//!    Proves on a gated module's Btor2 transition system from an arbitrary initial register
//!    assignment $S_0$ across `Save (dom_on=1) -> Collapse (dom_on=0, symbolic \xi) -> Restore (dom_on=1)`
//!    that every retained register equals its initial value $S_0[r]$ regardless of $\xi$.
//! 3. **Bounded UPF Protocol Safety (`prove_upf_protocol_bmc`)**:
//!    Unrolls `top` from reset/initial state for `K` cycles and checks whether `upf__violation`
//!    can fire.

use crate::pair::{init_terms, is_x_input, parse, solve, Model, Node, PairSide, Query, Side, Sort};
use crate::SmtVerdict;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use svcc_ir::SvccError;

const STAGE: &str = "svcc-equiv/upf";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpfCheckVerdict {
    ProvedUnsat,
    CounterexampleSat,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IsolationNonInterferenceReport {
    pub domain: String,
    pub verdict: UpfCheckVerdict,
    pub outputs_checked: usize,
    pub solver_time_us: u64,
    pub smtlib2_bytes: usize,
    pub witness: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetentionRoundTripReport {
    pub instance: String,
    pub reg_name: String,
    pub strategy: String,
    pub verdict: UpfCheckVerdict,
    pub solver_time_us: u64,
    pub smtlib2_bytes: usize,
    pub witness: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolBmcReport {
    pub bound_steps: u32,
    pub verdict: UpfCheckVerdict,
    pub solver_time_us: u64,
    pub smtlib2_bytes: usize,
    pub witness: Vec<String>,
}

fn define_custom_step(
    m: &Model,
    tag: char,
    smt: &mut String,
    step: u32,
    state0: &BTreeMap<u32, String>,
    input_term: &mut dyn FnMut(&str, Sort, &mut String) -> String,
) -> Result<(), SvccError> {
    let side = Side { m, tag };
    for (&id, (sort, node)) in &m.nodes {
        let me = format!("{tag}{step}_{id}");
        let body = match node {
            Node::Input { name } => {
                if is_x_input(name) {
                    sort.zero()
                } else if let Some(p) = m.port(name) {
                    input_term(p, *sort, smt)
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
                        Some(n) => side.r(*n, step - 1)?,
                        None => format!("{tag}{}_{id}", step - 1),
                    }
                }
            }
            Node::Const(lit) => lit.clone(),
            Node::Op { op, args, imm } => side.op(op, args, imm, *sort, step)?,
        };
        let _ = writeln!(smt, "(define-fun {me} () {} {body})", sort.smt());
    }
    for c in &m.constraints {
        let _ = writeln!(smt, "(assert (= {} #b1))", side.r(*c, step)?);
    }
    Ok(())
}

/// Proves 2-copy conditional corruption non-interference for `domain`:
/// Whenever `condition_output` (`upf__off_isolated_<D>`) is `1`, independent symbolic values
/// on `divergent_inputs` and any state belonging to `gated_instances` cannot cause any port in
/// `checked_outputs` to differ between Copy A and Copy B.
pub fn prove_isolation_non_interference(
    side: &PairSide,
    domain: &str,
    gated_instances: &[String],
    divergent_inputs: &[String],
    condition_output: &str,
    checked_outputs: &[String],
    timeout_ms: u32,
) -> Result<IsolationNonInterferenceReport, SvccError> {
    let m = parse(side)?;
    let div_set: BTreeSet<&str> = divergent_inputs.iter().map(String::as_str).collect();
    let mut smt = String::with_capacity(1 << 15);
    let _ = writeln!(smt, "(set-option :timeout {timeout_ms})");
    smt.push_str("(set-option :produce-models true)\n");
    let _ = writeln!(
        smt,
        "(set-logic {})",
        if m.has_arrays { "QF_ABV" } else { "QF_BV" }
    );

    let mut state_a = BTreeMap::new();
    let mut state_b = BTreeMap::new();
    let mut watch = Vec::new();

    for (id, sort, name) in m.states() {
        let key = m.state_key(&name);
        let in_gated = gated_instances
            .iter()
            .any(|inst| key == *inst || key.starts_with(&format!("{inst}.")));
        if in_gated {
            let sa = format!("st_a_{id}");
            let sb = format!("st_b_{id}");
            let _ = writeln!(smt, "(declare-const {sa} {})", sort.smt());
            let _ = writeln!(smt, "(declare-const {sb} {})", sort.smt());
            state_a.insert(id, sa);
            state_b.insert(id, sb);
        } else {
            let sh = format!("st_sh_{id}");
            let _ = writeln!(smt, "(declare-const {sh} {})", sort.smt());
            state_a.insert(id, sh.clone());
            state_b.insert(id, sh);
        }
    }

    let mut shared_ports: BTreeMap<String, Sort> = BTreeMap::new();
    let mut div_ports: BTreeMap<String, Sort> = BTreeMap::new();
    for (sort, node) in m.nodes.values() {
        if let Node::Input { name } = node {
            if let Some(p) = m.port(name) {
                if div_set.contains(p)
                    || p.starts_with("upf__corrupt_")
                    || p.starts_with("upf__float_")
                {
                    div_ports.insert(p.to_string(), *sort);
                } else {
                    shared_ports.insert(p.to_string(), *sort);
                }
            }
        }
    }

    for (p, sort) in &shared_ports {
        let _ = writeln!(smt, "(declare-const |in_sh_{p}| {})", sort.smt());
        watch.push((format!("input {p}"), format!("|in_sh_{p}|")));
    }
    for (p, sort) in &div_ports {
        let _ = writeln!(smt, "(declare-const |in_a_{p}| {})", sort.smt());
        let _ = writeln!(smt, "(declare-const |in_b_{p}| {})", sort.smt());
        watch.push((format!("copyA {p}"), format!("|in_a_{p}|")));
        watch.push((format!("copyB {p}"), format!("|in_b_{p}|")));
    }

    define_custom_step(&m, 'a', &mut smt, 0, &state_a, &mut |p, _, _| {
        if div_ports.contains_key(p) {
            format!("|in_a_{p}|")
        } else {
            format!("|in_sh_{p}|")
        }
    })?;
    define_custom_step(&m, 'b', &mut smt, 0, &state_b, &mut |p, _, _| {
        if div_ports.contains_key(p) {
            format!("|in_b_{p}|")
        } else {
            format!("|in_sh_{p}|")
        }
    })?;

    let sa = Side { m: &m, tag: 'a' };
    let sb = Side { m: &m, tag: 'b' };

    let cond_r = m.output_ref(condition_output).ok_or_else(|| {
        SvccError::new(
            STAGE,
            format!("missing off-isolated condition output '{condition_output}'"),
        )
    })?;
    let _ = writeln!(smt, "(assert (= {} #b1))", sa.r(cond_r, 0)?);
    let _ = writeln!(smt, "(assert (= {} #b1))", sb.r(cond_r, 0)?);

    let mut diffs = Vec::new();
    for out_p in checked_outputs {
        let Some(r) = m.output_ref(out_p) else {
            continue;
        };
        let ta = sa.r(r, 0)?;
        let tb = sb.r(r, 0)?;
        diffs.push(format!("(distinct {ta} {tb})"));
        watch.push((format!("copyA out {out_p}"), ta));
        watch.push((format!("copyB out {out_p}"), tb));
    }
    if diffs.is_empty() {
        smt.push_str("(assert false)\n");
    } else {
        let _ = writeln!(smt, "(assert (or {}))", diffs.join(" "));
    }
    smt.push_str("(check-sat)\n");

    let (v, cex, us, bytes) = solve(&Query { smt, watch });
    let verdict = match v {
        SmtVerdict::EquivalentUnsat => UpfCheckVerdict::ProvedUnsat,
        SmtVerdict::CounterexampleSat => UpfCheckVerdict::CounterexampleSat,
        SmtVerdict::SolverUnavailable => UpfCheckVerdict::Inconclusive,
    };
    Ok(IsolationNonInterferenceReport {
        domain: domain.to_string(),
        verdict,
        outputs_checked: checked_outputs.len(),
        solver_time_us: us,
        smtlib2_bytes: bytes,
        witness: cex,
    })
}

/// Proves on the specialized gated child module's Btor2 transition system that starting from
/// an arbitrary state $S_0$, executing:
/// - Step 0 (`save_dom_on` ? `1` : `0`, `ret_on = 1`, `save = 1`, `restore = 0`)
/// - Step 1 (`dom_on = 0`, `ret_on = 1`, `save = 0`, `restore = 0`, symbolic `upf__corrupt_*`)
/// - Step 2 (`dom_on = 1`, `ret_on = 1`, `save = 0`, `restore = 1`)
///
/// yields `state_step3[reg_name] == S_0[reg_name]` for all symbolic collapse assignments!
#[allow(clippy::too_many_arguments)]
pub fn prove_retention_roundtrip(
    child_side: &PairSide,
    instance: &str,
    reg_name: &str,
    strategy: &str,
    use_retention_as_primary: bool,
    save_while_dom_on: bool,
    timeout_ms: u32,
) -> Result<RetentionRoundTripReport, SvccError> {
    let m = parse(child_side)?;
    let mut smt = String::with_capacity(1 << 15);
    let _ = writeln!(smt, "(set-option :timeout {timeout_ms})");
    smt.push_str("(set-option :produce-models true)\n");
    let _ = writeln!(
        smt,
        "(set-logic {})",
        if m.has_arrays { "QF_ABV" } else { "QF_BV" }
    );

    let mut state0 = BTreeMap::new();
    let mut target_state: Option<(u32, String)> = None;
    let mut watch = Vec::new();

    for (id, sort, name) in m.states() {
        let key = m.state_key(&name);
        let sym = format!("init_st_{id}");
        let _ = writeln!(smt, "(declare-const {sym} {})", sort.smt());
        if key == reg_name {
            target_state = Some((id, sym.clone()));
            watch.push((format!("pre_collapse {reg_name}"), sym.clone()));
        }
        state0.insert(id, sym);
    }
    let (target_id, init_sym) = target_state.ok_or_else(|| {
        SvccError::new(
            STAGE,
            format!(
                "retained register '{reg_name}' not found in Btor2 states of '{}'",
                child_side.top
            ),
        )
    })?;

    let san_strat: String = strategy
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let p_ret_on = format!("upf__ret_{san_strat}_on");
    let p_save = format!("upf__ret_{san_strat}_save");
    let p_restore = format!("upf__ret_{san_strat}_restore");

    // Unroll 4 steps (0: save, 1: collapse, 2: wake+restore, 3: post-restore state readback).
    for step in 0..4u32 {
        define_custom_step(&m, 'r', &mut smt, step, &state0, &mut |p, sort, buf| {
            if p == "upf__dom_on" {
                match step {
                    0 => if save_while_dom_on { "#b1" } else { "#b0" }.to_string(),
                    1 => "#b0".to_string(),
                    _ => "#b1".to_string(),
                }
            } else if p == p_ret_on {
                "#b1".to_string()
            } else if !use_retention_as_primary && p == p_save {
                if step == 0 { "#b1" } else { "#b0" }.to_string()
            } else if !use_retention_as_primary && p == p_restore {
                if step == 2 { "#b1" } else { "#b0" }.to_string()
            } else {
                let v = format!("|in{step}_{p}|");
                let _ = writeln!(buf, "(declare-const {v} {})", sort.smt());
                v
            }
        })?;
    }

    let post_sym = format!("r3_{target_id}");
    watch.push((format!("post_restore {reg_name}"), post_sym.clone()));
    let _ = writeln!(smt, "(assert (distinct {post_sym} {init_sym}))");
    smt.push_str("(check-sat)\n");

    let (v, cex, us, bytes) = solve(&Query { smt, watch });
    let verdict = match v {
        SmtVerdict::EquivalentUnsat => UpfCheckVerdict::ProvedUnsat,
        SmtVerdict::CounterexampleSat => UpfCheckVerdict::CounterexampleSat,
        SmtVerdict::SolverUnavailable => UpfCheckVerdict::Inconclusive,
    };
    Ok(RetentionRoundTripReport {
        instance: instance.to_string(),
        reg_name: reg_name.to_string(),
        strategy: strategy.to_string(),
        verdict,
        solver_time_us: us,
        smtlib2_bytes: bytes,
        witness: cex,
    })
}

/// Bounded Model Checking (`K` cycles from initial state) of `bad_output` (`upf__violation`).
pub fn prove_upf_protocol_bmc(
    side: &PairSide,
    bad_output: &str,
    hold: &[(String, u64)],
    steps: u32,
    timeout_ms: u32,
) -> Result<ProtocolBmcReport, SvccError> {
    let m = parse(side)?;
    let init = init_terms(&m)?;
    let hold_map: BTreeMap<String, u64> = hold.iter().cloned().collect();
    let mut smt = String::with_capacity(1 << 15);
    let _ = writeln!(smt, "(set-option :timeout {timeout_ms})");
    smt.push_str("(set-option :produce-models true)\n");
    let _ = writeln!(
        smt,
        "(set-logic {})",
        if m.has_arrays { "QF_ABV" } else { "QF_BV" }
    );

    let mut ports: BTreeMap<String, Sort> = BTreeMap::new();
    for (sort, node) in m.nodes.values() {
        if let Node::Input { name } = node {
            if let Some(p) = m.port(name) {
                ports.insert(p.to_string(), *sort);
            }
        }
    }

    let side_p = Side { m: &m, tag: 'p' };
    let bad_r = m
        .output_ref(bad_output)
        .ok_or_else(|| SvccError::new(STAGE, format!("missing output '{bad_output}'")))?;
    let mut watch = Vec::new();
    let mut bad_terms = Vec::new();

    for step in 0..steps.max(1) {
        for (p, sort) in &ports {
            if !hold_map.contains_key(p) {
                let _ = writeln!(smt, "(declare-const |in{step}_{p}| {})", sort.smt());
                if !p.starts_with("upf__corrupt_") && !p.starts_with("upf__float_") {
                    watch.push((format!("step {step} {p}"), format!("|in{step}_{p}|")));
                }
            }
        }
        side_p.define_step(&mut smt, step, &init, &hold_map)?;
        let bt = side_p.r(bad_r, step)?;
        bad_terms.push(format!("(= {bt} #b1)"));
        watch.push((format!("step {step} {bad_output}"), bt));
    }

    let _ = writeln!(smt, "(assert (or {}))", bad_terms.join(" "));
    smt.push_str("(check-sat)\n");

    let (v, cex, us, bytes) = solve(&Query { smt, watch });
    let verdict = match v {
        SmtVerdict::EquivalentUnsat => UpfCheckVerdict::ProvedUnsat,
        SmtVerdict::CounterexampleSat => UpfCheckVerdict::CounterexampleSat,
        SmtVerdict::SolverUnavailable => UpfCheckVerdict::Inconclusive,
    };
    Ok(ProtocolBmcReport {
        bound_steps: steps.max(1),
        verdict,
        solver_time_us: us,
        smtlib2_bytes: bytes,
        witness: cex,
    })
}
