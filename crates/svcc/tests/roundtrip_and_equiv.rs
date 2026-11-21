use std::path::PathBuf;
use svcc_emit_sc::ScEmitter;
use svcc_emit_sv::SvEmitter;
use svcc_equiv::{prove_class_equivalence_with_enums, prove_sva_bmc_equivalence, SmtVerdict};
use svcc_front_sc::ScFrontend;
use svcc_front_sv::SvFrontend;
use svcc_ir::{BackendEmitter, Frontend, TargetCapabilities};
use svcc_passes::PassPipeline;

fn shim_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cpp_shims")
}

#[test]
fn apb4_roundtrip_clang_svlang_and_z3_proof() {
    let sv_src = include_str!("../../../testdata/sv/apb_timer_item.sv");
    let sv_fe = SvFrontend::new();
    let sc_fe = ScFrontend::new();
    let sc_em = ScEmitter::new();
    let sv_em = SvEmitter::new();

    let (mut m_sv, _) = sv_fe
        .lower_str("apb_timer_item.sv", sv_src)
        .expect("sv lower");
    PassPipeline::for_target(sc_em.capabilities())
        .run_all(&mut m_sv)
        .expect("sc legalize");

    let cpp_out = sc_em
        .emit_and_verify_clang(&m_sv, &shim_dir())
        .expect("sc emit & clang++ -Werror");
    assert!(cpp_out.contains("apb_timer_item(const apb_timer_item& rhs)"));
    assert!(cpp_out.contains("set_sv_signedness(true)"));
    assert!(cpp_out.contains("// uvir:nocompare seq_tag"));

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

    let rep = prove_class_equivalence_with_enums(
        &m_sv.classes[0],
        &m_sv.crv_arena,
        &m_sv.enums,
        &m_sc.classes[0],
        &m_sc.crv_arena,
        &m_sc.enums,
    )
    .expect("equiv check");
    assert_eq!(rep.hard_crv_verdict, SmtVerdict::EquivalentUnsat);
    assert_eq!(rep.soft_clauses_matched, 1);
    assert!(rep.coverage_isomorphic);
}

#[test]
fn axi4_enum_dist_randc_clocking_fork_env_full_roundtrip() {
    let sv_src = include_str!("../../../testdata/sv/axi4_burst_env.sv");
    let sv_fe = SvFrontend::new();
    let sc_fe = ScFrontend::new();
    let sc_em = ScEmitter::new();
    let sv_em = SvEmitter::new();

    let (mut m_sv, _) = sv_fe
        .lower_str("axi4_burst_env.sv", sv_src)
        .expect("axi4 sv lower");
    PassPipeline::for_target(sc_em.capabilities())
        .run_all(&mut m_sv)
        .expect("axi4 sc legalize");

    // Compile generated C++17 (enum class, clocking #1step, CRAVE dist, sc_spawn cancel_tok, env TLM) via clang++ -Werror!
    let cpp_out = sc_em
        .emit_and_verify_clang(&m_sv, &shim_dir())
        .expect("axi4 clang++ -Werror");
    assert!(cpp_out.contains("enum class axi_burst_e : uint8_t"));
    assert!(cpp_out.contains("struct uvir_clocking_cb : public sc_core::sc_module"));
    assert!(cpp_out.contains(".add_weighted_range("));
    assert!(cpp_out.contains("sc_core::sc_spawn"));
    assert!(cpp_out.contains("this->drv->item_ap.connect(scb->analysis_export)"));

    let (mut m_sc, _) = sc_fe
        .lower_str("axi4_burst_env.hpp", &cpp_out)
        .expect("axi4 sc lower");
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023())
        .run_all(&mut m_sc)
        .expect("axi4 sv legalize");
    let sv_rt = sv_em.emit_and_verify(&m_sc).expect("axi4 svlang verify");
    assert!(sv_rt.contains("typedef enum bit [1:0]"));
    assert!(sv_rt.contains("randc bit [2:0] awid;"));
    assert!(sv_rt.contains("join_any"));
    assert!(sv_rt.contains("disable fork;"));
    assert!(sv_rt.contains("drv.item_ap.connect(scb.analysis_export);"));

    let item_sv = m_sv.find_class("axi4_item").unwrap();
    let item_sc = m_sc.find_class("axi4_item").unwrap();
    let rep = prove_class_equivalence_with_enums(
        item_sv,
        &m_sv.crv_arena,
        &m_sv.enums,
        item_sc,
        &m_sc.crv_arena,
        &m_sc.enums,
    )
    .unwrap();
    assert_eq!(rep.hard_crv_verdict, SmtVerdict::EquivalentUnsat);
    assert_eq!(rep.dist_clauses_matched, 1);
}

#[test]
fn uart_ral_block_and_pipelined_sva_bmc_roundtrip() {
    let sv_src = include_str!("../../../testdata/sv/uart_ral_sva.sv");
    let sv_fe = SvFrontend::new();
    let sc_fe = ScFrontend::new();
    let sc_em = ScEmitter::new();
    let sv_em = SvEmitter::new();

    let (mut m_sv, _) = sv_fe
        .lower_str("uart_ral_sva.sv", sv_src)
        .expect("uart sv lower");
    assert_eq!(m_sv.properties.len(), 2);
    PassPipeline::for_target(sc_em.capabilities())
        .run_all(&mut m_sv)
        .expect("uart sc legalize");

    let cpp_out = sc_em
        .emit_and_verify_clang(&m_sv, &shim_dir())
        .expect("uart clang++ -Werror");
    assert!(cpp_out.contains("uvm::uvm_randomized_reg"));
    assert!(cpp_out.contains("this->default_map->add_reg(ctrl, 0x0ULL, \"RW\");"));
    assert!(cpp_out.contains("class uvir_sva_ap_tx_start_busy : public sc_core::sc_module"));

    // Prove Z3 BMC trace equivalence for both pipelined ##[1:4] and ##[2:8] SVA properties!
    for prop in &m_sv.properties {
        let bmc = prove_sva_bmc_equivalence(prop, 6).expect("sva bmc");
        assert_eq!(bmc.verdict, SmtVerdict::EquivalentUnsat);
    }

    let (mut m_sc, _) = sc_fe
        .lower_str("uart_ral_sva.hpp", &cpp_out)
        .expect("uart sc lower");
    assert_eq!(m_sc.properties.len(), 2);
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023())
        .run_all(&mut m_sc)
        .expect("uart sv legalize");
    let sv_rt = sv_em.emit_and_verify(&m_sc).expect("uart svlang verify");
    assert!(sv_rt.contains("default_map.add_reg(ctrl, 'h0, \"RW\");"));
    assert!(sv_rt.contains("ap_tx_start_busy: assert property (@(posedge clk) disable iff (!rst_n) tx_start |-> ##[1:4] tx_busy);"));
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

    let rep = prove_class_equivalence_with_enums(
        &m_sv.classes[0],
        &m_sv.crv_arena,
        &m_sv.enums,
        &m_sc.classes[0],
        &m_sc.crv_arena,
        &m_sc.enums,
    )
    .unwrap();
    assert_eq!(rep.hard_crv_verdict, SmtVerdict::CounterexampleSat);
}
