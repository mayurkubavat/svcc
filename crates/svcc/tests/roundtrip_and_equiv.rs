use std::path::PathBuf;
use svcc::prove_rtl_or_c2rtl_sec;
use svcc_emit_sc::ScEmitter;
use svcc_emit_sv::SvEmitter;
use svcc_equiv::{
    prove_class_equivalence_with_enums, prove_rtl_sec_equivalence, prove_sva_bmc_equivalence,
    SmtVerdict,
};
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
fn rtl_sv_to_sc_module_clang_svlang_and_btor2_sec_roundtrip() {
    let sv_src = include_str!("../../../testdata/sv/alu_pipe_rtl.sv");
    let sv_fe = SvFrontend::new();
    let sc_fe = ScFrontend::new();
    let sc_em = ScEmitter::new();
    let sv_em = SvEmitter::new();

    // 1. Lower synthesizable 2-stage pipelined SV RTL -> UVIR `rtl` dialect
    let (mut m_sv, metrics) = sv_fe.lower_str("alu_pipe_rtl.sv", sv_src).expect("rtl sv");
    assert_eq!(metrics.rtl_modules_lowered, 1);
    PassPipeline::for_target(sc_em.capabilities())
        .run_all(&mut m_sv)
        .unwrap();

    // 2. Emit SystemC `SC_MODULE` (`SC_METHOD` + `SC_CTHREAD` + auto-inferred `sensitive <<`) & check via clang++ -Werror!
    let sc_code = sc_em
        .emit_and_verify_clang(&m_sv, &shim_dir())
        .expect("rtl SC_MODULE clang++ -Werror");
    assert!(sc_code.contains("SC_MODULE(alu_pipe)"));
    assert!(sc_code.contains("SC_METHOD(comb_proc_0);"));
    assert!(sc_code.contains("sensitive << a_s1 << b_s1 << op_s1;"));
    assert!(sc_code.contains("SC_CTHREAD(sync_ff_0, clk.pos());"));
    assert!(sc_code.contains("async_reset_signal_is(rst_n, false);"));

    // 3. Lift `SC_MODULE` back to SV RTL and verify via svlang + Btor2/Z3 Sequential Equivalence!
    let (mut m_sc, _) = sc_fe.lower_str("alu_pipe.hpp", &sc_code).expect("sc lift");
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023())
        .run_all(&mut m_sc)
        .unwrap();
    let sv_rt = sv_em
        .emit_and_verify(&m_sc)
        .expect("rtl sv roundtrip check");

    let sec = prove_rtl_sec_equivalence(
        &m_sv.rtl_modules[0],
        sv_src,
        &m_sc.rtl_modules[0],
        &sv_rt,
        0,
        4,
    )
    .expect("btor2 sec");
    assert_eq!(sec.verdict, SmtVerdict::EquivalentUnsat);
    assert!(sec.btor2_lines > 20);
}

#[test]
fn c2rtl_untimed_cpp_vs_pipelined_sv_rtl_sec_and_bug_detection() {
    let cpp_golden = include_str!("../../../testdata/sc/alu_golden_c2rtl.hpp");
    let sv_pipelined = include_str!("../../../testdata/sv/alu_pipe_rtl.sv");

    // 1. Prove untimed C++17 golden ALU == 2-stage pipelined SV RTL (`latency = 2`) via Btor2 + Z3!
    let sec_ok = prove_rtl_or_c2rtl_sec(
        "alu_golden_c2rtl.hpp",
        cpp_golden,
        "alu_pipe_rtl.sv",
        sv_pipelined,
        2,
    )
    .expect("c2rtl sec");
    assert_eq!(sec_ok.verdict, SmtVerdict::EquivalentUnsat);

    // 2. Inject a subtle RTL datapath bug (`a_s1 | b_s1` instead of `a_s1 ^ b_s1` on op == 3)
    //    and verify Btor2 + Z3 catches it with `CounterexampleSat`!
    let sv_buggy = sv_pipelined.replace("a_s1 ^ b_s1", "a_s1 | b_s1");
    let sec_bug = prove_rtl_or_c2rtl_sec(
        "alu_golden_c2rtl.hpp",
        cpp_golden,
        "alu_pipe_buggy.sv",
        &sv_buggy,
        2,
    )
    .expect("c2rtl bug sec");
    assert_eq!(sec_bug.verdict, SmtVerdict::CounterexampleSat);

    // 3. Also test bounded C++17 `for` loop unrolling in C2RTL (4-bit XOR parity reducer)
    let cpp_loop = r#"
// svcc:c2rtl
bool xor_parity4(sc_dt::sc_uint<4> x) {
  bool p = 0;
  for (int i = 0; i < 4; ++i) {
    p ^= ((x >> i) & 1U);
  }
  return p;
}
"#;
    let sv_parity = r#"
module xor_parity4_rtl (
  input  logic       clk,
  input  logic       rst_n,
  input  logic [3:0] x,
  output logic       result
);
  assign result = x[0] ^ x[1] ^ x[2] ^ x[3];
endmodule
"#;
    let sec_loop = prove_rtl_or_c2rtl_sec("parity.hpp", cpp_loop, "parity.sv", sv_parity, 0)
        .expect("loop sec");
    assert_eq!(sec_loop.verdict, SmtVerdict::EquivalentUnsat);
}

#[test]
fn ieee_1666_1_rejects_non_synthesizable_cpp_fail_closed() {
    let bad_sc = r#"
SC_MODULE(bad_heap_mod) {
  sc_core::sc_in<bool> clk{"clk"};
  void proc() {
    int* p = new int(42);
    delete p;
  }
  SC_CTOR(bad_heap_mod) { SC_METHOD(proc); }
};
"#;
    let err = ScFrontend::new()
        .lower_str("bad_heap.hpp", bad_sc)
        .expect_err("must reject heap new in SC_MODULE");
    assert_eq!(err.stage, "svcc-front-sc/ieee1666.1");
    assert!(err.message.contains("bad_heap.hpp:5:"));
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
