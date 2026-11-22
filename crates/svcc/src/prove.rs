//! Formal gates of the typed RTL path.
//!
//! `prove_rtl` checks a SystemVerilog design against the SystemC that `svcc rtl2sc`
//! generates for it. The reference is always the *source text*, lowered by the `svlang`
//! netlist on its own, never something derived from svcc's IR:
//!
//! 1. **lowering**: the typed IR, written back as explicit-width SystemVerilog, against the
//!    source. Finds an IR that does not mean what the source means.
//! 2. **generated C++**: the generated header is read back by `svcc_front_sc::hdl_lift`,
//!    which applies C++ and SystemC evaluation rules (integer promotion, 64-bit `sc_uint`
//!    arithmetic, truncation only at constructions) and reports every operation whose result
//!    C++ leaves undefined. The lifted design, written as explicit-width SystemVerilog,
//!    against the source. Finds generated code that evaluates at the wrong width.
//!
//! See `svcc_equiv::pair` for what each verdict means.

use serde::Serialize;
use svcc_equiv::pair::{prove_btor2_pair, PairOptions, PairReport, PairSide, PairVerdict};
use svcc_equiv::sec::Btor2Design;
use svcc_ir::hdl::{Design, Dir};
use svcc_ir::SvccError;

/// Name of the 1-bit output a lifted module raises when it executes an operation that C++
/// leaves undefined.
pub const UB_FLAG: &str = "svcc__ub";

#[derive(Clone, Debug, Serialize)]
pub struct RtlProveReport {
    pub top: String,
    /// IR against source.
    pub lowering: PairReport,
    /// Generated C++ against source. `None` when the header could not be read back.
    pub generated_cpp: Option<PairReport>,
    /// Why the generated C++ was not checked.
    pub cpp_unchecked: Option<String>,
}

impl RtlProveReport {
    /// The generated C++ is proved equal to the source (bounded or unbounded).
    pub fn cpp_ok(&self) -> bool {
        self.generated_cpp.as_ref().is_some_and(|r| {
            matches!(
                r.verdict,
                PairVerdict::ProvedInductive | PairVerdict::BoundedNoDifference
            )
        })
    }
}

#[derive(Clone, Debug)]
pub struct RtlProveOptions {
    pub top: Option<String>,
    /// Check this header instead of the one `rtl2sc` generates now (a hand-edited or
    /// previously generated file).
    pub header: Option<String>,
    pub bound: u32,
    pub timeout_ms: u32,
}

impl Default for RtlProveOptions {
    fn default() -> Self {
        RtlProveOptions {
            top: None,
            header: None,
            bound: 8,
            timeout_ms: 120_000,
        }
    }
}

fn single_top(design: &Design) -> Result<&svcc_ir::hdl::Module, SvccError> {
    match design.tops.as_slice() {
        [t] => design
            .module(t)
            .ok_or_else(|| SvccError::new("svcc/prove", format!("top module '{t}' not found"))),
        [] => Err(SvccError::new("svcc/prove", "design has no top module")),
        many => Err(SvccError::new(
            "svcc/prove",
            format!(
                "design has {} top modules ({}); pick one with --top",
                many.len(),
                many.join(", ")
            ),
        )),
    }
}

/// Explicit-width SystemVerilog of `design` lowered to Btor2, top `prefix + top`.
pub fn design_to_btor2(design: &Design, top: &str, prefix: &str) -> Result<Btor2Design, SvccError> {
    let sv = svcc_emit_sv::hdl_emit::emit_hdl_design_sv(design, prefix)?;
    if let Ok(dir) = std::env::var("SVCC_KEEP_PROVE") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(format!("{dir}/{prefix}{top}.sv"), &sv);
    }
    svcc_equiv::sec::sv_to_btor2(&sv, &format!("{prefix}{top}.sv"), &format!("{prefix}{top}"))
}

