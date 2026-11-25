//! Formal IEEE 1801 UPF Lowering, 2-Copy Non-Interference & Symbolic Retention Tests.

use svcc::prove::design_to_btor2;
use svcc::upf::{lower_with_upf_str, prove_upf_str, UpfProveOptions};
use svcc_equiv::pair::PairSide;
use svcc_equiv::upf_miter::{prove_retention_roundtrip, UpfCheckVerdict};
use svcc_passes::upf_lower::UpfLowerOptions;

const APB_SC_SUBSYS: &str = r#"
#include <cstdint>
#include <systemc>

SC_MODULE(apb_pmu_ctrl) {
    sc_core::sc_in<bool>     clk{"clk"};
    sc_core::sc_in<bool>     rst_n{"rst_n"};
    sc_core::sc_in<bool>     psel{"psel"};
    sc_core::sc_in<bool>     penable{"penable"};
    sc_core::sc_in<bool>     pwrite{"pwrite"};
    sc_core::sc_in<uint32_t> paddr{"paddr"};
    sc_core::sc_in<uint32_t> pwdata{"pwdata"};
    sc_core::sc_in<bool>     tim_pwr_ack_in{"tim_pwr_ack_in"};

    sc_core::sc_out<bool>     tim_pwr_en{"tim_pwr_en"};
    sc_core::sc_out<bool>     tim_iso_en{"tim_iso_en"};
    sc_core::sc_out<bool>     tim_ret_save{"tim_ret_save"};
    sc_core::sc_out<bool>     tim_ret_restore{"tim_ret_restore"};
    sc_core::sc_out<bool>     tim_por_n{"tim_por_n"};
    sc_core::sc_out<uint32_t> pmu_rdata{"pmu_rdata"};

    sc_core::sc_signal<bool> tim_pwr_en_q{"tim_pwr_en_q", true};
    sc_core::sc_signal<bool> tim_iso_en_q{"tim_iso_en_q", false};
    sc_core::sc_signal<bool> tim_ret_save_q{"tim_ret_save_q", false};
    sc_core::sc_signal<bool> tim_ret_restore_q{"tim_ret_restore_q", false};
    sc_core::sc_signal<bool> tim_por_n_q{"tim_por_n_q", true};

    SC_CTOR(apb_pmu_ctrl) {
        SC_METHOD(seq_proc);
        sensitive << clk.pos() << rst_n.neg();

        SC_METHOD(comb_proc);
        sensitive << paddr << tim_pwr_en_q << tim_iso_en_q
                  << tim_ret_save_q << tim_ret_restore_q
                  << tim_por_n_q << tim_pwr_ack_in;
    }

    void seq_proc() {
        if (!rst_n.read()) {
            tim_pwr_en_q.write(true);
            tim_iso_en_q.write(false);
            tim_ret_save_q.write(false);
            tim_ret_restore_q.write(false);
            tim_por_n_q.write(true);
        } else {
            bool next_save    = false;
            bool next_restore = false;
            if (psel.read() && penable.read() && pwrite.read() && paddr.read() == 0x00u) {
                const uint32_t w = pwdata.read();
                tim_pwr_en_q.write((w & 0x1u) != 0u);
                tim_iso_en_q.write((w & 0x2u) != 0u);
                next_save    = ((w & 0x4u) != 0u);
                next_restore = ((w & 0x8u) != 0u);
                tim_por_n_q.write((w & 0x10u) != 0u);
            }
            tim_ret_save_q.write(next_save);
            tim_ret_restore_q.write(next_restore);
        }
    }

    void comb_proc() {
        tim_pwr_en.write(tim_pwr_en_q.read());
        tim_iso_en.write(tim_iso_en_q.read());
        tim_ret_save.write(tim_ret_save_q.read());
        tim_ret_restore.write(tim_ret_restore_q.read());
        tim_por_n.write(tim_por_n_q.read());

        uint32_t rd = 0u;
        if (paddr.read() == 0x00u) {
            rd = (tim_pwr_en_q.read() ? 0x1u : 0u) |
                 (tim_iso_en_q.read() ? 0x2u : 0u) |
                 (tim_por_n_q.read()  ? 0x10u : 0u);
        } else if (paddr.read() == 0x04u) {
            rd = tim_pwr_ack_in.read() ? 0x1u : 0u;
        }
        pmu_rdata.write(rd);
    }
};

