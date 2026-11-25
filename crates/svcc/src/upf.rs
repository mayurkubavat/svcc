//! End-to-End IEEE 1801 UPF Formal Lowering, SV+SVA Emission, and Btor2+Z3 Prover (`svcc::upf`).

use crate::prove::{design_to_btor2, UB_FLAG};
use serde::Serialize;
use std::path::Path;
use svcc_equiv::pair::PairSide;
use svcc_equiv::upf_miter::{
    prove_isolation_non_interference, prove_retention_roundtrip, prove_upf_protocol_bmc,
    IsolationNonInterferenceReport, ProtocolBmcReport, RetentionRoundTripReport, UpfCheckVerdict,
};
use svcc_ir::hdl::{Design, Dir};
use svcc_ir::upf::{parse_upf_file, parse_upf_str, UpfDesignIntent};
use svcc_ir::SvccError;
use svcc_passes::upf_lower::{
    collect_design_objects, lower_design_with_upf, LoweredDomainSummary, LoweredIsolationSummary,
    LoweredRetentionReg, UpfLowerOptions, UpfLowerReport, UpfSeverity, UpfStaticFinding,
};

const STAGE: &str = "svcc/upf";

#[derive(Clone, Debug)]
pub struct UpfProveOptions {
    pub top: Option<String>,
    pub bound: u32,
    pub hold: Vec<(String, u64)>,
    pub require_protocol_bmc: bool,
    pub timeout_ms: u32,
}