/// Checks the SystemC generated for `sv_source` against `sv_source` itself.
pub fn prove_rtl(
    filename: &str,
    sv_source: &str,
    opts: &RtlProveOptions,
) -> Result<RtlProveReport, SvccError> {
    let tops: Vec<String> = opts.top.iter().cloned().collect();
    let (code, design) = crate::sv_rtl_to_sc(filename, sv_source, &tops)?;
    let code = opts.header.clone().unwrap_or(code);
    let top = single_top(&design)?;
    // An array port is one output per element, named as the netlist names them.
    let mut outputs: Vec<String> = Vec::new();
    for p in top.ports.iter().filter(|p| p.dir == Dir::Out) {
        match p.dims.as_slice() {
            [] => outputs.push(p.name.clone()),
            [n] => outputs.extend((0..*n).map(|i| format!("{}[{i}]", p.name))),
            _ => {
                return Err(SvccError::new(
                    "svcc/prove-model",
                    format!("output '{}' is a multi-dimensional array port", p.name),
                ))
            }
        }
    }
    let spec = svcc_equiv::sec::sv_to_btor2(sv_source, filename, &top.definition)?;
    let popts = PairOptions {
        bound: opts.bound,
        timeout_ms: opts.timeout_ms,
        ..Default::default()
    };

    let ir = design_to_btor2(&design, &top.name, "svcc_ir__")?;
    let spec_side = PairSide {
        btor2: &spec.btor2,
        top: &top.definition,
        outputs: &spec.outputs,
    };
    let ir_top = format!("svcc_ir__{}", top.name);
    let lowering = prove_btor2_pair(
        &spec_side,
        &PairSide {
            btor2: &ir.btor2,
            top: &ir_top,
            outputs: &ir.outputs,
        },
        &outputs,
        &popts,
    )?;

    let (generated_cpp, cpp_unchecked) = match svcc_front_sc::hdl_lift::lift_hdl_design(&code) {
        Ok(lifted) => {
            // A module whose name is reserved in C++ is written with a trailing underscore.
            let ltop = lifted
                .module(&top.name)
                .or_else(|| lifted.module(&format!("{}_", top.name)))
                .ok_or_else(|| {
                    SvccError::new(
                        "svcc/prove",
                        format!("generated header has no module '{}'", top.name),
                    )
                })?;
            let cpp = design_to_btor2(&lifted, &ltop.name, "svcc_sc__")?;
            let copts = PairOptions {
                imp_flag: Some(UB_FLAG.to_string()),
                ..popts.clone()
            };
            let sc_top = format!("svcc_sc__{}", ltop.name);
            let r = prove_btor2_pair(
                &spec_side,
                &PairSide {
                    btor2: &cpp.btor2,
                    top: &sc_top,
                    outputs: &cpp.outputs,
                },
                &outputs,
                &copts,
            )?;
            (Some(r), None)
        }
        Err(e) => (None, Some(e.to_string())),
    };
    Ok(RtlProveReport {
        top: top.name.clone(),
        lowering,
        generated_cpp,
        cpp_unchecked,
    })
}

#[derive(Clone, Debug, Default)]
pub struct SecOptions {
    /// Top module of the implementation (and of an RTL specification) when a file has several.
    pub top: Option<String>,
    /// The implementation's outputs are compared this many clock cycles after the
    /// specification's.
    pub latency: u32,
    /// Inputs held at a constant, `name=value`. An asynchronous reset of the implementation
    /// is held inactive without being listed when `latency > 0`.
    pub hold: Vec<(String, u64)>,
    pub bound: Option<u32>,
}

fn top_of(filename: &str, src: &str, top: &Option<String>) -> Result<String, SvccError> {
    let tops: Vec<String> = top.iter().cloned().collect();
    let design = svcc_front_sv::hdl_lower::lower_hdl_design(filename, src, &tops)?;
    Ok(single_top(&design)?.definition.clone())
}

/// Inactive level of every asynchronous control of the top module, found from the edges its
/// edge-triggered processes are sensitive to.
fn async_controls(filename: &str, src: &str, top: &Option<String>) -> Vec<(String, u64)> {
    let tops: Vec<String> = top.iter().cloned().collect();
    let Ok(design) = svcc_front_sv::hdl_lower::lower_hdl_design(filename, src, &tops) else {
        return Vec::new();
    };
    let Ok(t) = single_top(&design) else {
        return Vec::new();
    };
    let mut out: Vec<(String, u64)> = Vec::new();
    for p in &t.processes {
        if let svcc_ir::hdl::ProcKind::Seq { edges } = &p.kind {
            for e in edges {
                let is_port = t.ports.iter().any(|x| x.name == e.sig && x.dir == Dir::In);
                if e.is_async_ctrl && is_port && !out.iter().any(|x| x.0 == e.sig) {
                    // Active on the edge it triggers on: posedge means active high.
                    out.push((e.sig.clone(), if e.posedge { 0 } else { 1 }));
                }
            }
        }
    }
    out
}

