//! `svcc` — Bidirectional IEEE 1800-2023 SystemVerilog <-> C++17 SystemC/UVM Compiler SDK.
//!
//! Exposes high-level single-call APIs for Rust callers, `PyO3` Python bindings (`pip install svcc`),
//! `InFormal` C2RTL/SEC integration, and `wasm32` browser artifacts, while re-exporting all
//! underlying compiler stages.

pub use svcc_emit_sc as emit_sc;
pub use svcc_emit_sv as emit_sv;
pub use svcc_equiv as equiv;
pub use svcc_front_sc as front_sc;
pub use svcc_front_sv as front_sv;
pub use svcc_ir as ir;
pub use svcc_passes as passes;

use svcc_emit_sc::ScEmitter;
use svcc_emit_sv::SvEmitter;
use svcc_equiv::{prove_class_equivalence_with_enums, EquivalenceReport};
use svcc_front_sc::ScFrontend;
use svcc_front_sv::SvFrontend;
use svcc_ir::{BackendEmitter, Frontend, LanguageRegistry, TargetCapabilities, UvirError};
use svcc_passes::PassPipeline;

/// Constructs a default [`LanguageRegistry`] populated with `sv-uvm` and `sc-uvm`.
pub fn default_registry() -> LanguageRegistry {
    let mut reg = LanguageRegistry::new();
    reg.register_frontend("sv-uvm", Box::new(SvFrontend::new()));
    reg.register_frontend("sc-uvm", Box::new(ScFrontend::new()));
    reg.register_backend("sv-uvm", Box::new(SvEmitter::new()));
    reg.register_backend("sc-uvm", Box::new(ScEmitter::new()));
    reg
}

/// Compiles IEEE 1800-2023 SystemVerilog/UVM source text into C++17 SystemC/UVM + CRAVE + FC4SC.
pub fn sv_to_sc(filename: &str, sv_source: &str) -> Result<String, UvirError> {
    let (mut module, _) = SvFrontend::new().lower_str(filename, sv_source)?;
    let emitter = ScEmitter::new();
    PassPipeline::for_target(emitter.capabilities()).run_all(&mut module)?;
    emitter.emit(&module)
}

/// Compiles C++17 SystemC/UVM + CRAVE + FC4SC source text into `svlang`-verified IEEE 1800-2023 SV/UVM.
pub fn sc_to_sv(filename: &str, cpp_source: &str) -> Result<String, UvirError> {
    let (mut module, _) = ScFrontend::new().lower_str(filename, cpp_source)?;
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023()).run_all(&mut module)?;
    SvEmitter::new().emit_and_verify(&module)
}

/// Proves Z3 `QF_BV` constraint/enum/`dist` and UCIS 1.0 coverage equivalence between two files.
pub fn prove_sources_equivalent(
    lhs_filename: &str,
    lhs_source: &str,
    rhs_filename: &str,
    rhs_source: &str,
) -> Result<Vec<EquivalenceReport>, UvirError> {
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
            UvirError::new(
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
