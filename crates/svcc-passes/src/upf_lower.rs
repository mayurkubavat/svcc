//! Structural IEEE 1801 UPF Power-Intent Lowering Pass over `svcc_ir::hdl::Design`.
//!
//! Transforms a pure synthesizable SystemC (`SC_MODULE`) or SystemVerilog (`module`) RTL
//! design paired with an [`UpfDesignIntent`] sidecar into an explicit two-state symbolic
//! hardware transition system suitable for both `svcc` (Btor2 + Z3 equivalence, 2-copy
//! corruption non-interference, and symbolic retention round-trip proofs) and `InFormal`
//! (`session_open` + `formal_lint` + `prove` / `check_sec` / `check_xprop`):
//!
//! 1. **Supply & Power-Switch DAG**: Evaluates top-level supply ports, `create_power_switch`
//!    boolean `-on_state` / `-off_state` expressions, `-ack_port` drivers, and `create_supply_net`
//!    `-resolve` rules (`one_hot` contention -> `0`).
//! 2. **Boundary Isolation Muxes**: Rewrites every output crossing of a switchable domain $D$
//!    into $y = \text{iso\_active} \;?\; c_{\text{clamp}} : (\text{dom\_on} \;?\; y_{\text{raw}} : \xi_{\text{float}})$,
//!    where $\xi_{\text{float}}$ is a fresh unconstrained primary input modeling off-domain float.
//! 3. **Shadow Retention & Symbolic Off-Domain State Corruption**: Specializes gated child
//!    instances with shadow registers $r_{\text{shadow}}$ updated on `save` while
//!    $\text{dom\_on} \land \text{ret\_on}$ (corrupting on `!ret_on` or simultaneous
//!    `save && restore`), restores $r \leftarrow r_{\text{shadow}}$ on `restore`, and drives
//!    every collapsed register $q \in D$ to a free symbolic input $\xi_q$ when `!\text{dom\_on}`.
//! 4. **Protocol Violation Monitors & SVA**: Synthesizes combinational `upf__violation` flags
//!    and companion IEEE 1800 SVA `assert property` + witness `cover property` monitors.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use svcc_ir::hdl::{
    BinOp, Conn, Design, Dir, Edge, Expr, LvBase, Lval, Module, Port, ProcKind, Process, Signal,
    Stmt, Ty, UnOp,
};
use svcc_ir::upf::{
    parse_upf_bool_expr, AppliesTo, ClampValue, Location, LsRule, SignalSense, SupplyResolve,
    SupplyStateKind, UpfBoolExpr, UpfDesignIntent, UpfDesignObject, UpfIsolation, UpfObjectKind,
    UpfRetention,
};
use svcc_ir::SvccError;