/// Sequential equivalence of a specification and an implementation.
///
/// - `spec` ending in `.sv` / `.v`: both sides are lowered by the `svlang` netlist and their
///   outputs (matched by name) compared.
/// - any other `spec` is C++: a function marked `// svcc:c2rtl` (an untimed golden model,
///   compared with the implementation's single output `latency` cycles later), or a header
///   of `SC_MODULE`s as `svcc rtl2sc` writes it. The C++ is read with C++ evaluation rules
///   and must not execute undefined behaviour.
pub fn prove_sec(
    spec_filename: &str,
    spec_src: &str,
    imp_filename: &str,
    imp_src: &str,
    opts: &SecOptions,
) -> Result<PairReport, SvccError> {
    let is_sv = |f: &str| f.ends_with(".sv") || f.ends_with(".v") || f.ends_with(".svh");
    if !is_sv(imp_filename) {
        return Err(SvccError::new(
            "svcc/sec",
            "the implementation must be SystemVerilog (to check generated SystemC against its source use `svcc rtl-prove`)",
        ));
    }
    let imp_top = top_of(imp_filename, imp_src, &opts.top)?;
    let imp = svcc_equiv::sec::sv_to_btor2(imp_src, imp_filename, &imp_top)?;
    let mut popts = PairOptions {
        latency: opts.latency,
        hold: opts.hold.clone(),
        ..Default::default()
    };
    if let Some(b) = opts.bound {
        popts.bound = b;
    }
    if opts.latency > 0 {
        popts.arbitrary_start = true;
        for (n, v) in async_controls(imp_filename, imp_src, &opts.top) {
            if !popts.hold.iter().any(|h| h.0 == n) {
                popts.hold.push((n, v));
            }
        }
    }
    let imp_side = PairSide {
        btor2: &imp.btor2,
        top: &imp_top,
        outputs: &imp.outputs,
    };
    let imp_outputs: Vec<String> = imp.outputs.iter().map(|o| o.0.clone()).collect();

    if is_sv(spec_filename) {
        let spec_top = top_of(spec_filename, spec_src, &opts.top)?;
        let spec = svcc_equiv::sec::sv_to_btor2(spec_src, spec_filename, &spec_top)?;
        let outputs: Vec<String> = spec.outputs.iter().map(|o| o.0.clone()).collect();
        return prove_btor2_pair(
            &PairSide {
                btor2: &spec.btor2,
                top: &spec_top,
                outputs: &spec.outputs,
            },
            &imp_side,
            &outputs,
            &popts,
        );
    }

    popts.spec_flag = Some(UB_FLAG.to_string());
    let (lifted, outputs) = if spec_src.contains("// svcc:c2rtl") {
        let [out] = imp_outputs.as_slice() else {
            return Err(SvccError::new(
                "svcc/sec",
                format!(
                    "a golden function has one result, but '{imp_top}' has {} outputs",
                    imp_outputs.len()
                ),
            ));
        };
        (
            svcc_front_sc::hdl_lift::lift_c2rtl_function(spec_src, out)?,
            vec![out.clone()],
        )
    } else {
        let d = svcc_front_sc::hdl_lift::lift_hdl_design(spec_src)?;
        let t = single_top(&d)?;
        let outs = t
            .ports
            .iter()
            .filter(|p| p.dir == Dir::Out && p.name != UB_FLAG)
            .map(|p| p.name.clone())
            .collect();
        (d, outs)
    };
    let ltop = single_top(&lifted)?.name.clone();
    let spec = design_to_btor2(&lifted, &ltop, "svcc_sc__")?;
    let sc_top = format!("svcc_sc__{ltop}");
    prove_btor2_pair(
        &PairSide {
            btor2: &spec.btor2,
            top: &sc_top,
            outputs: &spec.outputs,
        },
        &imp_side,
        &outputs,
        &popts,
    )
}

/// Outcome of [`prove_rtl_file`] for one top module.
#[derive(Clone, Debug, Serialize)]
pub enum RtlFileItem {
    Proved(Box<RtlProveReport>),
    /// Not compared, with the reason (a module without outputs has nothing to compare).
    Skipped {
        top: String,
        reason: String,
    },
    /// Outside what the proof models (more than one clock, a computed clock, a
    /// multi-dimensional array port, a construct the netlist does not lower). Neither proved
    /// nor refuted.
    NotCovered {
        top: String,
        reason: String,
    },
}

/// Runs [`prove_rtl`] on every top module of a source file. Returns an empty list for a file
/// without design modules.
pub fn prove_rtl_file(
    filename: &str,
    sv_source: &str,
    opts: &RtlProveOptions,
) -> Result<Vec<RtlFileItem>, SvccError> {
    let design = svcc_front_sv::hdl_lower::lower_hdl_design(filename, sv_source, &[])?;
    let mut out = Vec::new();
    for t in &design.tops {
        let Some(m) = design.module(t) else { continue };
        if !m.ports.iter().any(|p| p.dir == Dir::Out) {
            out.push(RtlFileItem::Skipped {
                top: m.definition.clone(),
                reason: "the module has no outputs to compare".to_string(),
            });
            continue;
        }
        let o = RtlProveOptions {
            top: Some(m.definition.clone()),
            ..opts.clone()
        };
        match prove_rtl(filename, sv_source, &o) {
            Ok(r) => out.push(RtlFileItem::Proved(Box::new(r))),
            // The netlist or the Btor2 model does not take the design: a limit of the
            // proof model, not a difference.
            Err(e) if is_model_limit(&e) => out.push(RtlFileItem::NotCovered {
                top: m.definition.clone(),
                reason: e.to_string(),
            }),
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

fn is_model_limit(e: &SvccError) -> bool {
    matches!(
        e.stage,
        "svcc-equiv/btor2" | "svcc-equiv/netlist" | "svcc/prove-model"
    )
}
