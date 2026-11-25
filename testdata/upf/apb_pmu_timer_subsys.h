#ifndef APB_PMU_TIMER_SUBSYS_H
#define APB_PMU_TIMER_SUBSYS_H

// Pure synthesizable IEEE 1666.1 SystemC RTL (Strict Flow A: ZERO LP macros in RTL).
// Power domains, switches, isolation, retention, and PST live strictly in apb_subsys.upf.

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
            // Auto-clear one-shot save/restore pulses unless written high
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

    // Retained CSRs (configured via apb_subsys.upf RET_TIM)
    sc_core::sc_signal<sc_dt::sc_uint<8>>  cfg_q{"cfg_q"};
    sc_core::sc_signal<sc_dt::sc_uint<16>> load_q{"load_q"};

    // Non-retained volatile registers (scramble on PD_TIM collapse!)
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
        tim_pwr_ack.write(true); // default when UPF switch is swapped out / disabled

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

        // APB firewall blocks downstream PSEL when isolation is active
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

#endif // APB_PMU_TIMER_SUBSYS_H