SC_MODULE(apb_gated_timer) {
    sc_core::sc_in<bool>     clk{"clk"};
    sc_core::sc_in<bool>     por_n{"por_n"};
    sc_core::sc_in<bool>     psel{"psel"};
    sc_core::sc_in<bool>     penable{"penable"};
    sc_core::sc_in<bool>     pwrite{"pwrite"};
    sc_core::sc_in<uint32_t> paddr{"paddr"};
    sc_core::sc_in<uint32_t> pwdata{"pwdata"};

    sc_core::sc_out<uint32_t> tim_rdata{"tim_rdata"};
    sc_core::sc_out<bool>     tim_irq{"tim_irq"};

    sc_core::sc_signal<sc_dt::sc_uint<8>>  cfg_q{"cfg_q"};
    sc_core::sc_signal<sc_dt::sc_uint<16>> load_q{"load_q"};
    sc_core::sc_signal<sc_dt::sc_uint<16>> scratch_q{"scratch_q"};
    sc_core::sc_signal<sc_dt::sc_uint<16>> count_q{"count_q"};

    SC_CTOR(apb_gated_timer) {
        SC_METHOD(seq_proc);
        sensitive << clk.pos();
        dont_initialize();

        SC_METHOD(comb_proc);
        sensitive << paddr << cfg_q << load_q << scratch_q << count_q;
    }

    void seq_proc() {
        if (!por_n.read()) {
            count_q.write(0);
        } else {
            const bool wr = psel.read() && penable.read() && pwrite.read();
            if (wr && paddr.read() == 0x10u) {
                cfg_q.write(static_cast<uint8_t>(pwdata.read() & 0x3u));
            } else if (wr && paddr.read() == 0x14u) {
                load_q.write(static_cast<uint16_t>(pwdata.read() & 0xFFFFu));
                count_q.write(static_cast<uint16_t>(pwdata.read() & 0xFFFFu));
            } else if (wr && paddr.read() == 0x18u) {
                scratch_q.write(static_cast<uint16_t>(pwdata.read() & 0xFFFFu));
            } else if ((cfg_q.read() & 0x1u) != 0u) {
                if (count_q.read() == 0u) {
                    count_q.write(load_q.read());
                } else {
                    count_q.write(count_q.read() - 1u);
                }
            }
        }
    }

    void comb_proc() {
        uint32_t rd = 0u;
        switch (paddr.read()) {
        case 0x10u: rd = cfg_q.read().to_uint(); break;
        case 0x14u: rd = load_q.read().to_uint(); break;
        case 0x18u: rd = scratch_q.read().to_uint(); break;
        case 0x1Cu: rd = count_q.read().to_uint(); break;
        default:    rd = 0u; break;
        }
        tim_rdata.write(rd);

        const bool en     = ((cfg_q.read() & 0x1u) != 0u);
        const bool irq_en = ((cfg_q.read() & 0x2u) != 0u);
        tim_irq.write(en && irq_en && (count_q.read() == 1u));
    }
};