impl Default for UpfProveOptions {
    fn default() -> Self {
        Self {
            top: None,
            bound: 6,
            hold: Vec::new(),
            require_protocol_bmc: false,
            timeout_ms: 120_000,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UpfProveReport {
    pub top: String,
    pub source_is_cpp: bool,
    pub static_findings: Vec<UpfStaticFinding>,
    pub domains: Vec<LoweredDomainSummary>,
    pub isolations: Vec<LoweredIsolationSummary>,
    pub retained_regs: Vec<LoweredRetentionReg>,
    pub cpp_ub_bmc: Option<ProtocolBmcReport>,
    pub isolation_non_interference: Vec<IsolationNonInterferenceReport>,
    pub retention_roundtrips: Vec<RetentionRoundTripReport>,
    pub protocol_bmc: ProtocolBmcReport,
}

impl UpfProveReport {
    pub fn has_static_errors(&self) -> bool {
        self.static_findings
            .iter()
            .any(|f| f.severity == UpfSeverity::Error)
    }

    pub fn all_passed(&self, require_protocol_bmc: bool) -> bool {
        if self.has_static_errors() {
            return false;
        }
        if let Some(ub) = &self.cpp_ub_bmc {
            if ub.verdict != UpfCheckVerdict::ProvedUnsat {
                return false;
            }
        }
        if self
            .isolation_non_interference
            .iter()
            .any(|r| r.verdict != UpfCheckVerdict::ProvedUnsat)
        {
            return false;
        }
        if self
            .retention_roundtrips
            .iter()
            .any(|r| r.verdict != UpfCheckVerdict::ProvedUnsat)
        {
            return false;
        }
        if require_protocol_bmc && self.protocol_bmc.verdict != UpfCheckVerdict::ProvedUnsat {
            return false;
        }
        true
    }
}

fn is_sv_file(path: &str) -> bool {
    path.ends_with(".sv") || path.ends_with(".v") || path.ends_with(".svh")
}

/// Parses an RTL design (`.sv`/`.v` or C++17 SystemC `SC_MODULE` `.h`/`.hpp`) into `hdl::Design`.
pub fn parse_rtl_design(
    input_path: &str,
    src: &str,
    top: Option<&str>,
) -> Result<(Design, bool), SvccError> {
    if is_sv_file(input_path) {
        let tops: Vec<String> = top.map(str::to_string).into_iter().collect();
        let d = svcc_front_sv::hdl_lower::lower_hdl_design(input_path, src, &tops)?;
        Ok((d, false))
    } else {
        let mut d = svcc_front_sc::hdl_lift::lift_hdl_modules(src)?;
        if let Some(t) = top {
            if d.module(t).is_none() {
                return Err(SvccError::new(
                    STAGE,
                    format!("top module '{t}' not found in {input_path}"),
                ));
            }
            d.tops = vec![t.to_string()];
        }
        Ok((d, true))
    }
}

/// Elaborates `input_path` + `upf_path` into a power-instrumented [`UpfLowerReport`].
pub fn lower_with_upf_file(
    input_path: &str,
    src: &str,
    upf_path: &Path,
    opts: &UpfLowerOptions,
) -> Result<(Design, bool, UpfDesignIntent, UpfLowerReport), SvccError> {
    let (base, is_cpp) = parse_rtl_design(input_path, src, opts.top.as_deref())?;
    let objs = collect_design_objects(&base, opts.top.as_deref());
    let intent = parse_upf_file(upf_path, &objs)?;
    let rep = lower_design_with_upf(&base, &intent, opts)?;
    Ok((base, is_cpp, intent, rep))
}

/// Elaborates in-memory RTL + UPF strings into a power-instrumented [`UpfLowerReport`].
pub fn lower_with_upf_str(
    input_path: &str,
    src: &str,
    upf_src: &str,
    upf_base_dir: Option<&Path>,
    opts: &UpfLowerOptions,
) -> Result<(Design, bool, UpfDesignIntent, UpfLowerReport), SvccError> {
    let (base, is_cpp) = parse_rtl_design(input_path, src, opts.top.as_deref())?;
    let objs = collect_design_objects(&base, opts.top.as_deref());
    let intent = parse_upf_str(upf_src, upf_base_dir, &objs)?;
    let rep = lower_design_with_upf(&base, &intent, opts)?;
    Ok((base, is_cpp, intent, rep))
}

fn inject_sva_into_top_sv(sv: &str, top: &str, sva_block: &str) -> Result<String, SvccError> {
    if sva_block.trim().is_empty() {
        return Ok(sv.to_string());
    }
    let marker = format!("module {top} (");
    let Some(mod_pos) = sv.find(&marker) else {
        return Err(SvccError::new(
            STAGE,
            format!("emitted SystemVerilog does not contain '{marker}'"),
        ));
    };
    let tail = &sv[mod_pos..];
    let Some(end_rel) = tail.find("\nendmodule") else {
        return Err(SvccError::new(
            STAGE,
            format!("emitted module '{top}' missing endmodule"),
        ));
    };
    let insert_at = mod_pos + end_rel + 1;
    let mut out = String::with_capacity(sv.len() + sva_block.len() + 8);
    out.push_str(&sv[..insert_at]);
    out.push_str(sva_block);
    out.push_str(&sv[insert_at..]);
    Ok(out)
}

/// Emits power-instrumented explicit-width SystemVerilog (with optional IEEE 1800 SVA monitors
/// for `InFormal`) from pure SystemC (`SC_MODULE`) or SystemVerilog RTL + `.upf`.
pub fn emit_upf_sv_file(
    input_path: &str,
    src: &str,
    upf_path: &Path,
    top: Option<String>,
    emit_sva: bool,
) -> Result<String, SvccError> {
    let opts = UpfLowerOptions {
        top,
        expose_violation_port: true,
        expose_obs_outputs: false,
        emit_sva,
    };
    let (_, _, _, rep) = lower_with_upf_file(input_path, src, upf_path, &opts)?;
    if rep.has_static_errors() {
        let msgs: Vec<String> = rep
            .static_findings
            .iter()
            .filter(|f| f.severity == UpfSeverity::Error)
            .map(|f| format!("{}: {}", f.code, f.message))
            .collect();
        return Err(SvccError::new(STAGE, msgs.join("; ")));
    }
    let raw_sv = svcc_emit_sv::hdl_emit::emit_hdl_design_sv(&rep.design, "")?;
    inject_sva_into_top_sv(&raw_sv, &rep.top, &rep.sva_block)
}

/// Emits power-instrumented C++17 SystemC from SystemVerilog/SystemC RTL + `.upf`.
pub fn emit_upf_sc_file(
    input_path: &str,
    src: &str,
    upf_path: &Path,
    top: Option<String>,
) -> Result<String, SvccError> {
    let opts = UpfLowerOptions {
        top,
        expose_violation_port: true,
        expose_obs_outputs: false,
        emit_sva: false,
    };
    let (_, _, _, rep) = lower_with_upf_file(input_path, src, upf_path, &opts)?;
    if rep.has_static_errors() {
        let msgs: Vec<String> = rep
            .static_findings
            .iter()
            .filter(|f| f.severity == UpfSeverity::Error)
            .map(|f| format!("{}: {}", f.code, f.message))
            .collect();
        return Err(SvccError::new(STAGE, msgs.join("; ")));
    }
    svcc_emit_sc::hdl_emit::emit_hdl_design(&rep.design)
}

/// Runs all formal UPF verification gates on `input_path` + `upf_path` via Btor2 + Z3.
pub fn prove_upf_file(
    input_path: &str,
    src: &str,
    upf_path: &Path,
    opts: &UpfProveOptions,
) -> Result<UpfProveReport, SvccError> {
    let lopts = UpfLowerOptions {
        top: opts.top.clone(),
        expose_violation_port: true,
        expose_obs_outputs: false,
        emit_sva: false,
    };
    let (_, is_cpp, _, rep) = lower_with_upf_file(input_path, src, upf_path, &lopts)?;
    run_upf_proofs(is_cpp, rep, opts)
}

/// In-memory variant of [`prove_upf_file`] for unit and regression tests.
pub fn prove_upf_str(
    input_path: &str,
    src: &str,
    upf_src: &str,
    upf_base_dir: Option<&Path>,
    opts: &UpfProveOptions,
) -> Result<UpfProveReport, SvccError> {
    let lopts = UpfLowerOptions {
        top: opts.top.clone(),
        expose_violation_port: true,
        expose_obs_outputs: false,
        emit_sva: false,
    };
    let (_, is_cpp, _, rep) = lower_with_upf_str(input_path, src, upf_src, upf_base_dir, &lopts)?;
    run_upf_proofs(is_cpp, rep, opts)
}

fn sanitize_id(s: &str) -> String {
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

fn run_upf_proofs(
    is_cpp: bool,
    rep: UpfLowerReport,
    opts: &UpfProveOptions,
) -> Result<UpfProveReport, SvccError> {
    let top_mod = rep
        .design
        .module(&rep.top)
        .ok_or_else(|| SvccError::new(STAGE, "lowered top module missing"))?;

    let func_outputs: Vec<String> = top_mod
        .ports
        .iter()
        .filter(|p| {
            p.dir == Dir::Out
                && p.name != UB_FLAG
                && !p.name.starts_with("upf__")
                && p.dims.is_empty()
        })
        .map(|p| p.name.clone())
        .collect();

    let top_btor = design_to_btor2(&rep.design, &rep.top, "svcc_upf__")?;
    let pref_top = format!("svcc_upf__{}", rep.top);
    let top_side = PairSide {
        btor2: &top_btor.btor2,
        top: &pref_top,
        outputs: &top_btor.outputs,
    };

    // 1. If lifted from C++17 SystemC, prove `svcc__ub == 0` under UPF lowering.
    let cpp_ub_bmc = if is_cpp && top_mod.ports.iter().any(|p| p.name == UB_FLAG) {
        Some(prove_upf_protocol_bmc(
            &top_side,
            UB_FLAG,
            &opts.hold,
            opts.bound,
            opts.timeout_ms,
        )?)
    } else {
        None
    };

    // 2. 2-Copy Self-Composition Isolation Non-Interference for every switchable domain.
    let mut iso_reports = Vec::new();
    for dom in &rep.domains {
        if !dom.switchable {
            continue;
        }
        let cond_port = format!("upf__off_isolated_{}", sanitize_id(&dom.name));
        if !top_mod.ports.iter().any(|p| p.name == cond_port) {
            continue;
        }
        iso_reports.push(prove_isolation_non_interference(
            &top_side,
            &dom.name,
            &dom.instances,
            &rep.corrupt_inputs,
            &cond_port,
            &func_outputs,
            opts.timeout_ms,
        )?);
    }

    // 3. Universal Symbolic Retention Save-Collapse-Wake-Restore Round-Trip Proofs.
    let mut ret_reports = Vec::new();
    for rr in &rep.retained_regs {
        // Build a sub-design rooted at `rr.specialized_module`.
        let mut sub_design = rep.design.clone();
        sub_design.tops = vec![rr.specialized_module.clone()];
        let child_btor = design_to_btor2(&sub_design, &rr.specialized_module, "svcc_ret__")?;
        let child_top = format!("svcc_ret__{}", rr.specialized_module);
        let child_side = PairSide {
            btor2: &child_btor.btor2,
            top: &child_top,
            outputs: &child_btor.outputs,
        };
        ret_reports.push(prove_retention_roundtrip(
            &child_side,
            &rr.instance,
            &rr.reg_name,
            &rr.strategy,
            rr.use_retention_as_primary,
            true,
            opts.timeout_ms,
        )?);
    }

    // 4. Bounded UPF Protocol Check (`upf__violation == 0`).
    let protocol_bmc = prove_upf_protocol_bmc(
        &top_side,
        "upf__violation",
        &opts.hold,
        opts.bound,
        opts.timeout_ms,
    )?;

    Ok(UpfProveReport {
        top: rep.top,
        source_is_cpp: is_cpp,
        static_findings: rep.static_findings,
        domains: rep.domains,
        isolations: rep.isolations,
        retained_regs: rep.retained_regs,
        cpp_ub_bmc,
        isolation_non_interference: iso_reports,
        retention_roundtrips: ret_reports,
        protocol_bmc,
    })
}