const STAGE: &str = "svcc-passes/upf-lower";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpfSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfStaticFinding {
    pub code: String,
    pub severity: UpfSeverity,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct UpfLowerOptions {
    /// Explicit top module override when `design.tops` has multiple roots.
    pub top: Option<String>,
    /// Expose `upf__violation` 1-bit output port on the top module.
    pub expose_violation_port: bool,
    /// Expose read-only `upf__ret_obs_*` and `upf__shadow_obs_*` ports on the top module.
    pub expose_obs_outputs: bool,
    /// Generate an IEEE 1800 SVA `assert property` + `cover property` block for `InFormal`.
    pub emit_sva: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoweredDomainSummary {
    pub name: String,
    pub primary_net: String,
    pub switchable: bool,
    pub dom_on_sig: String,
    pub instances: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoweredIsolationSummary {
    pub strategy: String,
    pub domain: String,
    pub signal: String,
    pub raw_signal: String,
    pub float_input: String,
    pub iso_active_sig: Option<String>,
    pub clamp: Option<ClampValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoweredRetentionReg {
    pub strategy: String,
    pub domain: String,
    pub instance: String,
    pub specialized_module: String,
    pub reg_name: String,
    pub width: u32,
    pub use_retention_as_primary: bool,
    pub active_obs_sig: String,
    pub shadow_obs_sig: String,
}

#[derive(Clone, Debug)]
pub struct UpfLowerReport {
    pub top: String,
    pub root_clock: Option<String>,
    pub reset_disable_sv: Option<String>,
    pub design: Design,
    pub static_findings: Vec<UpfStaticFinding>,
    pub domains: Vec<LoweredDomainSummary>,
    pub isolations: Vec<LoweredIsolationSummary>,
    pub retained_regs: Vec<LoweredRetentionReg>,
    pub non_retained_regs: Vec<(String, String, u32)>,
    pub corrupt_inputs: Vec<String>,
    pub supply_on_inputs: Vec<String>,
    pub sva_block: String,
}

impl UpfLowerReport {
    pub fn has_static_errors(&self) -> bool {
        self.static_findings
            .iter()
            .any(|f| f.severity == UpfSeverity::Error)
    }
}

/// Enumerates hierarchical instances, ports, nets, and sequential registers of `design`
/// so `[find_objects ...]` in UPF resolves against the elaborated RTL hierarchy.
pub fn collect_design_objects(design: &Design, top_opt: Option<&str>) -> Vec<UpfDesignObject> {
    let top_name = top_opt
        .map(str::to_string)
        .or_else(|| design.tops.first().cloned())
        .unwrap_or_default();
    let Some(top_mod) = design.module(&top_name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_rec(design, top_mod, "", &mut out);
    out
}

fn seq_written_names(m: &Module) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    for p in &m.processes {
        if matches!(p.kind, ProcKind::Seq { .. }) {
            Module::writes(&p.body, &mut s);
        }
    }
    s
}

fn collect_rec(design: &Design, m: &Module, prefix: &str, out: &mut Vec<UpfDesignObject>) {
    let seq_w = seq_written_names(m);
    let qualify = |n: &str| {
        if prefix.is_empty() {
            n.to_string()
        } else {
            format!("{prefix}.{n}")
        }
    };
    for p in &m.ports {
        if p.name == "svcc__ub" {
            continue;
        }
        out.push(UpfDesignObject {
            path: qualify(&p.name),
            kind: UpfObjectKind::Port,
        });
    }
    for s in &m.signals {
        if s.name.starts_with("svcc__") {
            continue;
        }
        out.push(UpfDesignObject {
            path: qualify(&s.name),
            kind: if seq_w.contains(&s.name) {
                UpfObjectKind::Reg
            } else {
                UpfObjectKind::Net
            },
        });
    }
    for inst in &m.instances {
        let sub = qualify(&inst.name);
        out.push(UpfDesignObject {
            path: sub.clone(),
            kind: UpfObjectKind::Inst,
        });
        if let Some(cm) = design.module(&inst.module) {
            collect_rec(design, cm, &sub, out);
        }
    }
}

fn bit(v: u64) -> Expr {
    Expr::konst(Ty::BIT, v)
}

fn sig1(name: &str) -> Expr {
    Expr::Sig {
        name: name.to_string(),
        ty: Ty::BIT,
    }
}

fn not1(e: Expr) -> Expr {
    Expr::Unary {
        op: UnOp::LogNot,
        arg: Box::new(e),
        ty: Ty::BIT,
    }
}

fn and1(a: Expr, b: Expr) -> Expr {
    Expr::Binary {
        op: BinOp::LogAnd,
        lhs: Box::new(a),
        rhs: Box::new(b),
        ty: Ty::BIT,
    }
}

fn or1(a: Expr, b: Expr) -> Expr {
    Expr::Binary {
        op: BinOp::LogOr,
        lhs: Box::new(a),
        rhs: Box::new(b),
        ty: Ty::BIT,
    }
}

fn xor1(a: Expr, b: Expr) -> Expr {
    Expr::Binary {
        op: BinOp::BitXor,
        lhs: Box::new(a),
        rhs: Box::new(b),
        ty: Ty::BIT,
    }
}

fn truth_of(e: Expr) -> Expr {
    if e.ty() == Ty::BIT {
        e
    } else {
        Expr::Unary {
            op: UnOp::RedOr,
            arg: Box::new(e),
            ty: Ty::BIT,
        }
    }
}

fn assign_sig_ty(name: &str, ty: Ty, rhs: Expr, nonblocking: bool) -> Stmt {
    Stmt::Assign {
        lhs: Lval {
            base: LvBase::Sig {
                name: name.to_string(),
                ty,
            },
            sel: None,
        },
        rhs,
        nonblocking,
    }
}

fn all_ones(ty: Ty) -> Expr {
    let n = ty.w.div_ceil(64) as usize;
    let mut words = vec![u64::MAX; n];
    let top = ty.w - 64 * (n as u32 - 1);
    if top < 64 {
        words[n - 1] = (1u64 << top) - 1;
    }
    Expr::Const { ty, words }
}

fn sanitize_ident(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn find_sig_or_port_ty(m: &Module, name: &str) -> Option<(Ty, Vec<u32>, Option<Dir>)> {
    if let Some(p) = m.ports.iter().find(|p| p.name == name) {
        return Some((p.ty, p.dims.clone(), Some(p.dir)));
    }
    if let Some(s) = m.signals.iter().find(|s| s.name == name) {
        return Some((s.ty, s.dims.clone(), None));
    }
    None
}

fn add_unique_signal(m: &mut Module, name: &str, ty: Ty, init: Option<Vec<u64>>) {
    if !m.signals.iter().any(|s| s.name == name) && !m.ports.iter().any(|p| p.name == name) {
        m.signals.push(Signal {
            name: name.to_string(),
            ty,
            scalar: ty == Ty::BIT,
            dims: Vec::new(),
            init,
        });
    }
}

fn add_unique_port(m: &mut Module, name: &str, dir: Dir, ty: Ty, dims: Vec<u32>) {
    if !m.ports.iter().any(|p| p.name == name) {
        m.ports.push(Port {
            name: name.to_string(),
            dir,
            ty,
            scalar: ty == Ty::BIT && dims.is_empty(),
            dims,
        });
    }
}

/// Discovers the primary clock input port and optional reset expression of `top`.
fn detect_clock_and_reset(
    design: &Design,
    top: &Module,
) -> (Option<String>, Option<(Expr, String)>) {
    let mut clk = None;
    for p in &top.processes {
        if let ProcKind::Seq { edges } = &p.kind {
            if let Some(e) = edges.first() {
                clk = Some(e.sig.clone());
                break;
            }
        }
    }
    if clk.is_none() {
        for inst in &top.instances {
            if let Some(cm) = design.module(&inst.module) {
                let mut child_clk = None;
                for p in &cm.processes {
                    if let ProcKind::Seq { edges } = &p.kind {
                        if let Some(e) = edges.first() {
                            child_clk = Some(e.sig.clone());
                            break;
                        }
                    }
                }
                if let Some(cc) = child_clk {
                    if let Some(conn) = inst.conns.iter().find(|c| c.port == cc) {
                        clk = Some(conn.sig.clone());
                        break;
                    }
                }
            }
        }
    }

    let mut rst = None;
    // Check explicit async reset edges first, then standard input port names.
    for port in &top.ports {
        if port.dir != Dir::In || port.ty != Ty::BIT || Some(&port.name) == clk.as_ref() {
            continue;
        }
        let low = port.name.to_ascii_lowercase();
        if low == "presetn"
            || low == "rst_n"
            || low == "aresetn"
            || low == "reset_n"
            || low.ends_with("_rst_n")
        {
            rst = Some((not1(sig1(&port.name)), format!("!{}", port.name)));
            break;
        }
        if low == "rst" || low == "reset" || low == "areset" || low == "preset" {
            rst = Some((sig1(&port.name), port.name.clone()));
            break;
        }
    }
    (clk, rst)
}

/// Ensures every child instance in `top` that needs UPF taps or domain instrumentation has
/// its own dedicated `Module` copy `<orig>__upf_<inst>` in `modules`.
/// Returns `(child_idx, new_top_idx)`.
fn ensure_instance_module(
    modules: &mut Vec<Module>,
    top_name: &str,
    inst_name: &str,
) -> Result<(usize, usize), SvccError> {
    let top_idx = modules
        .iter()
        .position(|m| m.name == top_name)
        .ok_or_else(|| SvccError::new(STAGE, format!("top module '{top_name}' not found")))?;
    let inst = modules[top_idx]
        .instances
        .iter()
        .find(|i| i.name == inst_name)
        .ok_or_else(|| SvccError::new(STAGE, format!("unknown child instance '{inst_name}'")))?
        .clone();
    let suffix = format!("__upf_{}", sanitize_ident(inst_name));
    if inst.module.ends_with(&suffix) {
        if let Some(idx) = modules.iter().position(|m| m.name == inst.module) {
            return Ok((idx, top_idx));
        }
    }
    let spec_name = format!("{}{suffix}", inst.module);
    if let Some(idx) = modules.iter().position(|m| m.name == spec_name) {
        return Ok((idx, top_idx));
    }
    let orig_idx = modules
        .iter()
        .position(|m| m.name == inst.module)
        .ok_or_else(|| {
            SvccError::new(
                STAGE,
                format!(
                    "module '{}' for instance '{inst_name}' not found",
                    inst.module
                ),
            )
        })?;
    let mut cloned = modules[orig_idx].clone();
    cloned.name = spec_name.clone();
    cloned.definition = spec_name.clone();
    // Insert right before `top_idx` so leaf-before-parent order holds.
    modules.insert(top_idx, cloned);
    let new_top_idx = top_idx + 1;
    if let Some(i_mut) = modules[new_top_idx]
        .instances
        .iter_mut()
        .find(|i| i.name == inst_name)
    {
        i_mut.module = spec_name;
    }
    Ok((top_idx, new_top_idx))
}

/// Exposes a hierarchical signal `<path>` (either `<sig>` in `top` or `<inst>.<sig>` in a
/// child instance of `top`) as a readable signal in `top`, returning `(top_sig_name, ty)`.
fn expose_signal_in_top(
    modules: &mut Vec<Module>,
    top_name: &str,
    top_idx: &mut usize,
    path: &str,
) -> Result<(String, Ty), SvccError> {
    let (base_path, _) = UpfDesignIntent::strip_bit_slice(path);
    if !base_path.contains('.') {
        let (ty, _, _) = find_sig_or_port_ty(&modules[*top_idx], base_path).ok_or_else(|| {
            SvccError::new(
                STAGE,
                format!(
                    "UPF signal '{base_path}' not found in top module '{}'",
                    modules[*top_idx].name
                ),
            )
        })?;
        return Ok((base_path.to_string(), ty));
    }
    let (inst_name, leaf) = base_path.split_once('.').unwrap();
    if leaf.contains('.') {
        return Err(SvccError::new(
            STAGE,
            format!("deeply nested UPF control path '{path}' (>1 instance hop) not yet flattened"),
        ));
    }
    // Check if `leaf` is already bound on `inst_name` in `top`.
    let existing_conn = modules[*top_idx]
        .instances
        .iter()
        .find(|i| i.name == inst_name)
        .and_then(|i| {
            i.conns
                .iter()
                .find(|c| c.port == leaf && c.elem.is_empty())
                .cloned()
        });
    if let Some(c) = existing_conn {
        if let Some((ty, _, _)) = find_sig_or_port_ty(&modules[*top_idx], &c.sig) {
            return Ok((c.sig, ty));
        }
    }

    let (child_idx, new_top_idx) = ensure_instance_module(modules, top_name, inst_name)?;
    *top_idx = new_top_idx;
    let (ty, dims, _) = find_sig_or_port_ty(&modules[child_idx], leaf).ok_or_else(|| {
        SvccError::new(
            STAGE,
            format!(
                "UPF signal '{leaf}' not found in instance '{inst_name}' (module '{}')",
                modules[child_idx].name
            ),
        )
    })?;
    if !dims.is_empty() {
        return Err(SvccError::new(
            STAGE,
            format!("UPF control signal '{path}' cannot be an unpacked array"),
        ));
    }

    let tap_port = format!("upf__tap_{}", sanitize_ident(leaf));
    if !modules[child_idx].ports.iter().any(|p| p.name == tap_port) {
        add_unique_port(&mut modules[child_idx], &tap_port, Dir::Out, ty, Vec::new());
        modules[child_idx].processes.push(Process {
            name: format!("p_{tap_port}"),
            kind: ProcKind::Comb,
            locals: Vec::new(),
            body: vec![assign_sig_ty(
                &tap_port,
                ty,
                Expr::Sig {
                    name: leaf.to_string(),
                    ty,
                },
                false,
            )],
            merged_from: 1,
            line: 0,
        });
    }

    let wire_name = format!(
        "upf__h_{}__{}",
        sanitize_ident(inst_name),
        sanitize_ident(leaf)
    );
    add_unique_signal(&mut modules[*top_idx], &wire_name, ty, None);
    let inst_mut = modules[*top_idx]
        .instances
        .iter_mut()
        .find(|i| i.name == inst_name)
        .unwrap();
    if !inst_mut.conns.iter().any(|c| c.port == tap_port) {
        inst_mut.conns.push(Conn {
            port: tap_port,
            sig: wire_name.clone(),
            elem: Vec::new(),
        });
    }
    Ok((wire_name, ty))
}

fn eval_upf_bool_to_hdl(
    e: &UpfBoolExpr,
    port_map: &BTreeMap<String, Expr>,
) -> Result<Expr, SvccError> {
    match e {
        UpfBoolExpr::Const(b) => Ok(bit(u64::from(*b))),
        UpfBoolExpr::Port(p) => port_map.get(p).cloned().ok_or_else(|| {
            SvccError::new(
                STAGE,
                format!("unknown control port '{p}' in UPF switch boolean expression"),
            )
        }),
        UpfBoolExpr::Not(a) => Ok(not1(eval_upf_bool_to_hdl(a, port_map)?)),
        UpfBoolExpr::And(a, b) => Ok(and1(
            eval_upf_bool_to_hdl(a, port_map)?,
            eval_upf_bool_to_hdl(b, port_map)?,
        )),
        UpfBoolExpr::Or(a, b) => Ok(or1(
            eval_upf_bool_to_hdl(a, port_map)?,
            eval_upf_bool_to_hdl(b, port_map)?,
        )),
        UpfBoolExpr::Xor(a, b) => Ok(xor1(
            eval_upf_bool_to_hdl(a, port_map)?,
            eval_upf_bool_to_hdl(b, port_map)?,
        )),
    }
}

fn matches_element_list(elems: &[String], excludes: &[String], candidates: &[String]) -> bool {
    for ex in excludes {
        let (ex_base, _) = UpfDesignIntent::strip_bit_slice(ex);
        if candidates.iter().any(|c| {
            if ex_base.contains('*') || ex_base.contains('?') {
                UpfDesignIntent::glob_match(ex_base, c)
            } else {
                c == ex_base
            }
        }) {
            return false;
        }
    }
    if elems.is_empty() {
        return true;
    }
    for el in elems {
        let (el_base, _) = UpfDesignIntent::strip_bit_slice(el);
        if candidates.iter().any(|c| {
            if el_base.contains('*') || el_base.contains('?') {
                UpfDesignIntent::glob_match(el_base, c)
            } else {
                c == el_base
            }
        }) {
            return true;
        }
    }
    false
}

/// Runs static UPF structural, PST, and voltage level-shifter checks against `design`.
fn run_static_checks(
    design: &Design,
    top: &Module,
    intent: &UpfDesignIntent,
    objects: &[UpfDesignObject],
) -> Vec<UpfStaticFinding> {
    let mut findings = Vec::new();
    let obj_paths: BTreeSet<&str> = objects.iter().map(|o| o.path.as_str()).collect();
    let has_obj = |raw: &str| -> bool {
        let (base, _) = UpfDesignIntent::strip_bit_slice(raw);
        if base.is_empty() {
            return true;
        }
        if base.contains('*') || base.contains('?') {
            objects
                .iter()
                .any(|o| UpfDesignIntent::glob_match(base, &o.path))
        } else {
            obj_paths.contains(base)
        }
    };

    if intent.domains.is_empty() {
        findings.push(UpfStaticFinding {
            code: "UPF_NO_DOMAINS".into(),
            severity: UpfSeverity::Error,
            message: "UPF file defines no power domains".into(),
        });
    }

    for dom in intent.domains.values() {
        for el in &dom.elements {
            if !has_obj(el) {
                findings.push(UpfStaticFinding {
                    code: "UPF_UNRESOLVED_TARGET".into(),
                    severity: UpfSeverity::Error,
                    message: format!(
                        "create_power_domain '{}' element '{}' matched no object in '{}'",
                        dom.name, el, top.name
                    ),
                });
            }
        }
    }

    for iso in intent.isolations.values() {
        if !intent.domains.contains_key(&iso.domain) {
            findings.push(UpfStaticFinding {
                code: "UPF_UNKNOWN_DOMAIN".into(),
                severity: UpfSeverity::Error,
                message: format!(
                    "set_isolation '{}' references unknown domain '{}'",
                    iso.name, iso.domain
                ),
            });
        }
        for el in &iso.elements {
            if !has_obj(el) {
                findings.push(UpfStaticFinding {
                    code: "UPF_UNRESOLVED_TARGET".into(),
                    severity: UpfSeverity::Error,
                    message: format!(
                        "set_isolation '{}' element '{}' matched no object in '{}'",
                        iso.name, el, top.name
                    ),
                });
            }
        }
        if !iso.no_isolation && !iso.isolation_signal.is_empty() && !has_obj(&iso.isolation_signal)
        {
            findings.push(UpfStaticFinding {
                code: "UPF_UNRESOLVED_TARGET".into(),
                severity: UpfSeverity::Error,
                message: format!(
                    "set_isolation '{}' control signal '{}' matched no object in '{}'",
                    iso.name, iso.isolation_signal, top.name
                ),
            });
        }
    }

    for ret in intent.retentions.values() {
        if !intent.domains.contains_key(&ret.domain) {
            findings.push(UpfStaticFinding {
                code: "UPF_UNKNOWN_DOMAIN".into(),
                severity: UpfSeverity::Error,
                message: format!(
                    "set_retention '{}' references unknown domain '{}'",
                    ret.name, ret.domain
                ),
            });
        }
        for el in &ret.elements {
            if !has_obj(el) {
                findings.push(UpfStaticFinding {
                    code: "UPF_UNRESOLVED_TARGET".into(),
                    severity: UpfSeverity::Error,
                    message: format!(
                        "set_retention '{}' element '{}' matched no object in '{}'",
                        ret.name, el, top.name
                    ),
                });
            }
        }
        if !ret.use_retention_as_primary {
            for (kind, s) in [("save", &ret.save_signal), ("restore", &ret.restore_signal)] {
                if !s.is_empty() && !has_obj(s) {
                    findings.push(UpfStaticFinding {
                        code: "UPF_UNRESOLVED_TARGET".into(),
                        severity: UpfSeverity::Error,
                        message: format!(
                            "set_retention '{}' {kind} signal '{s}' matched no object in '{}'",
                            ret.name, top.name
                        ),
                    });
                }
            }
        }
    }

    // PST state table validation & cross-domain voltage delta calculation.
    let mut port_to_net: BTreeMap<String, String> = BTreeMap::new();
    for c in &intent.connections {
        for p in &c.port_names {
            port_to_net.insert(p.clone(), c.net_name.clone());
        }
    }
    for pst in intent.psts.values() {
        for (st_name, tuple) in &pst.states {
            if tuple.len() != pst.supplies.len() {
                findings.push(UpfStaticFinding {
                    code: "UPF_PST_ARITY".into(),
                    severity: UpfSeverity::Error,
                    message: format!(
                        "PST '{}' state '{}' has {} entries for {} supplies",
                        pst.name,
                        st_name,
                        tuple.len(),
                        pst.supplies.len()
                    ),
                });
                continue;
            }
            for (sup, pstate) in pst.supplies.iter().zip(tuple) {
                if let Some(sp) = intent.supply_ports.get(sup) {
                    if !sp.states.is_empty() && !sp.states.contains_key(pstate) {
                        findings.push(UpfStaticFinding {
                            code: "UPF_PST_UNDEFINED_PORT_STATE".into(),
                            severity: UpfSeverity::Error,
                            message: format!(
                                "PST '{}' state '{}' names undefined port state '{}' on '{}'",
                                pst.name, st_name, pstate, sup
                            ),
                        });
                    }
                }
            }
        }
    }

    // Level-shifter static crossing audit across child instances vs top domain.
    let top_dom = intent.find_domain_for_path("").unwrap_or("");
    for inst in &top.instances {
        let Some(inst_dom) = intent.find_domain_for_path(&inst.name) else {
            continue;
        };
        if inst_dom == top_dom || top_dom.is_empty() {
            continue;
        }
        let Some(cm) = design.module(&inst.module) else {
            continue;
        };
        let top_net = intent.domain_power_net(top_dom).unwrap_or("");
        let inst_net = intent.domain_power_net(inst_dom).unwrap_or("");
        if top_net.is_empty() || inst_net.is_empty() || top_net == inst_net {
            continue;
        }

        // Check voltage pairs across PST rows where both nets are ON.
        let mut max_l2h_out = 0i32;
        let mut max_h2l_out = 0i32;
        let mut max_l2h_in = 0i32;
        let mut max_h2l_in = 0i32;
        for pst in intent.psts.values() {
            for tuple in pst.states.values() {
                let mut net_uv: BTreeMap<&str, (SupplyStateKind, i32)> = BTreeMap::new();
                for (sup, st) in pst.supplies.iter().zip(tuple) {
                    if let Some(sp) = intent.supply_ports.get(sup) {
                        if let Some(ps) = sp.states.get(st) {
                            let net = port_to_net.get(sup).map(String::as_str).unwrap_or(sup);
                            net_uv.insert(net, (ps.state, ps.nom_voltage_uv));
                        }
                    }
                }
                if let (Some((s_top, uv_top)), Some((s_inst, uv_inst))) =
                    (net_uv.get(top_net), net_uv.get(inst_net))
                {
                    if *s_top != SupplyStateKind::Off && *s_inst != SupplyStateKind::Off {
                        // Output crossing: inst_net -> top_net
                        if *uv_top > *uv_inst {
                            max_l2h_out = max_l2h_out.max(uv_top - uv_inst);
                        } else if *uv_inst > *uv_top {
                            max_h2l_out = max_h2l_out.max(uv_inst - uv_top);
                        }
                        // Input crossing: top_net -> inst_net
                        if *uv_inst > *uv_top {
                            max_l2h_in = max_l2h_in.max(uv_inst - uv_top);
                        } else if *uv_top > *uv_inst {
                            max_h2l_in = max_h2l_in.max(uv_top - uv_inst);
                        }
                    }
                }
            }
        }

        for conn in &inst.conns {
            if conn.port == "svcc__ub" {
                continue;
            }
            let Some(cp) = cm.ports.iter().find(|p| p.name == conn.port) else {
                continue;
            };
            let (l2h, h2l, is_out) = match cp.dir {
                Dir::Out => (max_l2h_out, max_h2l_out, true),
                Dir::In => (max_l2h_in, max_h2l_in, false),
            };
            let delta = l2h.max(h2l);
            if delta <= 50_000 {
                continue;
            }
            let cands = vec![conn.sig.clone(), format!("{}.{}", inst.name, conn.port)];
            let covered = intent.level_shifters.values().any(|ls| {
                if ls.domain != inst_dom && ls.domain != top_dom {
                    return false;
                }
                let dir_ok = match (ls.applies_to, is_out) {
                    (AppliesTo::Both, _) => true,
                    (AppliesTo::Outputs, true) | (AppliesTo::Inputs, false) => {
                        ls.domain == inst_dom
                    }
                    (AppliesTo::Inputs, true) | (AppliesTo::Outputs, false) => ls.domain == top_dom,
                };
                if !dir_ok || !matches_element_list(&ls.elements, &ls.exclude_elements, &cands) {
                    return false;
                }
                let rule_ok = match ls.rule {
                    LsRule::Both => true,
                    LsRule::LowToHigh => h2l <= ls.threshold_uv,
                    LsRule::HighToLow => l2h <= ls.threshold_uv,
                };
                // Threshold in UPF `set_level_shifter -threshold T` waives crossings <= T OR
                // declares shifter tolerance; if no shifter covers a delta > 50mV with matching rule, fail!
                rule_ok
            });
            if !covered {
                findings.push(UpfStaticFinding {
                    code: "LS_MISSING".into(),
                    severity: UpfSeverity::Error,
                    message: format!(
                        "crossing '{}.{}' ({}) spans {} uV voltage difference without a matching set_level_shifter",
                        inst.name, conn.port, conn.sig, delta
                    ),
                });
            }
        }
    }

    findings
}

/// Structurally lowers `design` with IEEE 1801 `intent` into a power-aware `hdl::Design`.
pub fn lower_design_with_upf(
    design: &Design,
    intent: &UpfDesignIntent,
    opts: &UpfLowerOptions,
) -> Result<UpfLowerReport, SvccError> {
    let top_name = match &opts.top {
        Some(t) => t.clone(),
        None => match design.tops.as_slice() {
            [t] => t.clone(),
            [] => return Err(SvccError::new(STAGE, "design has no top module")),
            many => {
                return Err(SvccError::new(
                    STAGE,
                    format!(
                        "design has multiple top modules ({}); specify --top",
                        many.join(", ")
                    ),
                ))
            }
        },
    };
    let orig_top = design
        .module(&top_name)
        .ok_or_else(|| SvccError::new(STAGE, format!("top module '{top_name}' not found")))?;

    let objects = collect_design_objects(design, Some(&top_name));
    let static_findings = run_static_checks(design, orig_top, intent, &objects);
    let (root_clock, rst_info) = detect_clock_and_reset(design, orig_top);

    let mut modules = design.modules.clone();
    let mut top_idx = modules
        .iter()
        .position(|m| m.name == top_name)
        .expect("top module index");

    let mut pwr_comb_stmts: Vec<Stmt> = Vec::new();
    let mut pwr_seq_stmts: Vec<Stmt> = Vec::new();
    let mut supply_on_inputs: Vec<String> = Vec::new();
    let mut corrupt_inputs: Vec<String> = Vec::new();

    // ------------------------------------------------------------------
    // 1. Top-level Supply Ports, Power Switches & Supply Nets
    // ------------------------------------------------------------------
    let mut port_on_expr: BTreeMap<String, Expr> = BTreeMap::new();
    let mut port_state_exprs: BTreeMap<String, BTreeMap<String, Expr>> = BTreeMap::new();

    for sp in intent.supply_ports.values() {
        if sp.is_output {
            continue;
        }
        let has_off = sp.states.values().any(|s| {
            s.state == SupplyStateKind::Off
                && s.nom_voltage_uv == 0
                && !sp.name.eq_ignore_ascii_case("VSS")
        });
        let is_gnd = !sp.states.is_empty()
            && sp
                .states
                .values()
                .all(|s| s.nom_voltage_uv == 0 && s.state != SupplyStateKind::Off);
        if is_gnd {
            port_on_expr.insert(sp.name.clone(), bit(1));
            let mut st_map = BTreeMap::new();
            for st in sp.states.keys() {
                st_map.insert(st.clone(), bit(1));
            }
            port_state_exprs.insert(sp.name.clone(), st_map);
            continue;
        }

        let on_expr = if has_off {
            let in_name = format!("upf__sup_{}_on", sanitize_ident(&sp.name));
            add_unique_port(
                &mut modules[top_idx],
                &in_name,
                Dir::In,
                Ty::BIT,
                Vec::new(),
            );
            supply_on_inputs.push(in_name.clone());
            sig1(&in_name)
        } else {
            bit(1)
        };
        port_on_expr.insert(sp.name.clone(), on_expr.clone());

        let mut on_states: Vec<&String> = sp
            .states
            .values()
            .filter(|s| s.state != SupplyStateKind::Off)
            .map(|s| &s.name)
            .collect();
        if !sp.default_on_state.is_empty() {
            if let Some(pos) = on_states.iter().position(|n| **n == sp.default_on_state) {
                if pos != 0 {
                    let def = on_states.remove(pos);
                    on_states.insert(0, def);
                }
            }
        }
        let mut st_map = BTreeMap::new();
        if on_states.len() >= 2 {
            let sel_name = format!("upf__sup_{}_dvfs", sanitize_ident(&sp.name));
            add_unique_port(
                &mut modules[top_idx],
                &sel_name,
                Dir::In,
                Ty::BIT,
                Vec::new(),
            );
            st_map.insert(
                on_states[0].clone(),
                and1(on_expr.clone(), not1(sig1(&sel_name))),
            );
            st_map.insert(on_states[1].clone(), and1(on_expr.clone(), sig1(&sel_name)));
            for extra in on_states.iter().skip(2) {
                st_map.insert((*extra).clone(), bit(0));
            }
        } else if let Some(only) = on_states.first() {
            st_map.insert((*only).clone(), on_expr.clone());
        }
        for s in sp
            .states
            .values()
            .filter(|s| s.state == SupplyStateKind::Off)
        {
            st_map.insert(s.name.clone(), not1(on_expr.clone()));
        }
        port_state_exprs.insert(sp.name.clone(), st_map);
    }

    // Seed supply nets connected to top-level input supply ports.
    let mut net_drivers: BTreeMap<String, Vec<Expr>> = BTreeMap::new();
    for conn in &intent.connections {
        for p in &conn.port_names {
            if let Some(e) = port_on_expr.get(p) {
                net_drivers
                    .entry(conn.net_name.clone())
                    .or_default()
                    .push(e.clone());
            }
        }
    }

    let resolve_net_expr = |drivers: &[Expr], resolve: SupplyResolve| -> Expr {
        match drivers {
            [] => bit(1),
            [only] => only.clone(),
            many => {
                let any_on = many.iter().cloned().reduce(or1).unwrap_or_else(|| bit(0));
                if matches!(
                    resolve,
                    SupplyResolve::OneHot | SupplyResolve::ParallelOneHot
                ) {
                    // Contention (`>= 2` simultaneously active drivers) collapses `one_hot` net to 0.
                    let mut pair_clash = bit(0);
                    for i in 0..many.len() {
                        for j in (i + 1)..many.len() {
                            pair_clash = or1(pair_clash, and1(many[i].clone(), many[j].clone()));
                        }
                    }
                    and1(any_on, not1(pair_clash))
                } else {
                    any_on
                }
            }
        }
    };

    // Pre-expose all switch control ports in `top`.
    let mut sw_ctrl_top_sigs: BTreeMap<(String, String), Expr> = BTreeMap::new();
    for sw in intent.power_switches.values() {
        for (port_name, hdl_path) in &sw.control_ports {
            let (top_sig, ty) =
                expose_signal_in_top(&mut modules, &top_name, &mut top_idx, hdl_path)?;
            let e = truth_of(Expr::Sig { name: top_sig, ty });
            sw_ctrl_top_sigs.insert((sw.name.clone(), port_name.clone()), e);
        }
    }

    // Evaluate switches across up to `N + 1` topological rounds for cascaded switches.
    let mut sw_on_sigs: BTreeMap<String, String> = BTreeMap::new();
    let sw_names: Vec<String> = intent.power_switches.keys().cloned().collect();
    let mut resolved_switches = BTreeSet::new();
    for _ in 0..=sw_names.len() {
        for sw_name in &sw_names {
            if resolved_switches.contains(sw_name) {
                continue;
            }
            let sw = &intent.power_switches[sw_name];
            let inputs_ready = sw.input_supply_ports.iter().all(|(_, net)| {
                net_drivers.contains_key(net)
                    || !intent
                        .power_switches
                        .values()
                        .any(|other| other.output_supply_port.1 == *net)
            });
            if !inputs_ready {
                continue;
            }

            let mut ctrl_map = BTreeMap::new();
            for (port_name, _) in &sw.control_ports {
                if let Some(e) = sw_ctrl_top_sigs.get(&(sw.name.clone(), port_name.clone())) {
                    ctrl_map.insert(port_name.clone(), e.clone());
                }
            }

            let mut any_on = bit(0);
            for on_st in &sw
                .on_states
                .iter()
                .chain(&sw.on_partial_states)
                .collect::<Vec<_>>()
            {
                let b_ast = parse_upf_bool_expr(&on_st.boolean_expr)?;
                let b_expr = eval_upf_bool_to_hdl(&b_ast, &ctrl_map)?;
                let vin_net = sw
                    .input_supply_ports
                    .iter()
                    .find(|(p, _)| *p == on_st.input_supply_port)
                    .or_else(|| sw.input_supply_ports.first())
                    .map(|(_, n)| n.as_str())
                    .unwrap_or("");
                let vin_on = if vin_net.is_empty() {
                    bit(1)
                } else {
                    let res = intent
                        .supply_nets
                        .get(vin_net)
                        .map(|n| n.resolve)
                        .unwrap_or_default();
                    let drvs = net_drivers.get(vin_net).map(Vec::as_slice).unwrap_or(&[]);
                    resolve_net_expr(drvs, res)
                };
                any_on = or1(any_on, and1(vin_on, b_expr));
            }
            // If both `on_state` and `off_state` evaluate true simultaneously, contention -> OFF!
            let mut any_off_overlap = bit(0);
            for off_st in &sw.off_states {
                let b_ast = parse_upf_bool_expr(&off_st.boolean_expr)?;
                let b_expr = eval_upf_bool_to_hdl(&b_ast, &ctrl_map)?;
                any_off_overlap = or1(any_off_overlap, b_expr);
            }
            let sw_final = if sw.off_states.is_empty() {
                any_on
            } else {
                // Standard `-on_state {ON vin {en}} -off_state {OFF {!en}}` is mutually exclusive;
                // if overlapping `-off_state` is true, it forces OFF (`UNDETERMINED` -> corrupt).
                and1(any_on, not1(any_off_overlap))
            };

            let sw_sig = format!("upf__sw_{}_on", sanitize_ident(sw_name));
            add_unique_signal(&mut modules[top_idx], &sw_sig, Ty::BIT, None);
            pwr_comb_stmts.push(assign_sig_ty(&sw_sig, Ty::BIT, sw_final, false));
            sw_on_sigs.insert(sw_name.clone(), sw_sig.clone());

            let (out_port, out_net) = &sw.output_supply_port;
            if !out_net.is_empty() {
                net_drivers
                    .entry(out_net.clone())
                    .or_default()
                    .push(sig1(&sw_sig));
            }
            if !out_port.is_empty() {
                let full_out = format!("{sw_name}.{out_port}");
                port_on_expr.insert(full_out.clone(), sig1(&sw_sig));
                if let Some(sp) = intent.supply_ports.get(&full_out) {
                    let vin_port = sw
                        .input_supply_ports
                        .first()
                        .and_then(|(_, net)| {
                            intent
                                .connections
                                .iter()
                                .find(|c| c.net_name == *net)
                                .and_then(|c| c.port_names.first())
                        })
                        .cloned();
                    let mut st_map = BTreeMap::new();
                    for ps in sp.states.values() {
                        if ps.state == SupplyStateKind::Off {
                            st_map.insert(ps.name.clone(), not1(sig1(&sw_sig)));
                        } else if let Some(vp) = &vin_port {
                            let vin_st = port_state_exprs
                                .get(vp)
                                .and_then(|m| m.get(&ps.name))
                                .cloned()
                                .unwrap_or_else(|| bit(1));
                            st_map.insert(ps.name.clone(), and1(sig1(&sw_sig), vin_st));
                        } else {
                            st_map.insert(ps.name.clone(), sig1(&sw_sig));
                        }
                    }
                    port_state_exprs.insert(full_out, st_map);
                }
            }

            // Drive `-ack_port` if it is a top-level undriven signal.
            let mut already_written = BTreeSet::new();
            for p in &modules[top_idx].processes {
                Module::writes(&p.body, &mut already_written);
            }
            for inst in &modules[top_idx].instances {
                if let Some(cm) = modules.iter().find(|m| m.name == inst.module) {
                    for c in &inst.conns {
                        if cm
                            .ports
                            .iter()
                            .any(|p| p.name == c.port && p.dir == Dir::Out)
                        {
                            already_written.insert(c.sig.clone());
                        }
                    }
                }
            }
            for (_, ack_path) in &sw.ack_ports {
                if !ack_path.contains('.') && !already_written.contains(ack_path) {
                    if let Some(s) = modules[top_idx]
                        .signals
                        .iter_mut()
                        .find(|s| s.name == *ack_path)
                    {
                        s.init = None;
                        let sty = s.ty;
                        let rhs = if sty == Ty::BIT {
                            sig1(&sw_sig)
                        } else {
                            Expr::Resize {
                                arg: Box::new(sig1(&sw_sig)),
                                ty: Ty::unsigned(sty.w),
                            }
                        };
                        pwr_comb_stmts.push(assign_sig_ty(ack_path, sty, rhs, false));
                    }
                }
            }

            resolved_switches.insert(sw_name.clone());
        }
    }

    // Materialize `upf__net_<N>_on` and `upf__dom_<D>_on` signals in `top`.
    let mut net_on_sig: BTreeMap<String, String> = BTreeMap::new();
    let mut switchable_nets: BTreeSet<String> = BTreeSet::new();
    for sw in intent.power_switches.values() {
        if !sw.output_supply_port.1.is_empty() {
            switchable_nets.insert(sw.output_supply_port.1.clone());
        }
    }
    for conn in &intent.connections {
        for p in &conn.port_names {
            if intent
                .supply_ports
                .get(p)
                .is_some_and(|sp| sp.states.values().any(|s| s.state == SupplyStateKind::Off))
            {
                switchable_nets.insert(conn.net_name.clone());
            }
        }
    }

    for (net_name, net_def) in &intent.supply_nets {
        let sname = format!("upf__net_{}_on", sanitize_ident(net_name));
        add_unique_signal(&mut modules[top_idx], &sname, Ty::BIT, None);
        let drvs = net_drivers.get(net_name).map(Vec::as_slice).unwrap_or(&[]);
        let e = resolve_net_expr(drvs, net_def.resolve);
        pwr_comb_stmts.push(assign_sig_ty(&sname, Ty::BIT, e, false));
        net_on_sig.insert(net_name.clone(), sname);
    }

    let mut dom_on_sig: BTreeMap<String, String> = BTreeMap::new();
    let mut dom_switchable: BTreeMap<String, bool> = BTreeMap::new();
    let mut domain_summaries = Vec::new();
    for dom in intent.domains.values() {
        let pnet = intent.domain_power_net(&dom.name).unwrap_or("").to_string();
        let dsig = format!("upf__dom_{}_on", sanitize_ident(&dom.name));
        add_unique_signal(&mut modules[top_idx], &dsig, Ty::BIT, None);
        let e = net_on_sig
            .get(&pnet)
            .map(|s| sig1(s))
            .unwrap_or_else(|| bit(1));
        pwr_comb_stmts.push(assign_sig_ty(&dsig, Ty::BIT, e, false));
        let is_sw = switchable_nets.contains(&pnet);
        dom_on_sig.insert(dom.name.clone(), dsig.clone());
        dom_switchable.insert(dom.name.clone(), is_sw);
        let insts: Vec<String> = modules[top_idx]
            .instances
            .iter()
            .filter(|i| intent.find_domain_for_path(&i.name) == Some(dom.name.as_str()))
            .map(|i| i.name.clone())
            .collect();
        domain_summaries.push(LoweredDomainSummary {
            name: dom.name.clone(),
            primary_net: pnet,
            switchable: is_sw,
            dom_on_sig: dsig,
            instances: insts,
        });
    }

    // ------------------------------------------------------------------
    // 2. Retention Control Signals (`save_trig`, `restore_trig`, `ret_on`)
    // ------------------------------------------------------------------
    struct RetCtrlSigs {
        ret_on_sig: String,
        save_trig_sig: Option<String>,
        restore_trig_sig: Option<String>,
    }
    let mut ret_ctrls: BTreeMap<String, RetCtrlSigs> = BTreeMap::new();
    for ret in intent.retentions.values() {
        let rnet = if !ret.retention_power_net.is_empty() {
            Some(ret.retention_power_net.as_str())
        } else {
            intent.supply_set_power_net(&ret.retention_supply_set)
        };
        let ret_on_name = format!("upf__ret_{}_on", sanitize_ident(&ret.name));
        add_unique_signal(&mut modules[top_idx], &ret_on_name, Ty::BIT, None);
        let ret_on_e = rnet
            .and_then(|n| net_on_sig.get(n))
            .map(|s| sig1(s))
            .unwrap_or_else(|| bit(1));
        pwr_comb_stmts.push(assign_sig_ty(&ret_on_name, Ty::BIT, ret_on_e, false));

        if ret.use_retention_as_primary {
            ret_ctrls.insert(
                ret.name.clone(),
                RetCtrlSigs {
                    ret_on_sig: ret_on_name,
                    save_trig_sig: None,
                    restore_trig_sig: None,
                },
            );
            continue;
        }

        let mut build_trig =
            |kind: &str, raw_path: &str, sense: SignalSense| -> Result<String, SvccError> {
                let trig_name = format!("upf__ret_{}_{}_trig", sanitize_ident(&ret.name), kind);
                add_unique_signal(&mut modules[top_idx], &trig_name, Ty::BIT, None);
                if raw_path.is_empty() {
                    pwr_comb_stmts.push(assign_sig_ty(&trig_name, Ty::BIT, bit(0), false));
                    return Ok(trig_name);
                }
                let (top_s, ty) =
                    expose_signal_in_top(&mut modules, &top_name, &mut top_idx, raw_path)?;
                let raw_b = truth_of(Expr::Sig { name: top_s, ty });
                let trig_e = match (sense, root_clock.is_some()) {
                    (SignalSense::High, _) | (SignalSense::Posedge, false) => raw_b,
                    (SignalSense::Low, _) | (SignalSense::Negedge, false) => not1(raw_b),
                    (SignalSense::Posedge, true) => {
                        let prev = format!("upf__ret_{}_{}_prev", sanitize_ident(&ret.name), kind);
                        add_unique_signal(&mut modules[top_idx], &prev, Ty::BIT, Some(vec![0]));
                        pwr_seq_stmts.push(assign_sig_ty(&prev, Ty::BIT, raw_b.clone(), true));
                        and1(raw_b, not1(sig1(&prev)))
                    }
                    (SignalSense::Negedge, true) => {
                        let prev = format!("upf__ret_{}_{}_prev", sanitize_ident(&ret.name), kind);
                        add_unique_signal(&mut modules[top_idx], &prev, Ty::BIT, Some(vec![1]));
                        pwr_seq_stmts.push(assign_sig_ty(&prev, Ty::BIT, raw_b.clone(), true));
                        and1(not1(raw_b), sig1(&prev))
                    }
                };
                pwr_comb_stmts.push(assign_sig_ty(&trig_name, Ty::BIT, trig_e, false));
                Ok(trig_name)
            };

        let save_trig = build_trig("save", &ret.save_signal, ret.save_sense)?;
        let restore_trig = build_trig("restore", &ret.restore_signal, ret.restore_sense)?;
        ret_ctrls.insert(
            ret.name.clone(),
            RetCtrlSigs {
                ret_on_sig: ret_on_name,
                save_trig_sig: Some(save_trig),
                restore_trig_sig: Some(restore_trig),
            },
        );
    }

    // ------------------------------------------------------------------
    // 3. Isolation Control Active Predicates
    // ------------------------------------------------------------------
    let mut iso_active_sigs: BTreeMap<String, String> = BTreeMap::new();
    for iso in intent.isolations.values() {
        if iso.no_isolation {
            continue;
        }
        let act_name = format!("upf__iso_{}_active", sanitize_ident(&iso.name));
        add_unique_signal(&mut modules[top_idx], &act_name, Ty::BIT, None);

        let sig_e = if iso.isolation_signal.is_empty() {
            bit(1)
        } else {
            let (top_s, ty) =
                expose_signal_in_top(&mut modules, &top_name, &mut top_idx, &iso.isolation_signal)?;
            let raw_b = truth_of(Expr::Sig { name: top_s, ty });
            match iso.isolation_sense {
                SignalSense::Low | SignalSense::Negedge => not1(raw_b),
                SignalSense::High | SignalSense::Posedge => raw_b,
            }
        };

        // Isolation cell supply: if `-location self` with no explicit `-isolation_supply_set` /
        // `-isolation_power_net`, the isolation cell sits on the domain's own primary supply!
        let iso_net = if !iso.isolation_power_net.is_empty() {
            Some(iso.isolation_power_net.as_str())
        } else if !iso.isolation_supply_set.is_empty() {
            intent.supply_set_power_net(&iso.isolation_supply_set)
        } else if iso.has_explicit_location && iso.location == Location::SelfLoc {
            intent.domain_power_net(&iso.domain)
        } else {
            None
        };
        let sup_e = iso_net
            .and_then(|n| net_on_sig.get(n))
            .map(|s| sig1(s))
            .unwrap_or_else(|| bit(1));
        pwr_comb_stmts.push(assign_sig_ty(&act_name, Ty::BIT, and1(sig_e, sup_e), false));
        iso_active_sigs.insert(iso.name.clone(), act_name);
    }

    // ------------------------------------------------------------------
    // 4. Gated Child Instances: Retention, Symbolic Collapse & Isolation Muxes
    // ------------------------------------------------------------------
    let mut retained_regs = Vec::new();
    let mut non_retained_regs = Vec::new();
    let mut isolation_summaries = Vec::new();
    let mut domain_has_isolated_crossing: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let inst_names: Vec<String> = modules[top_idx]
        .instances
        .iter()
        .map(|i| i.name.clone())
        .collect();

    for inst_name in inst_names {
        let Some(dom_name) = intent.find_domain_for_path(&inst_name).map(str::to_string) else {
            continue;
        };
        if !dom_switchable.get(&dom_name).copied().unwrap_or(false) {
            continue;
        }
        let d_on_top = dom_on_sig[&dom_name].clone();

        let (child_idx, new_top_idx) = ensure_instance_module(&mut modules, &top_name, &inst_name)?;
        top_idx = new_top_idx;
        let spec_mod_name = modules[child_idx].name.clone();

        // Thread `upf__dom_on` into child module.
        add_unique_port(
            &mut modules[child_idx],
            "upf__dom_on",
            Dir::In,
            Ty::BIT,
            Vec::new(),
        );
        modules[top_idx]
            .instances
            .iter_mut()
            .find(|i| i.name == inst_name)
            .unwrap()
            .conns
            .push(Conn {
                port: "upf__dom_on".into(),
                sig: d_on_top.clone(),
                elem: Vec::new(),
            });

        // Identify all sequential signals/ports in child module.
        let seq_sigs = seq_written_names(&modules[child_idx]);
        let mut child_ret_map: BTreeMap<String, UpfRetention> = BTreeMap::new();
        for sname in &seq_sigs {
            if sname.starts_with("svcc__") || sname.starts_with("upf__") {
                continue;
            }
            let cands = vec![format!("{inst_name}.{sname}"), sname.clone()];
            if let Some(ret) = intent.retentions.values().find(|r| {
                r.domain == dom_name
                    && matches_element_list(&r.elements, &r.exclude_elements, &cands)
            }) {
                child_ret_map.insert(sname.clone(), ret.clone());
            }
        }

        // Thread required retention control signals into child module.
        let mut threaded_rets = BTreeSet::new();
        for ret in child_ret_map.values() {
            if !threaded_rets.insert(ret.name.clone()) {
                continue;
            }
            let rc = &ret_ctrls[&ret.name];
            let p_on = format!("upf__ret_{}_on", sanitize_ident(&ret.name));
            add_unique_port(&mut modules[child_idx], &p_on, Dir::In, Ty::BIT, Vec::new());
            modules[top_idx]
                .instances
                .iter_mut()
                .find(|i| i.name == inst_name)
                .unwrap()
                .conns
                .push(Conn {
                    port: p_on,
                    sig: rc.ret_on_sig.clone(),
                    elem: Vec::new(),
                });
            if let (Some(sv), Some(rs)) = (&rc.save_trig_sig, &rc.restore_trig_sig) {
                let p_sv = format!("upf__ret_{}_save", sanitize_ident(&ret.name));
                let p_rs = format!("upf__ret_{}_restore", sanitize_ident(&ret.name));
                add_unique_port(&mut modules[child_idx], &p_sv, Dir::In, Ty::BIT, Vec::new());
                add_unique_port(&mut modules[child_idx], &p_rs, Dir::In, Ty::BIT, Vec::new());
                let inst_mut = modules[top_idx]
                    .instances
                    .iter_mut()
                    .find(|i| i.name == inst_name)
                    .unwrap();
                inst_mut.conns.push(Conn {
                    port: p_sv,
                    sig: sv.clone(),
                    elem: Vec::new(),
                });
                inst_mut.conns.push(Conn {
                    port: p_rs,
                    sig: rs.clone(),
                    elem: Vec::new(),
                });
            }
        }

        // Add symbolic corruption ports and shadow registers for each sequential register.
        let mut child_obs_comb: Vec<Stmt> = Vec::new();
        for sname in &seq_sigs {
            if sname.starts_with("svcc__") || sname.starts_with("upf__") {
                continue;
            }
            let Some((sty, sdims, _)) = find_sig_or_port_ty(&modules[child_idx], sname) else {
                continue;
            };
            if sdims.len() > 1 {
                continue;
            }
            let c_child_port = format!("upf__corrupt_{}", sanitize_ident(sname));
            let c_top_port = format!(
                "upf__corrupt_{}_{}",
                sanitize_ident(&inst_name),
                sanitize_ident(sname)
            );
            add_unique_port(
                &mut modules[child_idx],
                &c_child_port,
                Dir::In,
                sty,
                sdims.clone(),
            );
            add_unique_port(
                &mut modules[top_idx],
                &c_top_port,
                Dir::In,
                sty,
                sdims.clone(),
            );
            corrupt_inputs.push(c_top_port.clone());
            modules[top_idx]
                .instances
                .iter_mut()
                .find(|i| i.name == inst_name)
                .unwrap()
                .conns
                .push(Conn {
                    port: c_child_port,
                    sig: c_top_port,
                    elem: Vec::new(),
                });

            if let Some(ret) = child_ret_map.get(sname) {
                if sdims.is_empty() && !ret.use_retention_as_primary {
                    let s_init = modules[child_idx]
                        .signals
                        .iter()
                        .find(|s| s.name == *sname)
                        .and_then(|s| s.init.clone());
                    let shadow_name = format!("upf__shadow_{}", sanitize_ident(sname));
                    add_unique_signal(&mut modules[child_idx], &shadow_name, sty, s_init);

                    let cr_child = format!("upf__corrupt_ret_{}", sanitize_ident(sname));
                    let cr_top = format!(
                        "upf__corrupt_ret_{}_{}",
                        sanitize_ident(&inst_name),
                        sanitize_ident(sname)
                    );
                    add_unique_port(&mut modules[child_idx], &cr_child, Dir::In, sty, Vec::new());
                    add_unique_port(&mut modules[top_idx], &cr_top, Dir::In, sty, Vec::new());
                    corrupt_inputs.push(cr_top.clone());
                    modules[top_idx]
                        .instances
                        .iter_mut()
                        .find(|i| i.name == inst_name)
                        .unwrap()
                        .conns
                        .push(Conn {
                            port: cr_child,
                            sig: cr_top,
                            elem: Vec::new(),
                        });

                    // Observation taps for active & shadow register values.
                    let obs_act_p = format!("upf__ret_obs_{}", sanitize_ident(sname));
                    let obs_shd_p = format!("upf__shadow_obs_{}", sanitize_ident(sname));
                    add_unique_port(
                        &mut modules[child_idx],
                        &obs_act_p,
                        Dir::Out,
                        sty,
                        Vec::new(),
                    );
                    add_unique_port(
                        &mut modules[child_idx],
                        &obs_shd_p,
                        Dir::Out,
                        sty,
                        Vec::new(),
                    );
                    child_obs_comb.push(assign_sig_ty(
                        &obs_act_p,
                        sty,
                        Expr::Sig {
                            name: sname.clone(),
                            ty: sty,
                        },
                        false,
                    ));
                    child_obs_comb.push(assign_sig_ty(
                        &obs_shd_p,
                        sty,
                        Expr::Sig {
                            name: shadow_name,
                            ty: sty,
                        },
                        false,
                    ));

                    let top_act_obs = format!(
                        "upf__ret_obs_{}_{}",
                        sanitize_ident(&inst_name),
                        sanitize_ident(sname)
                    );
                    let top_shd_obs = format!(
                        "upf__shadow_obs_{}_{}",
                        sanitize_ident(&inst_name),
                        sanitize_ident(sname)
                    );
                    if opts.expose_obs_outputs {
                        add_unique_port(
                            &mut modules[top_idx],
                            &top_act_obs,
                            Dir::Out,
                            sty,
                            Vec::new(),
                        );
                        add_unique_port(
                            &mut modules[top_idx],
                            &top_shd_obs,
                            Dir::Out,
                            sty,
                            Vec::new(),
                        );
                    } else {
                        add_unique_signal(&mut modules[top_idx], &top_act_obs, sty, None);
                        add_unique_signal(&mut modules[top_idx], &top_shd_obs, sty, None);
                    }
                    let inst_mut = modules[top_idx]
                        .instances
                        .iter_mut()
                        .find(|i| i.name == inst_name)
                        .unwrap();
                    inst_mut.conns.push(Conn {
                        port: obs_act_p,
                        sig: top_act_obs.clone(),
                        elem: Vec::new(),
                    });
                    inst_mut.conns.push(Conn {
                        port: obs_shd_p,
                        sig: top_shd_obs.clone(),
                        elem: Vec::new(),
                    });

                    retained_regs.push(LoweredRetentionReg {
                        strategy: ret.name.clone(),
                        domain: dom_name.clone(),
                        instance: inst_name.clone(),
                        specialized_module: spec_mod_name.clone(),
                        reg_name: sname.clone(),
                        width: sty.w,
                        use_retention_as_primary: false,
                        active_obs_sig: top_act_obs,
                        shadow_obs_sig: top_shd_obs,
                    });
                } else {
                    retained_regs.push(LoweredRetentionReg {
                        strategy: ret.name.clone(),
                        domain: dom_name.clone(),
                        instance: inst_name.clone(),
                        specialized_module: spec_mod_name.clone(),
                        reg_name: sname.clone(),
                        width: sty.w,
                        use_retention_as_primary: true,
                        active_obs_sig: String::new(),
                        shadow_obs_sig: String::new(),
                    });
                }
            } else {
                non_retained_regs.push((inst_name.clone(), sname.clone(), sty.w));
            }
        }
        if !child_obs_comb.is_empty() {
            modules[child_idx].processes.push(Process {
                name: "upf__obs_p".into(),
                kind: ProcKind::Comb,
                locals: Vec::new(),
                body: child_obs_comb,
                merged_from: 1,
                line: 0,
            });
        }

        // Instrument every `ProcKind::Seq` process in `modules[child_idx]`.
        let child_ports_sigs: BTreeMap<String, (Ty, Vec<u32>)> = modules[child_idx]
            .ports
            .iter()
            .map(|p| (p.name.clone(), (p.ty, p.dims.clone())))
            .chain(
                modules[child_idx]
                    .signals
                    .iter()
                    .map(|s| (s.name.clone(), (s.ty, s.dims.clone()))),
            )
            .collect();

        for proc in &mut modules[child_idx].processes {
            let ProcKind::Seq { edges } = &mut proc.kind else {
                continue;
            };
            if let Some(root_e) = edges.first().cloned() {
                *edges = vec![Edge {
                    sig: root_e.sig,
                    posedge: root_e.posedge,
                    is_async_ctrl: false,
                }];
            }
            let mut p_writes = BTreeSet::new();
            Module::writes(&proc.body, &mut p_writes);
            let orig_body = std::mem::take(&mut proc.body);

            let mut shadow_updates = Vec::new();
            let mut off_updates = Vec::new();
            let mut restore_overrides = Vec::new();

            for sname in &p_writes {
                if sname.starts_with("svcc__") || sname.starts_with("upf__") {
                    continue;
                }
                let Some((sty, sdims)) = child_ports_sigs.get(sname) else {
                    continue;
                };
                if sdims.len() > 1 {
                    continue;
                }
                let corrupt_p = format!("upf__corrupt_{}", sanitize_ident(sname));
                let assign_corrupt = |dst: &mut Vec<Stmt>| {
                    if sdims.is_empty() {
                        dst.push(assign_sig_ty(
                            sname,
                            *sty,
                            Expr::Sig {
                                name: corrupt_p.clone(),
                                ty: *sty,
                            },
                            true,
                        ));
                    } else if let [n] = sdims.as_slice() {
                        for idx in 0..*n {
                            let ix = Expr::konst(Ty::unsigned(32), idx as u64);
                            dst.push(Stmt::Assign {
                                lhs: Lval {
                                    base: LvBase::Elem {
                                        name: sname.clone(),
                                        idx: vec![ix.clone()],
                                        ty: *sty,
                                    },
                                    sel: None,
                                },
                                rhs: Expr::Elem {
                                    name: corrupt_p.clone(),
                                    idx: vec![ix],
                                    ty: *sty,
                                },
                                nonblocking: true,
                            });
                        }
                    }
                };

                match child_ret_map.get(sname) {
                    None => assign_corrupt(&mut off_updates),
                    Some(ret) if ret.use_retention_as_primary => {
                        let p_on = format!("upf__ret_{}_on", sanitize_ident(&ret.name));
                        let mut c_stmts = Vec::new();
                        assign_corrupt(&mut c_stmts);
                        off_updates.push(Stmt::If {
                            cond: not1(sig1(&p_on)),
                            then_s: c_stmts,
                            else_s: Vec::new(),
                        });
                    }
                    Some(ret) if sdims.is_empty() => {
                        let r_id = sanitize_ident(&ret.name);
                        let p_on = sig1(&format!("upf__ret_{r_id}_on"));
                        let p_sv = sig1(&format!("upf__ret_{r_id}_save"));
                        let p_rs = sig1(&format!("upf__ret_{r_id}_restore"));
                        let shadow_s = format!("upf__shadow_{}", sanitize_ident(sname));
                        let cr_p = Expr::Sig {
                            name: format!("upf__corrupt_ret_{}", sanitize_ident(sname)),
                            ty: *sty,
                        };
                        let active_e = Expr::Sig {
                            name: sname.clone(),
                            ty: *sty,
                        };

                        // Shadow update:
                        // if (!ret_on || (save && restore)) shadow <= corrupt_ret;
                        // else if (save) shadow <= dom_on ? r : corrupt_ret;
                        shadow_updates.push(Stmt::If {
                            cond: or1(not1(p_on.clone()), and1(p_sv.clone(), p_rs.clone())),
                            then_s: vec![assign_sig_ty(&shadow_s, *sty, cr_p.clone(), true)],
                            else_s: vec![Stmt::If {
                                cond: p_sv.clone(),
                                then_s: vec![Stmt::If {
                                    cond: sig1("upf__dom_on"),
                                    then_s: vec![assign_sig_ty(&shadow_s, *sty, active_e, true)],
                                    else_s: vec![assign_sig_ty(&shadow_s, *sty, cr_p, true)],
                                }],
                                else_s: Vec::new(),
                            }],
                        });

                        // Active flop collapses when primary power is OFF.
                        assign_corrupt(&mut off_updates);

                        // On wakeup `restore` while `dom_on`: restore shadow (or corrupt on hazard).
                        let mut c_stmts = Vec::new();
                        assign_corrupt(&mut c_stmts);
                        restore_overrides.push(Stmt::If {
                            cond: p_rs,
                            then_s: vec![Stmt::If {
                                cond: or1(p_sv, not1(p_on)),
                                then_s: c_stmts,
                                else_s: vec![assign_sig_ty(
                                    sname,
                                    *sty,
                                    Expr::Sig {
                                        name: shadow_s,
                                        ty: *sty,
                                    },
                                    true,
                                )],
                            }],
                            else_s: Vec::new(),
                        });
                    }
                    Some(_) => assign_corrupt(&mut off_updates),
                }
            }

            let mut on_branch = orig_body;
            on_branch.extend(restore_overrides);

            let mut new_body = shadow_updates;
            new_body.push(Stmt::If {
                cond: not1(sig1("upf__dom_on")),
                then_s: off_updates,
                else_s: on_branch,
            });
            proc.body = new_body;
        }

        // Insert boundary isolation / off-domain float muxes on all functional outputs of `inst_name`.
        let child_out_ports: BTreeSet<String> = modules[child_idx]
            .ports
            .iter()
            .filter(|p| {
                p.dir == Dir::Out
                    && p.name != "svcc__ub"
                    && !p.name.starts_with("upf__tap_")
                    && !p.name.starts_with("upf__ret_obs_")
                    && !p.name.starts_with("upf__shadow_obs_")
            })
            .map(|p| p.name.clone())
            .collect();

        let conns_snapshot = modules[top_idx]
            .instances
            .iter()
            .find(|i| i.name == inst_name)
            .unwrap()
            .conns
            .clone();

        for conn in conns_snapshot {
            if !child_out_ports.contains(&conn.port) || !conn.elem.is_empty() {
                continue;
            }
            let Some((sty, sdims, _)) = find_sig_or_port_ty(&modules[top_idx], &conn.sig) else {
                continue;
            };
            if !sdims.is_empty() {
                continue;
            }
            let orig_sig = conn.sig.clone();
            let raw_sig = format!(
                "upf__raw_{}_{}",
                sanitize_ident(&inst_name),
                sanitize_ident(&conn.port)
            );
            let float_in = format!(
                "upf__float_{}_{}",
                sanitize_ident(&inst_name),
                sanitize_ident(&conn.port)
            );
            add_unique_signal(&mut modules[top_idx], &raw_sig, sty, None);
            add_unique_port(&mut modules[top_idx], &float_in, Dir::In, sty, Vec::new());
            corrupt_inputs.push(float_in.clone());

            // Redirect child connection to `raw_sig`.
            if let Some(c_mut) = modules[top_idx]
                .instances
                .iter_mut()
                .find(|i| i.name == inst_name)
                .and_then(|i| i.conns.iter_mut().find(|c| c.port == conn.port))
            {
                c_mut.sig = raw_sig.clone();
            }

            let cands = vec![orig_sig.clone(), format!("{inst_name}.{}", conn.port)];
            let matched_iso: Option<&UpfIsolation> = intent.isolations.values().find(|iso| {
                iso.domain == dom_name
                    && !matches!(iso.applies_to, AppliesTo::Inputs)
                    && matches_element_list(&iso.elements, &iso.exclude_elements, &cands)
            });

            let raw_e = Expr::Sig {
                name: raw_sig.clone(),
                ty: sty,
            };
            let float_e = Expr::Sig {
                name: float_in.clone(),
                ty: sty,
            };
            let uniso_e = Expr::Cond {
                cond: Box::new(sig1(&d_on_top)),
                then_e: Box::new(raw_e.clone()),
                else_e: Box::new(float_e),
                ty: sty,
            };

            let (mux_e, iso_name_str, act_opt, clamp_opt) = match matched_iso {
                Some(iso) if !iso.no_isolation => {
                    let act_sig = iso_active_sigs[&iso.name].clone();
                    domain_has_isolated_crossing
                        .entry(dom_name.clone())
                        .or_default()
                        .push(act_sig.clone());
                    let clamp_e = match iso.clamp_value {
                        ClampValue::Zero | ClampValue::HighZ => Expr::konst(sty, 0),
                        ClampValue::One => all_ones(sty),
                        ClampValue::Latch => {
                            let latch_s = format!("upf__iso_latch_{}", sanitize_ident(&orig_sig));
                            add_unique_signal(&mut modules[top_idx], &latch_s, sty, None);
                            pwr_seq_stmts.push(Stmt::If {
                                cond: and1(sig1(&d_on_top), not1(sig1(&act_sig))),
                                then_s: vec![assign_sig_ty(&latch_s, sty, raw_e, true)],
                                else_s: Vec::new(),
                            });
                            Expr::Sig {
                                name: latch_s,
                                ty: sty,
                            }
                        }
                    };
                    let e = Expr::Cond {
                        cond: Box::new(sig1(&act_sig)),
                        then_e: Box::new(clamp_e),
                        else_e: Box::new(uniso_e),
                        ty: sty,
                    };
                    (e, iso.name.clone(), Some(act_sig), Some(iso.clamp_value))
                }
                Some(iso) => (uniso_e, iso.name.clone(), None, None),
                None => (uniso_e, "<unisolated>".into(), None, None),
            };

            pwr_comb_stmts.push(assign_sig_ty(&orig_sig, sty, mux_e, false));
            isolation_summaries.push(LoweredIsolationSummary {
                strategy: iso_name_str,
                domain: dom_name.clone(),
                signal: orig_sig,
                raw_signal: raw_sig,
                float_input: float_in,
                iso_active_sig: act_opt,
                clamp: clamp_opt,
            });
        }
    }

    // ------------------------------------------------------------------
    // 5. Protocol Violation Flags & SVA Property Generation
    // ------------------------------------------------------------------
    let not_in_rst = match &rst_info {
        Some((rst_e, _)) => not1(rst_e.clone()),
        None => bit(1),
    };
    let mut viol_terms: Vec<Expr> = Vec::new();
    let mut pst_viol_sigs: BTreeMap<String, (String, Vec<(String, Expr)>)> = BTreeMap::new();

    for (dom_name, act_sigs) in &domain_has_isolated_crossing {
        let d_on = sig1(&dom_on_sig[dom_name]);
        let mut uniq_acts = act_sigs.clone();
        uniq_acts.sort();
        uniq_acts.dedup();
        let mut all_acts = bit(1);
        for act in uniq_acts {
            viol_terms.push(and1(
                not_in_rst.clone(),
                and1(not1(d_on.clone()), not1(sig1(&act))),
            ));
            all_acts = and1(all_acts, sig1(&act));
        }
        if opts.expose_violation_port {
            let off_iso_port = format!("upf__off_isolated_{}", sanitize_ident(dom_name));
            add_unique_port(
                &mut modules[top_idx],
                &off_iso_port,
                Dir::Out,
                Ty::BIT,
                Vec::new(),
            );
            pwr_comb_stmts.push(assign_sig_ty(
                &off_iso_port,
                Ty::BIT,
                and1(not_in_rst.clone(), and1(not1(d_on), all_acts)),
                false,
            ));
        }
    }
    for dom in intent.domains.values() {
        if dom_switchable.get(&dom.name).copied().unwrap_or(false)
            && !domain_has_isolated_crossing.contains_key(&dom.name)
            && opts.expose_violation_port
        {
            let d_on = sig1(&dom_on_sig[&dom.name]);
            let off_iso_port = format!("upf__off_isolated_{}", sanitize_ident(&dom.name));
            add_unique_port(
                &mut modules[top_idx],
                &off_iso_port,
                Dir::Out,
                Ty::BIT,
                Vec::new(),
            );
            pwr_comb_stmts.push(assign_sig_ty(
                &off_iso_port,
                Ty::BIT,
                and1(not_in_rst.clone(), not1(d_on)),
                false,
            ));
        }
    }

    for ret in intent.retentions.values() {
        if ret.use_retention_as_primary {
            continue;
        }
        let Some(d_on_s) = dom_on_sig.get(&ret.domain) else {
            continue;
        };
        let rc = &ret_ctrls[&ret.name];
        if let (Some(sv), Some(rs)) = (&rc.save_trig_sig, &rc.restore_trig_sig) {
            let d_on = sig1(d_on_s);
            let r_on = sig1(&rc.ret_on_sig);
            let bad_save = and1(sig1(sv), or1(not1(d_on.clone()), not1(r_on.clone())));
            let bad_restore = and1(sig1(rs), or1(not1(d_on), not1(r_on)));
            let hazard = and1(sig1(sv), sig1(rs));
            viol_terms.push(and1(
                not_in_rst.clone(),
                or1(bad_save, or1(bad_restore, hazard)),
            ));
        }
    }

    for pst in intent.psts.values() {
        let mut row_exprs: Vec<(String, Expr)> = Vec::new();
        for (st_name, tuple) in &pst.states {
            if tuple.len() != pst.supplies.len() {
                continue;
            }
            let mut row_ok = bit(1);
            for (sup, pstate) in pst.supplies.iter().zip(tuple) {
                let col_e = port_state_exprs
                    .get(sup)
                    .and_then(|m| m.get(pstate))
                    .cloned()
                    .unwrap_or_else(|| bit(0));
                row_ok = and1(row_ok, col_e);
            }
            let row_sig = format!(
                "upf__pst_{}_{}",
                sanitize_ident(&pst.name),
                sanitize_ident(st_name)
            );
            add_unique_signal(&mut modules[top_idx], &row_sig, Ty::BIT, None);
            pwr_comb_stmts.push(assign_sig_ty(&row_sig, Ty::BIT, row_ok, false));
            row_exprs.push((st_name.clone(), sig1(&row_sig)));
        }
        if !row_exprs.is_empty() {
            let legal_any = row_exprs
                .iter()
                .map(|(_, e)| e.clone())
                .reduce(or1)
                .unwrap_or_else(|| bit(1));
            let v_sig = format!("upf__viol_pst_{}", sanitize_ident(&pst.name));
            add_unique_signal(&mut modules[top_idx], &v_sig, Ty::BIT, None);
            pwr_comb_stmts.push(assign_sig_ty(
                &v_sig,
                Ty::BIT,
                and1(not_in_rst.clone(), not1(legal_any)),
                false,
            ));
            viol_terms.push(sig1(&v_sig));
            pst_viol_sigs.insert(pst.name.clone(), (v_sig, row_exprs));
        }
    }

    let any_viol = viol_terms.into_iter().reduce(or1).unwrap_or_else(|| bit(0));
    if opts.expose_violation_port {
        add_unique_port(
            &mut modules[top_idx],
            "upf__violation",
            Dir::Out,
            Ty::BIT,
            Vec::new(),
        );
    } else {
        add_unique_signal(&mut modules[top_idx], "upf__violation", Ty::BIT, None);
    }
    pwr_comb_stmts.push(assign_sig_ty("upf__violation", Ty::BIT, any_viol, false));

    if !pwr_comb_stmts.is_empty() {
        modules[top_idx].processes.push(Process {
            name: "upf__pwr_p".into(),
            kind: ProcKind::Comb,
            locals: Vec::new(),
            body: pwr_comb_stmts,
            merged_from: 1,
            line: 0,
        });
    }
    if !pwr_seq_stmts.is_empty() {
        if let Some(clk) = &root_clock {
            modules[top_idx].processes.push(Process {
                name: "upf__pwr_seq_p".into(),
                kind: ProcKind::Seq {
                    edges: vec![Edge {
                        sig: clk.clone(),
                        posedge: true,
                        is_async_ctrl: false,
                    }],
                },
                locals: Vec::new(),
                body: pwr_seq_stmts,
                merged_from: 1,
                line: 0,
            });
        }
    }

    // Build IEEE 1800 SVA block when requested and a clock is present.
    let mut sva = String::new();
    if opts.emit_sva {
        if let Some(clk) = &root_clock {
            let dis = match &rst_info {
                Some((_, expr_sv)) => format!(" disable iff ({expr_sv})"),
                None => String::new(),
            };
            sva.push_str("  // Auto-synthesized IEEE 1801 UPF Formal Properties (svcc --upf)\n");
            for (dom_name, act_sigs) in &domain_has_isolated_crossing {
                let d_on = &dom_on_sig[dom_name];
                let mut uniq_acts = act_sigs.clone();
                uniq_acts.sort();
                uniq_acts.dedup();
                for act in uniq_acts {
                    let tag = format!("{}_{}", sanitize_ident(dom_name), sanitize_ident(&act));
                    let _ = writeln!(
                        sva,
                        "  upf_iso_when_off_{tag}: assert property (@(posedge {clk}){dis} (!{d_on}) |-> {act});"
                    );
                    let _ = writeln!(
                        sva,
                        "  upf_cov_off_isolated_{tag}: cover property (@(posedge {clk}){dis} (!{d_on} && {act}));"
                    );
                }
            }
            for ret in intent.retentions.values() {
                if ret.use_retention_as_primary {
                    continue;
                }
                let Some(d_on) = dom_on_sig.get(&ret.domain) else {
                    continue;
                };
                let rc = &ret_ctrls[&ret.name];
                let r_on = &rc.ret_on_sig;
                let rtag = sanitize_ident(&ret.name);
                if let (Some(sv), Some(rs)) = (&rc.save_trig_sig, &rc.restore_trig_sig) {
                    let _ = writeln!(
                        sva,
                        "  upf_ret_save_valid_{rtag}: assert property (@(posedge {clk}){dis} {sv} |-> ({d_on} && {r_on}));"
                    );
                    let _ = writeln!(
                        sva,
                        "  upf_ret_restore_valid_{rtag}: assert property (@(posedge {clk}){dis} {rs} |-> ({d_on} && {r_on}));"
                    );
                    let _ = writeln!(
                        sva,
                        "  upf_ret_no_hazard_{rtag}: assert property (@(posedge {clk}){dis} !({sv} && {rs}));"
                    );
                    let _ = writeln!(
                        sva,
                        "  upf_cov_ret_cycle_{rtag}: cover property (@(posedge {clk}){dis} ({sv} && {d_on}) ##[1:4] (!{d_on}) ##[1:4] ({rs} && {d_on}));"
                    );
                }
            }
            for rr in &retained_regs {
                if rr.use_retention_as_primary || rr.active_obs_sig.is_empty() {
                    continue;
                }
                let d_on = &dom_on_sig[&rr.domain];
                let rc = &ret_ctrls[&rr.strategy];
                let r_on = &rc.ret_on_sig;
                if let (Some(sv), Some(rs)) = (&rc.save_trig_sig, &rc.restore_trig_sig) {
                    let tag = format!(
                        "{}_{}",
                        sanitize_ident(&rr.instance),
                        sanitize_ident(&rr.reg_name)
                    );
                    let act = &rr.active_obs_sig;
                    let shd = &rr.shadow_obs_sig;
                    let _ = writeln!(
                        sva,
                        "  upf_ret_save_captures_{tag}: assert property (@(posedge {clk}){dis} ({d_on} && {r_on} && {sv} && !{rs}) |=> ({shd} == $past({act})));"
                    );
                    let _ = writeln!(
                        sva,
                        "  upf_ret_shadow_holds_off_{tag}: assert property (@(posedge {clk}){dis} (!{d_on} && {r_on} && !{sv} && !{rs}) |=> ({shd} == $past({shd})));"
                    );
                    let _ = writeln!(
                        sva,
                        "  upf_ret_restore_exact_{tag}: assert property (@(posedge {clk}){dis} ({d_on} && {r_on} && {rs} && !{sv}) |=> ({act} == $past({shd})));"
                    );
                }
            }
            for (pst_name, (v_sig, rows)) in &pst_viol_sigs {
                let ptag = sanitize_ident(pst_name);
                let _ = writeln!(
                    sva,
                    "  upf_pst_legal_{ptag}: assert property (@(posedge {clk}){dis} !{v_sig});"
                );
                for (st_name, _) in rows {
                    let stag = sanitize_ident(st_name);
                    let rsig = format!("upf__pst_{ptag}_{stag}");
                    let _ = writeln!(
                        sva,
                        "  upf_cov_pst_{ptag}_{stag}: cover property (@(posedge {clk}){dis} {rsig});"
                    );
                }
            }
        }
    }

    let lowered_design = Design {
        modules,
        tops: vec![top_name.clone()],
        notes: design.notes.clone(),
    };

    Ok(UpfLowerReport {
        top: top_name,
        root_clock,
        reset_disable_sv: rst_info.map(|(_, s)| s),
        design: lowered_design,
        static_findings,
        domains: domain_summaries,
        isolations: isolation_summaries,
        retained_regs,
        non_retained_regs,
        corrupt_inputs,
        supply_on_inputs,
        sva_block: sva,
    })
}
