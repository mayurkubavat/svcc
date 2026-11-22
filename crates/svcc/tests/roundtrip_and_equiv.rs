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
    assert!(cpp_out.contains("// svcc:nocompare seq_tag"));

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

    let cpp_out = sc_em
        .emit_and_verify_clang(&m_sv, &shim_dir())
        .expect("axi4 clang++ -Werror");
    assert!(cpp_out.contains("enum class axi_burst_e : uint8_t"));
    assert!(cpp_out.contains("struct svcc_clocking_cb : public sc_core::sc_module"));
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
    assert!(cpp_out.contains("class svcc_sva_ap_tx_start_busy : public sc_core::sc_module"));

    for prop in &m_sv.properties {
        let bmc = prove_sva_bmc_equivalence(prop, 6).expect("sva bmc");
        assert_eq!(bmc.verdict, SmtVerdict::EquivalentUnsat);
    }

    let (mut m_sc, _) = sc_fe
        .lower_str("uart_ral_sva.hpp", &cpp_out)
        .expect("uart sc lower");
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023())
        .run_all(&mut m_sc)
        .expect("uart sv legalize");
    let sv_rt = sv_em.emit_and_verify(&m_sc).expect("uart svlang verify");
    assert!(sv_rt.contains("default_map.add_reg(ctrl, 'h0, \"RW\");"));
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

#[test]
fn p2_dist_lcm_p3_cover_multiplicity_p5_feistel_and_oracle_params() {
    let sv_src = r#"
class param_dist_item extends uvm_sequence_item;
  localparam int MAX_BUCKET = 3;
  randc bit [3:0] tag;
  rand  bit [3:0] bucket;

  `uvm_object_utils(param_dist_item)

  constraint c_dist_param {
    bucket <= MAX_BUCKET;
    bucket dist { [4'd0 : 4'd2] :/ 10, 4'd3 := 2 };
  }
endclass

module burst_sva_mon(input logic clk, input logic rst_n, input logic req, input logic ack);
  cp_burst: cover property (@(posedge clk) disable iff (!rst_n) req |-> ##[1:3] ack);
endmodule
"#;
    let (mut m_sv, _) = SvFrontend::new().lower_str("p235.sv", sv_src).unwrap();
    PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc())
        .run_all(&mut m_sv)
        .unwrap();

    let shim_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cpp_shims");
    let sc_code = ScEmitter::new()
        .emit_and_verify_clang(&m_sv, &shim_dir)
        .expect("clang++ clean on P2/P3/P5");

    // P2: [0:2] :/ 10, 3 := 2 -> L=3 -> (30, 6) -> admissible GCD(10,6)=2 -> (15, 3), 15 % 3 == 0
    assert!(sc_code.contains(".add_weighted_range(0U, 2U, 15U)"));
    assert!(sc_code.contains(".add(3U, 3U)"));
    // P3: cover property uses per-age multiplicity array
    assert!(sc_code.contains("std::array<uint32_t, 4U> pending_age_{};"));
    // P5: randc uses O(1) cycle-walking Feistel helper
    assert!(sc_code.contains("struct svcc_randc_feistel"));
    assert!(sc_code.contains("svcc_randc_feistel<4U> tag_randc_gen_{};"));

    // P3 Z3 BMC: per-age matches IEEE 1800 multiplicity (Unsat), bitmask under-counts (Sat)!
    let prop = &m_sv.properties[0];
    let bmc_age = svcc_equiv::prove_sva_cover_multiplicity_bmc(prop, 4, true).unwrap();
    assert_eq!(bmc_age.verdict, SmtVerdict::EquivalentUnsat);
    let bmc_mask = svcc_equiv::prove_sva_cover_multiplicity_bmc(prop, 4, false).unwrap();
    assert_eq!(bmc_mask.verdict, SmtVerdict::CounterexampleSat);

    // Round-trip back through svcc-front-sc + svcc-emit-sv (with svlang-lint gate!) + Z3 CRV miter
    let (mut m_sc, _) = ScFrontend::new().lower_str("p235.hpp", &sc_code).unwrap();
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023())
        .run_all(&mut m_sc)
        .unwrap();
    let sv_rt = SvEmitter::new()
        .emit_and_verify(&m_sc)
        .expect("svlang + svlang-lint clean");
    let (mut m_sv2, _) = SvFrontend::new().lower_str("p235_rt.sv", &sv_rt).unwrap();
    PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc())
        .run_all(&mut m_sv2)
        .unwrap();

    let eq = prove_class_equivalence_with_enums(
        &m_sv.classes[0],
        &m_sv.crv_arena,
        &m_sv.enums,
        &m_sv2.classes[0],
        &m_sv2.crv_arena,
        &m_sv2.enums,
    )
    .unwrap();
    assert_eq!(eq.hard_crv_verdict, SmtVerdict::EquivalentUnsat);
}

#[test]
fn p2_sliced_dist_clips_weighted_range_and_proves_z3_roundtrip_equiv() {
    let sv_src = r#"
class sliced_dist_item extends uvm_sequence_item;
  rand bit [3:0] x;
  rand bit [4:0] y;

  `uvm_object_utils(sliced_dist_item)

  constraint c_sliced {
    x >= 4'd1;
    x dist { [4'd0 : 4'd2] :/ 10, 4'd3 := 2 };
    y >= 5'd8;
    y dist { [5'd0 : 5'd15] :/ 5, 5'd16 := 5 };
  }
endclass
"#;
    let (m_orig_raw, _) = SvFrontend::new().lower_str("sliced.sv", sv_src).unwrap();
    let mut m_sc_target = m_orig_raw.clone();
    PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc())
        .run_all(&mut m_sc_target)
        .unwrap();

    let shim_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cpp_shims");
    let sc_code = ScEmitter::new()
        .emit_and_verify_clang(&m_sc_target, &shim_dir)
        .expect("clang++ clean on sliced dist");

    // Case 1: [0:2]:/10, 3:=2 under x >= 1 -> [1:2] :/ 10, 3 := 3 (exact conditional probs 5/13, 5/13, 3/13)
    assert!(sc_code.contains(".add_weighted_range(1U, 2U, 10U).add(3U, 3U)"));
    // Case 2: [0:15]:/5, 16:=5 under y >= 8 -> [8:15] :/ 8, 16 := 16 (exact conditional probs 1/24 .. 16/24)
    assert!(sc_code.contains(".add_weighted_range(8U, 15U, 8U).add(0x10U, 16U)"));

    // Verify Z3 QF_BV proves EquivalentUnsat against the unclipped raw SV spec!
    let mut m_sv_spec = m_orig_raw;
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023())
        .run_all(&mut m_sv_spec)
        .unwrap();
    let eq = prove_class_equivalence_with_enums(
        &m_sv_spec.classes[0],
        &m_sv_spec.crv_arena,
        &m_sv_spec.enums,
        &m_sc_target.classes[0],
        &m_sc_target.crv_arena,
        &m_sc_target.enums,
    )
    .unwrap();
    assert_eq!(eq.hard_crv_verdict, SmtVerdict::EquivalentUnsat);
}
