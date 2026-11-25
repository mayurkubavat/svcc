//! `svcc` — Bidirectional IEEE 1800-2023 SystemVerilog <-> C++17 SystemC/UVM & RTL Compiler SDK.

pub use svcc_emit_sc as emit_sc;
pub use svcc_emit_sv as emit_sv;
pub use svcc_equiv as equiv;
pub use svcc_front_sc as front_sc;
pub use svcc_front_sv as front_sv;
pub use svcc_ir as ir;
pub use svcc_passes as passes;

pub mod cosim;
pub mod prove;
pub mod upf;

use svcc_emit_sc::ScEmitter;
use svcc_emit_sv::SvEmitter;
use svcc_equiv::{prove_class_equivalence_with_enums, EquivalenceReport};
use svcc_front_sc::ScFrontend;
use svcc_front_sv::SvFrontend;
use svcc_ir::{BackendEmitter, Frontend, LanguageRegistry, SvccError, TargetCapabilities};
use svcc_passes::PassPipeline;

pub fn default_registry() -> LanguageRegistry {
    let mut reg = LanguageRegistry::new();
    reg.register_frontend("sv-uvm", Box::new(SvFrontend::new()));
    reg.register_frontend("sc-uvm", Box::new(ScFrontend::new()));
    reg.register_backend("sv-uvm", Box::new(SvEmitter::new()));
    reg.register_backend("sc-uvm", Box::new(ScEmitter::new()));
    reg
}

/// Compiles IEEE 1800-2023 SystemVerilog (RTL modules + UVM classes/SVA) into C++17 SystemC.
pub fn sv_to_sc(filename: &str, sv_source: &str) -> Result<String, SvccError> {
    let (mut module, _) = SvFrontend::new().lower_str(filename, sv_source)?;
    let emitter = ScEmitter::new();
    PassPipeline::for_target(emitter.capabilities()).run_all(&mut module)?;
    emit_sc_with_typed_rtl(filename, sv_source, module, &emitter)
}

/// Emits C++ for a lowered SystemVerilog file, converting its design modules through the
/// typed, elaborated `hdl` path ([`sv_rtl_to_sc`]).
///
/// A file that holds only modules becomes a header that needs nothing but `<systemc>`. A file
/// that mixes modules with classes, covergroups or assertions keeps the UVM prelude and gets
/// the typed modules appended. A module the typed path cannot convert is an error: there is
/// no other RTL path.
pub fn emit_sc_with_typed_rtl(
    filename: &str,
    sv_source: &str,
    module: svcc_ir::IrModule,
    emitter: &ScEmitter,
) -> Result<String, SvccError> {
    if module.module_names.is_empty() {
        return emitter.emit(&module);
    }
    let pure_rtl = module.classes.is_empty()
        && module.standalone_covergroups.is_empty()
        && module.properties.is_empty()
        && module.clockings.is_empty();
    if pure_rtl {
        return sv_rtl_to_sc(filename, sv_source, &[]).map(|(code, _)| code);
    }
    let design = svcc_front_sv::hdl_lower::lower_hdl_design(filename, sv_source, &[])?;
    let mut out = emitter.emit(&module)?;
    svcc_emit_sc::hdl_emit::emit_hdl_modules(&mut out, &design)?;
    Ok(out)
}

/// Converts synthesizable SystemVerilog RTL into SystemC through the typed, elaborated
/// `hdl` design IR. Returns the generated header and the design it was emitted from.
///
/// `tops` names the top modules; when empty every uninstantiated module is a top.
pub fn sv_rtl_to_sc(
    filename: &str,
    sv_source: &str,
    tops: &[String],
) -> Result<(String, svcc_ir::hdl::Design), SvccError> {
    let design = svcc_front_sv::hdl_lower::lower_hdl_design(filename, sv_source, tops)?;
    let code = svcc_emit_sc::hdl_emit::emit_hdl_design(&design)?;
    Ok((code, design))
}