SC_MODULE(apb_pmu_timer_subsys) {
    sc_core::sc_in<bool>      PCLK{"PCLK"};
    sc_core::sc_in<bool>      PRESETn{"PRESETn"};
    sc_core::sc_in<bool>      PSEL{"PSEL"};
    sc_core::sc_in<bool>      PENABLE{"PENABLE"};
    sc_core::sc_in<bool>      PWRITE{"PWRITE"};
    sc_core::sc_in<uint32_t>  PADDR{"PADDR"};
    sc_core::sc_in<uint32_t>  PWDATA{"PWDATA"};
    sc_core::sc_out<uint32_t> PRDATA{"PRDATA"};
    sc_core::sc_out<bool>     PREADY{"PREADY"};
    sc_core::sc_out<bool>     PSLVERR{"PSLVERR"};
    sc_core::sc_out<bool>     IRQ{"IRQ"};

    sc_core::sc_signal<bool>     tim_pwr_en{"tim_pwr_en", true};
    sc_core::sc_signal<bool>     tim_pwr_ack{"tim_pwr_ack", true};
    sc_core::sc_signal<bool>     tim_iso_en{"tim_iso_en", false};
    sc_core::sc_signal<bool>     tim_ret_save{"tim_ret_save", false};
    sc_core::sc_signal<bool>     tim_ret_restore{"tim_ret_restore", false};
    sc_core::sc_signal<bool>     tim_por_n{"tim_por_n", true};
    sc_core::sc_signal<bool>     tim_psel{"tim_psel"};
    sc_core::sc_signal<uint32_t> pmu_rdata{"pmu_rdata"};
    sc_core::sc_signal<uint32_t> tim_rdata{"tim_rdata"};
    sc_core::sc_signal<bool>     tim_irq{"tim_irq"};

    apb_pmu_ctrl    u_pmu{"u_pmu"};
    apb_gated_timer u_timer{"u_timer"};

    SC_CTOR(apb_pmu_timer_subsys) {
        tim_pwr_ack.write(true);

        u_pmu.clk(PCLK);
        u_pmu.rst_n(PRESETn);
        u_pmu.psel(PSEL);
        u_pmu.penable(PENABLE);
        u_pmu.pwrite(PWRITE);
        u_pmu.paddr(PADDR);
        u_pmu.pwdata(PWDATA);
        u_pmu.tim_pwr_ack_in(tim_pwr_ack);
        u_pmu.tim_pwr_en(tim_pwr_en);
        u_pmu.tim_iso_en(tim_iso_en);
        u_pmu.tim_ret_save(tim_ret_save);
        u_pmu.tim_ret_restore(tim_ret_restore);
        u_pmu.tim_por_n(tim_por_n);
        u_pmu.pmu_rdata(pmu_rdata);

        u_timer.clk(PCLK);
        u_timer.por_n(tim_por_n);
        u_timer.psel(tim_psel);
        u_timer.penable(PENABLE);
        u_timer.pwrite(PWRITE);
        u_timer.paddr(PADDR);
        u_timer.pwdata(PWDATA);
        u_timer.tim_rdata(tim_rdata);
        u_timer.tim_irq(tim_irq);

        SC_METHOD(fabric_mux);
        sensitive << PSEL << PENABLE << PADDR << tim_iso_en << pmu_rdata << tim_rdata << tim_irq;
    }

    void fabric_mux() {
        const uint32_t a = PADDR.read();
        const bool is_tim = (a >= 0x10u && a <= 0x1Cu);
        const bool is_pmu = (a == 0x00u || a == 0x04u);

        tim_psel.write(PSEL.read() && is_tim && !tim_iso_en.read());
        PREADY.write(true);

        if (PSEL.read() && PENABLE.read() && (!is_pmu && (!is_tim || tim_iso_en.read()))) {
            PSLVERR.write(true);
            PRDATA.write(0u);
        } else {
            PSLVERR.write(false);
            PRDATA.write(is_tim ? tim_rdata.read() : pmu_rdata.read());
        }

        IRQ.write(tim_irq.read());
    }
};
"#;

const APB_SUBSYS_UPF: &str = r#"
upf_version 3.1
set_design_top dut

create_supply_port VDD_AON -direction in
create_supply_port VSS     -direction in

add_port_state VDD_AON -state {ON_0V9 0.90} -state {LV_0V75 0.75}
add_port_state VSS     -state {GND 0.0}

