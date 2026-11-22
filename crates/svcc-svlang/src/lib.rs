//! Single adapter between `svcc` and the pinned `svlang` SDK (`svlang`, `svlang-model`,
//! `svlang-lint`).
//!
//! This is the only workspace crate whose manifest names an `svlang` crate. Every other crate
//! reaches `svlang` through the items below, so an `svlang` API change is absorbed here.
//!
//! - [`syntax`]: the parse/semantic types the SV front end walks directly (re-exported).
//! - `model` (feature `model`): the UVM/RAL design model the SV front end lifts (re-exported).
//! - `verify_emitted_sv` (feature `lint`): parse + semantic + `svlang-lint` gate for emitted SV.
//! - [`lower_sv_to_btor2_design`]: elaborate SV RTL into a word-level Btor2 transition system.

pub mod syntax {
    //! Parse, semantic and elaboration types used by `svcc-front-sv`.
    //!
    //! These are re-exported rather than wrapped: the front end walks the lossless CST
    //! ([`Tree`], [`NodeId`], [`SyntaxKind`]) node by node and reads elaborated symbols, so a
    //! wrapper would duplicate the whole `svlang` tree API.

    /// Constant evaluator for parameter values found in the CST.
    pub use svlang::semantic::oracle::TypeOracle;
    /// Root of a parsed + semantically analyzed compilation unit.
    pub use svlang::Compilation;
    /// Elaborated design, read for parameterized RTL port and signal widths.
    pub use svlang::Elaborated;
    /// Diagnostic stage and severity, used to filter front-end parse errors.
    pub use svlang::{DiagnosticStage, Severity};
    /// Lossless CST walked for constraints, covergroups, SVA and RTL bodies.
    pub use svlang::{NodeId, SyntaxKind, Tree};
}

pub mod elab {
    //! Elaborated design and bound-expression types used by the typed RTL (`hdl`) lowering in
    //! `svcc-front-sv`.
    //!
    //! Re-exported rather than wrapped: the lowering walks per-instance symbols and items and
    //! matches on every bound expression kind, so a wrapper would mirror the whole model.

    /// Subroutine formal extraction from a `FunctionDecl` CST node.
    pub use svlang::semantic::expr::extract_subroutine_formals;
    /// CST child (token or node), for statements whose shape depends on punctuation.
    pub use svlang::syntax::Child;
    /// IEEE 1800-2023 11.6 / 11.8 expression binder and its typed expression arena.
    pub use svlang::{
        BinaryOp, CVal, ExprArena, ExprBinder, ExprId, ExprKind, InsideItem, Loc, LocalBinding,
        LocalId, SysFunc, UnaryOp,
    };
    /// Elaborated instances, symbols and items.
    pub use svlang::{
        ElabItem, ElabItemKind, ElaboratedDesign, InstanceId, PortConn, PortDir, RtlType, SymbolId,
        SymbolKind,
    };
}

#[cfg(feature = "model")]
pub mod model {
    //! UVM/RAL design model used by `svcc-front-sv`.
    //!
    //! Re-exported rather than wrapped: the front end matches on [`UvmKind`] and [`PhaseKind`]
    //! and reads the model's class, property and RAL records field by field.

    /// Extracts `uvm_reg` / `uvm_reg_block` maps from parsed trees.
    pub use svlang_model::ral::extract_ral_blocks;
    /// Class-level UVM design model built from a [`crate::syntax::Compilation`].
    pub use svlang_model::{Design, PhaseKind, UvmKind};
}

/// Why emitted SystemVerilog failed `verify_emitted_sv`.
#[cfg(feature = "lint")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmittedSvError {
    /// A parser, preprocessor or semantic error from `svlang`.
    Diagnostic { line: u32, message: String },
    /// An error-severity finding from the `svlang-lint` `formal-ready` rule set.
    Lint {
        rule: &'static str,
        line: u32,
        message: String,
    },
}