/// As [`sv_rtl_to_sc`], for a multi-file design given as command-line style arguments (source
/// files, `-f`, `+incdir+`, `+define+`, `-G name=value`, `-top module`).
pub fn sv_rtl_args_to_sc(args: &[String]) -> Result<(String, svcc_ir::hdl::Design), SvccError> {
    let design = svcc_front_sv::hdl_lower::lower_hdl_from_args(args)?;
    let code = svcc_emit_sc::hdl_emit::emit_hdl_design(&design)?;
    Ok((code, design))
}

/// Compiles C++17 SystemC UVM/CRAVE/FC4SC classes into `svlang`-verified IEEE 1800-2023
/// SystemVerilog.
///
/// `SC_MODULE`s and a `// svcc:c2rtl` function in the source are appended as explicit-width
/// SystemVerilog read back with C++ evaluation rules ([`svcc_front_sc::hdl_lift`]). That text
/// is the form the formal gates compare; each module carries the `svcc__ub` output that is 1
/// when the C++ executes undefined behaviour.
pub fn sc_to_sv(filename: &str, cpp_source: &str) -> Result<String, SvccError> {
    let (mut module, _) = ScFrontend::new().lower_str(filename, cpp_source)?;
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023()).run_all(&mut module)?;
    let mut out = SvEmitter::new().emit_and_verify(&module)?;
    if cpp_source.contains("SC_MODULE(") {
        let design = svcc_front_sc::hdl_lift::lift_hdl_modules(cpp_source)?;
        out.push_str(&svcc_emit_sv::hdl_emit::emit_hdl_design_sv(&design, "")?);
    }
    if cpp_source.contains("// svcc:c2rtl") {
        let design = svcc_front_sc::hdl_lift::lift_c2rtl_function(cpp_source, "result")?;
        out.push_str(&svcc_emit_sv::hdl_emit::emit_hdl_design_sv(&design, "")?);
    }
    Ok(out)
}

/// Proves Z3 `QF_BV` CRV/Enum/`dist` and UCIS 1.0 coverage equivalence between two files.
pub fn prove_sources_equivalent(
    lhs_filename: &str,
    lhs_source: &str,
    rhs_filename: &str,
    rhs_source: &str,
) -> Result<Vec<EquivalenceReport>, SvccError> {
    let reg = default_registry();
    let detect = |p: &str| {
        if p.ends_with(".sv") || p.ends_with(".svh") {
            "sv-uvm"
        } else {
            "sc-uvm"
        }
    };
    let (mut m_l, _) = reg
        .frontend(detect(lhs_filename))
        .unwrap()
        .lower_str(lhs_filename, lhs_source)?;
    let (mut m_r, _) = reg
        .frontend(detect(rhs_filename))
        .unwrap()
        .lower_str(rhs_filename, rhs_source)?;

    let norm = PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc());
    norm.run_all(&mut m_l)?;
    norm.run_all(&mut m_r)?;

    let mut reports = Vec::with_capacity(m_l.classes.len());
    for cls_l in &m_l.classes {
        let cls_r = m_r.find_class(&cls_l.name).ok_or_else(|| {
            SvccError::new(
                "svcc/equiv",
                format!("class '{}' missing in {rhs_filename}", cls_l.name),
            )
        })?;
        reports.push(prove_class_equivalence_with_enums(
            cls_l,
            &m_l.crv_arena,
            &m_l.enums,
            cls_r,
            &m_r.crv_arena,
            &m_r.enums,
        )?);
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_sv_to_sc_and_back_works() {
        let sv = r#"
class mini_pkt extends uvm_sequence_item;
  rand bit [15:0] payload;
  constraint c_nz { payload != 16'h0; }
endclass
"#;
        let cpp = sv_to_sc("mini.sv", sv).expect("sv_to_sc");
        let sv_rt = sc_to_sv("mini.hpp", &cpp).expect("sc_to_sv");
        let reps = prove_sources_equivalent("mini.sv", sv, "mini_rt.sv", &sv_rt).expect("prove");
        assert_eq!(reps.len(), 1);
        assert!(cpp.contains("crave::crv_variable<uint16_t> payload"));
    }
}