create_supply_net VDD_AON
create_supply_net VDD_TIM
create_supply_net VSS

connect_supply_net VDD_AON -ports {VDD_AON}
connect_supply_net VSS     -ports {VSS}

create_supply_set SS_AON -function {power VDD_AON} -function {ground VSS}
create_supply_set SS_TIM -function {power VDD_TIM} -function {ground VSS}

create_power_domain PD_AON -include_scope -supply {primary SS_AON}
create_power_domain PD_TIM -elements {u_timer} -supply {primary SS_TIM}

create_power_switch SW_TIM \
    -domain PD_TIM \
    -input_supply_port  {vin  VDD_AON} \
    -output_supply_port {vout VDD_TIM} \
    -control_port       {en   u_pmu/tim_pwr_en_q} \
    -ack_port           {ack  tim_pwr_ack} \
    -on_state           {ON   vin {en}} \
    -off_state          {OFF  {!en}}

add_port_state SW_TIM/vout -state {ON_0V9 0.90} -state {LV_0V75 0.75} -state {OFF off}

create_pst SUBSYS_PST -supplies {VDD_AON SW_TIM/vout}
add_pst_state ALL_ON_NOM      -pst SUBSYS_PST -state {ON_0V9  ON_0V9}
add_pst_state DVFS_LOW_VOLT   -pst SUBSYS_PST -state {LV_0V75 LV_0V75}
add_pst_state TIM_POWER_GATED -pst SUBSYS_PST -state {ON_0V9  OFF}

set_isolation ISO_TIM -domain PD_TIM \
    -isolation_supply_set SS_AON \
    -clamp_value 0 \
    -elements {tim_irq tim_rdata}
set_isolation_control ISO_TIM -domain PD_TIM \
    -isolation_signal u_pmu/tim_iso_en_q \
    -isolation_sense high

set_retention RET_TIM -domain PD_TIM \
    -retention_supply_set SS_AON \
    -elements {u_timer/cfg_q u_timer/load_q}
set_retention_control RET_TIM -domain PD_TIM \
    -save_signal    {u_pmu/tim_ret_save_q posedge} \
    -restore_signal {u_pmu/tim_ret_restore_q posedge}

set_level_shifter LS_TIM -domain PD_TIM \
    -rule both -threshold 0.20
"#;

#[test]
fn strict_flow_a_systemc_apb_subsys_proves_2copy_isolation_and_retention() {
    let rep = prove_upf_str(
        "apb_pmu_timer_subsys.h",
        APB_SC_SUBSYS,
        APB_SUBSYS_UPF,
        None,
        &UpfProveOptions::default(),
    )
    .expect("prove_upf_str");

    assert!(
        rep.static_findings.is_empty(),
        "unexpected static findings: {:?}",
        rep.static_findings
    );
    assert_eq!(
        rep.cpp_ub_bmc.as_ref().expect("cpp_ub").verdict,
        UpfCheckVerdict::ProvedUnsat
    );
    assert_eq!(rep.isolation_non_interference.len(), 1);
    assert_eq!(
        rep.isolation_non_interference[0].verdict,
        UpfCheckVerdict::ProvedUnsat
    );
    assert_eq!(rep.isolation_non_interference[0].outputs_checked, 4);
    assert_eq!(rep.retention_roundtrips.len(), 2);
    for rt in &rep.retention_roundtrips {
        assert_eq!(rt.verdict, UpfCheckVerdict::ProvedUnsat, "{rt:?}");
    }
    assert!(rep.all_passed(false));

    // Unconstrained APB CSR writes to 0x00 allow rogue software to clear pwr_en without iso_en:
    // BMC finds the exact software contract violation unless PWRITE/PSEL is constrained.
    assert_eq!(rep.protocol_bmc.verdict, UpfCheckVerdict::CounterexampleSat);

    // Holding PWRITE=0 (read-only APB traffic) keeps PMU in legal state -> protocol_bmc UNSAT!
    let ro_opts = UpfProveOptions {
        hold: vec![("PRESETn".into(), 1), ("PWRITE".into(), 0)],
        require_protocol_bmc: true,
        ..Default::default()
    };
    let ro_rep = prove_upf_str(
        "apb_pmu_timer_subsys.h",
        APB_SC_SUBSYS,
        APB_SUBSYS_UPF,
        None,
        &ro_opts,
    )
    .expect("prove_upf_str ro");
    assert!(ro_rep.all_passed(true));
}