/// Checks SystemVerilog text emitted by `svcc-emit-sv`.
///
/// Stages 1-2: `svlang` parse + semantic errors (unknown `uvm_` macros are ignored, and semantic
/// errors are ignored when `has_uvm_classes` is set). Stage 3: the in-memory `svlang-lint`
/// `formal-ready` gate (`SV-CMP-001` is skipped when `has_uvm_classes` is set). Returns the
/// first error found.
#[cfg(feature = "lint")]
pub fn verify_emitted_sv(
    filename: &str,
    sv_text: &str,
    has_uvm_classes: bool,
) -> Result<(), EmittedSvError> {
    use svlang::{Compilation, DiagnosticStage, Severity};

    let comp = Compilation::from_source(filename, sv_text);
    let errs: Vec<_> = comp
        .diagnostics()
        .into_iter()
        .filter(|d| {
            if d.severity != Severity::Error {
                return false;
            }
            match d.stage {
                DiagnosticStage::Parser => true,
                DiagnosticStage::Preprocessor => {
                    !d.message.contains("unknown macro or directive `uvm_")
                }
                DiagnosticStage::Semantic => !has_uvm_classes,
                _ => false,
            }
        })
        .collect();
    if !errs.is_empty() {
        return Err(EmittedSvError::Diagnostic {
            line: errs[0].line,
            message: errs[0].message.clone(),
        });
    }

    // Stage 3: In-memory `svlang-lint` semantic & formal-ready quality gate
    let design = svlang_model::Design::from_sv(comp.design());
    let session = svlang_lint::Session::new();
    if let Ok(keep) = svlang_lint::rule_selection(Some("formal-ready"), false) {
        let (files, findings) = session.lint_all(&design, comp.trees(), |id| {
            keep(id) && !(has_uvm_classes && id == "SV-CMP-001")
        });
        if let Some(err_f) = findings
            .iter()
            .find(|f| f.severity == svlang_lint::Severity::Error)
        {
            let (_, ln, _) = err_f.position_in(&files);
            return Err(EmittedSvError::Lint {
                rule: err_f.rule,
                line: ln,
                message: err_f.message.clone(),
            });
        }
    }
    Ok(())
}

/// Why [`lower_sv_to_btor2_design`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Btor2Error {
    /// Netlist lowering was unsupported or failed; one rendered diagnostic per entry.
    Netlist(Vec<String>),
    /// The Btor2 writer failed; the writer error rendered with `{:?}`.
    Write(String),
}

/// A design as a Btor2 transition system, with where to find its output ports in it.
#[derive(Clone, Debug)]
pub struct Btor2Design {
    pub btor2: String,
    /// `(port name, Btor2 symbol)` for every top-level output. The symbol names an `output`
    /// line, or the `state` or `input` line the port is a plain copy of.
    pub outputs: Vec<(String, String)>,
    /// Model steps per clock cycle (2 when the design uses both edges of its clock).
    pub steps_per_cycle: u32,
}

/// Parses `sv_source`, lowers module `top` to an `svlang` netlist with default settings,
/// compiles its properties, and writes it as a word-level Btor2 transition system. A design
/// on both edges of its one clock is accepted (two model steps per clock cycle).
pub fn lower_sv_to_btor2_design(
    sv_source: &str,
    filename: &str,
    top: &str,
) -> Result<Btor2Design, Btor2Error> {
    use svlang::netlist::{lower_trees, LowerConfig, LowerOutcome, TraceSignal, ValueKind};

    let tree = svlang::syntax::parse_source(sv_source, filename, Default::default());
    let mut netlist = match lower_trees(&[tree], &[top.to_string()], &[], &LowerConfig::default()) {
        LowerOutcome::Lowered(nl) => nl,
        LowerOutcome::Unsupported(ds) | LowerOutcome::Error(ds) => {
            return Err(Btor2Error::Netlist(
                ds.iter().map(ToString::to_string).collect(),
            ));
        }
    };
    let _ = netlist.compile_properties(&svlang::netlist::PropertyOptions::default());
    let mut outputs = Vec::new();
    let mut extra = Vec::new();
    for o in &netlist.outputs {
        let full = format!("{top}.{}", o.name);
        let symbol = match &netlist.value(o.value).kind {
            ValueKind::Input(i) => format!("{top}.{}", netlist.inputs[*i].name),
            ValueKind::StateOut(s) if netlist.states[s.0 as usize].latch_enable.is_none() => {
                netlist.states[s.0 as usize].name.clone()
            }
            _ => {
                if !netlist
                    .traces
                    .iter()
                    .any(|t| t.name == full && t.value == o.value)
                {
                    extra.push(TraceSignal {
                        name: full.clone(),
                        width: o.width,
                        signed: o.signed,
                        four_state: o.four_state,
                        value: o.value,
                        loc: o.loc,
                    });
                }
                full
            }
        };
        outputs.push((o.name.clone(), symbol));
    }
    netlist
        .traces
        .retain(|t| !extra.iter().any(|e| e.name == t.name));
    netlist.traces.extend(extra);
    let mut opts = svlang::btor2::Options::default();
    opts.both_edges = true;
    opts.trace_outputs = true;
    let model =
        svlang::btor2::write(&netlist, &opts).map_err(|e| Btor2Error::Write(format!("{e:?}")))?;
    Ok(Btor2Design {
        btor2: model.btor2,
        outputs,
        steps_per_cycle: model.steps_per_cycle,
    })
}
