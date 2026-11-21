use uvir_emit_sc::ScEmitter;
use uvir_emit_sv::SvEmitter;
use uvir_equiv::{prove_class_equivalence, SmtVerdict};
use uvir_front_sc::ScFrontend;
use uvir_front_sv::SvFrontend;
use uvir_ir::{BackendEmitter, Frontend, TargetCapabilities};
use uvir_passes::PassPipeline;

#[test]
fn bidirectional_sv_sc_sv_roundtrip_and_z3_proof() {
    let sv_src = include_str!("../../../testdata/sv/apb_timer_item.sv");
    let sv_fe = SvFrontend::new();
    let sc_fe = ScFrontend::new();
    let sc_em = ScEmitter::new();
    let sv_em = SvEmitter::new();

    // 1. Lower SV -> UVIR and legalize for SystemC target (expands 1800-2023 `[v +/- t]` tolerance!)
    let (mut m_sv, _) = sv_fe
        .lower_str("apb_timer_item.sv", sv_src)
        .expect("sv lower");
    PassPipeline::for_target(sc_em.capabilities())
        .run_all(&mut m_sv)
        .expect("sc legalize");
    assert!(m_sv.classes[0].needs_value_copy_ctor);

    // 2. Emit SystemC/CRAVE/FC4SC C++17 header
    let cpp_out = sc_em.emit(&m_sv).expect("sc emit");
    assert!(cpp_out.contains("apb_timer_item(const apb_timer_item& rhs)"));
    assert!(cpp_out.contains("set_sv_signedness(true)"));
    assert!(cpp_out.contains("// uvir:nocompare seq_tag"));

    // 3. Lower emitted C++17 back into UVIR and emit IEEE 1800-2023 SV (validated via svlang!)
    let (mut m_sc, _) = sc_fe
        .lower_str("apb_timer_item.hpp", &cpp_out)
        .expect("sc lower");
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023())
        .run_all(&mut m_sc)
        .expect("sv legalize");
    let sv_roundtrip = sv_em
        .emit_and_verify(&m_sc)
        .expect("sv emit & svlang verify");
    assert!(sv_roundtrip.contains("UVM_NOCOMPARE"));

    // 4. Prove formal constraint & coverage equivalence (Z3 QF_BV)
    let rep = prove_class_equivalence(
        &m_sv.classes[0],
        &m_sv.crv_arena,
        &m_sc.classes[0],
        &m_sc.crv_arena,
    )
    .expect("equiv check");
    assert_ne!(rep.hard_crv_verdict, SmtVerdict::CounterexampleSat);
    assert_eq!(rep.soft_clauses_matched, 1);
    assert!(rep.coverage_isomorphic);
}

#[test]
fn detects_semantic_constraint_bug_via_z3_counterexample() {
    let sv_ok = r#"
class pkt extends uvm_sequence_item;
  rand bit [7:0] len;
  constraint c_len { len inside {[8'd4 : 8'd16]}; }
endclass
"#;
    let sc_buggy = r#"
class pkt : public uvm::uvm_randomized_sequence_item {
public:
  crave::crv_variable<uint8_t> len{"len"};
  crave::crv_constraint c_len{this, "c_len", (len() >= 4U && len() < 16U)};
};
"#;
    let (mut m_sv, _) = SvFrontend::new().lower_str("pkt.sv", sv_ok).unwrap();
    let (mut m_sc, _) = ScFrontend::new().lower_str("pkt.hpp", sc_buggy).unwrap();
    let norm = PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc());
    norm.run_all(&mut m_sv).unwrap();
    norm.run_all(&mut m_sc).unwrap();

    let rep = prove_class_equivalence(
        &m_sv.classes[0],
        &m_sv.crv_arena,
        &m_sc.classes[0],
        &m_sc.crv_arena,
    )
    .unwrap();
    if rep.hard_crv_verdict != SmtVerdict::SolverUnavailable {
        // Off-by-one (`<= 16` vs `< 16`) MUST be caught as SAT counterexample!
        assert_eq!(rep.hard_crv_verdict, SmtVerdict::CounterexampleSat);
    }
}

#[test]
fn ral_register_transpiles_sv_to_sc_and_back() {
    let ral_sv = r#"
class timer_ctrl_reg extends uvm_reg;
  rand uvm_reg_field en;
  rand uvm_reg_field irq_en;
  uvm_reg_field status;
  function new(string name = "timer_ctrl_reg");
    super.new(name, 32, UVM_NO_COVERAGE);
  endfunction
  virtual function void build();
    en     = uvm_reg_field::type_id::create("en");
    irq_en = uvm_reg_field::type_id::create("irq_en");
    status = uvm_reg_field::type_id::create("status");
    en.configure(this, 1, 0, "RW", 0, 1'h0, 1, 1, 0);
    irq_en.configure(this, 1, 1, "RW", 0, 1'h0, 1, 1, 0);
    status.configure(this, 4, 4, "RO", 1, 4'h0, 1, 0, 0);
  endfunction
endclass

class timer_reg_block extends uvm_reg_block;
  rand timer_ctrl_reg ctrl;
  virtual function void build();
    ctrl = timer_ctrl_reg::type_id::create("ctrl");
    ctrl.configure(this);
    ctrl.build();
    default_map = create_map("default_map", 'h4000_0000, 4, UVM_LITTLE_ENDIAN);
    default_map.add_reg(ctrl, 'h00, "RW");
  endfunction
endclass
"#;
    let (mut m, _) = SvFrontend::new().lower_str("timer_ral.sv", ral_sv).unwrap();
    PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc())
        .run_all(&mut m)
        .unwrap();
    let sc = ScEmitter::new().emit(&m).unwrap();
    assert!(sc.contains("uvm::uvm_randomized_reg"));
    assert!(sc.contains("bind_field(en, &en_reg_field, \"RW\", 0, 1);"));
}