#[test]
fn missing_tim_rdata_isolation_leaks_on_apb_setup_phase_in_2copy_miter() {
    // Drop `tim_rdata` from ISO_TIM: during APB setup (`PENABLE == 0, PADDR == 0x10`),
    // `fabric_mux` forwards `tim_rdata` onto `PRDATA` even while `tim_iso_en == 1`!
    let buggy_upf = APB_SUBSYS_UPF.replace("-elements {tim_irq tim_rdata}", "-elements {tim_irq}");
    let rep = prove_upf_str(
        "apb_pmu_timer_subsys.h",
        APB_SC_SUBSYS,
        &buggy_upf,
        None,
        &UpfProveOptions::default(),
    )
    .expect("prove_upf_str");
    assert_eq!(
        rep.isolation_non_interference[0].verdict,
        UpfCheckVerdict::CounterexampleSat,
        "2-copy miter must catch APB setup-phase PRDATA leakage"
    );
    assert!(!rep.all_passed(false));
}

#[test]
fn save_after_power_collapse_lp_bug2_is_refuted_with_counterexample() {
    let (_, _, _, rep) = lower_with_upf_str(
        "apb_pmu_timer_subsys.h",
        APB_SC_SUBSYS,
        APB_SUBSYS_UPF,
        None,
        &UpfLowerOptions {
            expose_violation_port: true,
            ..Default::default()
        },
    )
    .expect("lower");
    let rr = &rep.retained_regs[0];
    let mut sub = rep.design.clone();
    sub.tops = vec![rr.specialized_module.clone()];
    let child_btor = design_to_btor2(&sub, &rr.specialized_module, "svcc_ret__").expect("btor2");
    let top_name = format!("svcc_ret__{}", rr.specialized_module);
    let side = PairSide {
        btor2: &child_btor.btor2,
        top: &top_name,
        outputs: &child_btor.outputs,
    };
    // `save_while_dom_on = false` models LP_BUG=2 (pulsing `save` after domain rail collapsed).
    let bad = prove_retention_roundtrip(
        &side,
        &rr.instance,
        &rr.reg_name,
        &rr.strategy,
        false,
        false,
        30_000,
    )
    .expect("roundtrip");
    assert_eq!(bad.verdict, UpfCheckVerdict::CounterexampleSat);
}

#[test]
fn narrow_32bit_comparator_x_leakage_caught_in_one_formal_query() {
    let sv = r#"
module worker (
    input  logic        clk,
    input  logic [31:0] din,
    output logic [31:0] tag_o
);
    logic [31:0] tag_q;
    always_ff @(posedge clk) tag_q <= din;
    assign tag_o = tag_q;
endmodule

module top_cmp (
    input  logic        clk,
    input  logic        rst_n,
    input  logic        pwr_en,
    input  logic        iso_en,
    input  logic [31:0] din,
    output logic        fatal_irq
);
    logic [31:0] tag_w;
    worker u_w (.clk(clk), .din(din), .tag_o(tag_w));
    // 1-in-2^32 comparator: 2-state random simulation misses this with probability 1 - 2^-32!
    assign fatal_irq = (tag_w == 32'hDEAD_BEEF);
endmodule
"#;
    let upf_uniso = r#"
upf_version 3.1
set_design_top top_cmp
create_supply_port VDD_AON -direction in
create_supply_port VSS -direction in
add_port_state VDD_AON -state {ON 0.90}
add_port_state VSS -state {GND 0.0}
create_supply_set SS_AON -function {power VDD_AON} -function {ground VSS}
create_supply_set SS_W   -function {power VDD_W}   -function {ground VSS}
create_power_domain PD_AON -include_scope -supply {primary SS_AON}
create_power_domain PD_W   -elements {u_w} -supply {primary SS_W}
create_power_switch SW_W -domain PD_W \
    -input_supply_port {vin VDD_AON} \
    -output_supply_port {vout VDD_W} \
    -control_port {en pwr_en} \
    -on_state {ON vin {en}}
"#;
    let leak = prove_upf_str(
        "top_cmp.sv",
        sv,
        upf_uniso,
        None,
        &UpfProveOptions::default(),
    )
    .expect("uniso");
    assert_eq!(
        leak.isolation_non_interference[0].verdict,
        UpfCheckVerdict::CounterexampleSat
    );

    let upf_iso = format!(
        "{upf_uniso}\nset_isolation ISO_W -domain PD_W -isolation_supply_set SS_AON -clamp_value 0 -elements {{tag_w}} -isolation_signal iso_en -isolation_sense high\n"
    );
    let safe = prove_upf_str(
        "top_cmp.sv",
        sv,
        &upf_iso,
        None,
        &UpfProveOptions::default(),
    )
    .expect("iso");
    assert_eq!(
        safe.isolation_non_interference[0].verdict,
        UpfCheckVerdict::ProvedUnsat
    );
}

#[test]
fn missing_level_shifter_and_self_located_isolation_collapse_are_detected() {
    // 1. Cross-voltage PST state `{ON_0V9 LV_0V75}` (150 mV delta) without `set_level_shifter`:
    let cross_volt_upf = APB_SUBSYS_UPF
        .replace(
            "add_pst_state DVFS_LOW_VOLT   -pst SUBSYS_PST -state {LV_0V75 LV_0V75}",
            "add_pst_state ASYM_VOLT       -pst SUBSYS_PST -state {ON_0V9  LV_0V75}",
        )
        .replace(
            "set_level_shifter LS_TIM -domain PD_TIM \\\n    -rule both -threshold 0.20",
            "",
        );
    let rep_ls = prove_upf_str(
        "apb_pmu_timer_subsys.h",
        APB_SC_SUBSYS,
        &cross_volt_upf,
        None,
        &UpfProveOptions::default(),
    )
    .expect("ls check");
    assert!(
        rep_ls
            .static_findings
            .iter()
            .any(|f| f.code == "LS_MISSING"),
        "expected LS_MISSING finding, got {:?}",
        rep_ls.static_findings
    );

    // 2. `-location self` without `-isolation_supply_set SS_AON` powers the isolation cell
    // from collapsed `VDD_TIM`, so `upf__off_isolated_PD_TIM` can never hold (`iso_active == 0` when `!dom_on`)!
    let self_iso_upf = APB_SUBSYS_UPF.replace("-isolation_supply_set SS_AON", "-location self");
    let rep_self = prove_upf_str(
        "apb_pmu_timer_subsys.h",
        APB_SC_SUBSYS,
        &self_iso_upf,
        None,
        &UpfProveOptions {
            hold: vec![
                ("PRESETn".into(), 1),
                ("PSEL".into(), 1),
                ("PENABLE".into(), 1),
                ("PWRITE".into(), 1),
                ("PADDR".into(), 0),
                ("PWDATA".into(), 0x12),
            ],
            require_protocol_bmc: true,
            ..Default::default()
        },
    )
    .expect("self iso");
    // Even when firmware writes `PWDATA = 0x12` (`iso_en = 1, pwr_en = 0`), self-located isolation
    // loses power when `VDD_TIM` drops, firing `upf__violation` (`ISO_WHILE_OFF`)!
    assert_eq!(
        rep_self.protocol_bmc.verdict,
        UpfCheckVerdict::CounterexampleSat
    );
}
