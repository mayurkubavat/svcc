#!/usr/bin/env python3
"""Materializes the 20-design svcc benchmark corpus under benchmarks/corpus/:
  - Category A (01..06): UVM Sequence Items, Enums, Dist, Tolerance Inside, Driver Concurrency, Env TLM, FC4SC
  - Category B (07..12): UVM RAL Register Blocks + Clocked Pipelined SVA Assertions & Covers
  - Category C (13..20): Synthesizable IEEE 1800-2023 RTL Modules + Untimed C++17 C2RTL Golden Models (latencies 0..3)
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parent / "corpus"

FILES: dict[str, str] = {
    # ---------------- Category A: UVM Protocol VIPs (01..06) ----------------
    "01_apb4_timer_vip.sv": Path(__file__).resolve().parent.parent.joinpath("testdata/sv/apb_timer_item.sv").read_text(),
    "02_axi4_burst_vip.sv": Path(__file__).resolve().parent.parent.joinpath("testdata/sv/axi4_burst_env.sv").read_text(),
    "03_pcie_tlp_hdr.sv": """\
typedef enum bit [1:0] {
  TLP_MEM_RD = 2'd0,
  TLP_MEM_WR = 2'd1,
  TLP_CFG_RD = 2'd2,
  TLP_CPLD   = 2'd3
} pcie_tlp_fmt_e;

class pcie_tlp_item extends uvm_sequence_item;
  rand pcie_tlp_fmt_e fmt;
  randc bit [4:0]     tag;
  rand bit [9:0]      length_dw;
  rand bit [3:0]      first_be;
  rand bit [31:0]     addr;
  bit [15:0]          requester_id;

  `uvm_object_utils_begin(pcie_tlp_item)
    `uvm_field_int(length_dw, UVM_ALL_ON)
    `uvm_field_int(first_be, UVM_ALL_ON)
    `uvm_field_int(addr, UVM_ALL_ON)
    `uvm_field_int(requester_id, UVM_NOCOMPARE)
  `uvm_object_utils_end

  constraint c_pcie_spec {
    (addr & 32'h3) == 32'h0;
    length_dw inside {[10'd1 : 10'd128]};
    fmt dist { TLP_MEM_RD := 40, TLP_MEM_WR := 40, [TLP_CFG_RD : TLP_CPLD] :/ 20 };
    (length_dw == 10'd1) -> (first_be != 4'h0);
    soft length_dw == 10'd4;
  }

  covergroup cg_pcie_tlp;
    cp_len: coverpoint length_dw {
      bins single_dw = {1};
      bins burst_sm  = {[2 : 16]};
      bins burst_lg  = {[17 : 128]};
    }
    cp_be: coverpoint first_be {
      bins full_dw = {15};
      bins partial = {[1 : 14]};
    }
    cx_len_be: cross cp_len, cp_be;
  endgroup

  function new(string name = "pcie_tlp_item");
    super.new(name);
    cg_pcie_tlp = new();
  endfunction
endclass
""",
    "04_chi_coh_req.sv": """\
typedef enum bit [2:0] {
  CHI_READ_SHARED = 3'd0,
  CHI_READ_UNIQUE = 3'd1,
  CHI_CLEAN_UNIQUE = 3'd2,
  CHI_WRITE_BACK = 3'd3
} chi_opcode_e;

class chi_req_item extends uvm_sequence_item;
  rand chi_opcode_e opcode;
  rand bit [6:0]    src_nid;
  rand bit [6:0]    tgt_nid;
  rand bit [3:0]    qos;
  rand bit [31:0]   line_addr;

  `uvm_object_utils(chi_req_item)

  constraint c_chi_coherency {
    unique { src_nid, tgt_nid };
    (line_addr & 32'h3F) == 32'h0;
    line_addr inside {[32'h8000_0000 : 32'h800F_FFFF], [32'h9000_1000 +/- 32'h40]};
    qos dist { 4'd0 := 20, [4'd1 : 4'd7] := 10, [4'd8 : 4'd15] :/ 40 };
    (opcode == CHI_WRITE_BACK) -> (qos >= 4'd4);
    soft qos == 4'd8;
  }

  covergroup cg_chi;
    cp_qos: coverpoint qos {
      bins low_pri  = {[0 : 3]};
      bins med_pri  = {[4 : 11]};
      bins high_pri = {[12 : 15]};
    }
  endgroup

  function new(string name = "chi_req_item");
    super.new(name);
    cg_chi = new();
  endfunction
endclass
""",
    "05_eth_mac_frame.sv": """\
class eth_mac_item extends uvm_sequence_item;
  rand bit [15:0] ethertype;
  rand bit [13:0] payload_bytes;
  rand bit        vlan_en;
  rand bit [11:0] vlan_vid;
  bit [31:0]      fcs_crc;

  `uvm_object_utils_begin(eth_mac_item)
    `uvm_field_int(ethertype, UVM_ALL_ON)
    `uvm_field_int(payload_bytes, UVM_ALL_ON)
    `uvm_field_int(vlan_en, UVM_ALL_ON)
    `uvm_field_int(vlan_vid, UVM_ALL_ON)
    `uvm_field_int(fcs_crc, UVM_NOCOMPARE)
  `uvm_object_utils_end

  constraint c_ieee_802_3 {
    payload_bytes inside {[14'd46 : 14'd1500], [14'd9000 +/- 14'd16]};
    vlan_en -> (vlan_vid inside {[12'd1 : 12'd4094]});
    (!vlan_en) -> (vlan_vid == 12'd0);
    ethertype inside {16'h0800, 16'h0806, 16'h86DD};
    soft payload_bytes == 14'd64;
  }

  covergroup cg_eth_frame;
    cp_size: coverpoint payload_bytes {
      bins min_pkt   = {46};
      bins std_frame = {[47 : 1500]};
      bins jumbo     = {[8984 : 9016]};
    }
    cp_vlan: coverpoint vlan_en {
      bins untagged = {0};
      bins tagged_v = {1};
    }
    cx_vlan_sz: cross cp_size, cp_vlan;
  endgroup

  function new(string name = "eth_mac_item");
    super.new(name);
    cg_eth_frame = new();
  endfunction
endclass
""",
    "06_ddr5_cmd_sched.sv": """\
typedef enum bit [1:0] {
  DDR_ACT = 2'd0,
  DDR_RD  = 2'd1,
  DDR_WR  = 2'd2,
  DDR_PRE = 2'd3
} ddr5_cmd_e;

interface ddr5_dfi_if(input logic dfi_clk);
  logic dfi_cs_n;
  logic dfi_rddata_valid;
  clocking cb_dfi @(posedge dfi_clk);
    default input #1step output #0;
    input dfi_rddata_valid;
    output dfi_cs_n;
  endclocking
endinterface

class ddr5_cmd_item extends uvm_sequence_item;
  rand ddr5_cmd_e cmd;
  randc bit [2:0] bank_group;
  rand bit [1:0]  bank_addr;
  rand bit [15:0] row_addr;
  rand bit [9:0]  col_addr;

  `uvm_object_utils(ddr5_cmd_item)

  constraint c_ddr5_jedec {
    (col_addr & 10'hF) == 10'h0;
    cmd dist { DDR_RD := 45, DDR_WR := 35, [DDR_ACT : DDR_PRE] :/ 20 };
    (cmd == DDR_PRE) -> (col_addr == 10'd0);
    soft bank_addr == 2'd0;
  }

  function new(string name = "ddr5_cmd_item");
    super.new(name);
  endfunction
endclass

class ddr5_driver extends uvm_driver #(ddr5_cmd_item);
  uvm_analysis_port #(ddr5_cmd_item) cmd_ap;

  `uvm_component_utils(ddr5_driver)

  function new(string name = "ddr5_driver", uvm_component parent = null);
    super.new(name, parent);
  endfunction

  virtual task run_phase(uvm_phase phase);
    fork
      begin
        @(cb_dfi);
      end
      begin
        #500ns;
      end
    join_any
    disable fork;
  endtask
endclass
""",
    # ---------------- Category B: UVM RAL + SVA Suites (07..12) ----------------
    "07_uart_16550_ral_sva.sv": Path(__file__).resolve().parent.parent.joinpath("testdata/sv/uart_ral_sva.sv").read_text(),
    "08_dma_ch_ral_sva.sv": """\
class dma_ch_cfg_reg extends uvm_reg;
  rand uvm_reg_field ch_en;
  rand uvm_reg_field burst_max;
  rand uvm_reg_field irq_mask;
  uvm_reg_field      ch_active_ro;

  function new(string name = "dma_ch_cfg_reg");
    super.new(name, 32, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    ch_en        = uvm_reg_field::type_id::create("ch_en");
    burst_max    = uvm_reg_field::type_id::create("burst_max");
    irq_mask     = uvm_reg_field::type_id::create("irq_mask");
    ch_active_ro = uvm_reg_field::type_id::create("ch_active_ro");
    ch_en.configure(this, 1, 0, "RW", 0, 1'h0, 1, 1, 0);
    burst_max.configure(this, 4, 4, "RW", 0, 4'h4, 1, 1, 0);
    irq_mask.configure(this, 2, 8, "RW", 0, 2'h0, 1, 1, 0);
    ch_active_ro.configure(this, 1, 16, "RO", 1, 1'h0, 1, 0, 0);
  endfunction
endclass

class dma_reg_block extends uvm_reg_block;
  rand dma_ch_cfg_reg ch0_cfg;

  function new(string name = "dma_reg_block");
    super.new(name, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    ch0_cfg = dma_ch_cfg_reg::type_id::create("ch0_cfg");
    ch0_cfg.configure(this);
    ch0_cfg.build();
    default_map = create_map("default_map", 'h5000_0000, 4, UVM_LITTLE_ENDIAN);
    default_map.add_reg(ch0_cfg, 'h00, "RW");
  endfunction
endclass

module dma_sva_mon(input logic clk, input logic rst_n, input logic dma_req, input logic dma_ack, input logic dma_done);
  ap_dma_req_ack:  assert property (@(posedge clk) disable iff (!rst_n) dma_req |-> ##[1:3] dma_ack);
  ap_dma_ack_done: assert property (@(posedge clk) disable iff (!rst_n) dma_ack |-> ##[1:6] dma_done);
endmodule
""",
    "09_spi_master_ral_sva.sv": """\
class spi_ctrl_reg extends uvm_reg;
  rand uvm_reg_field spi_en;
  rand uvm_reg_field cpol;
  rand uvm_reg_field cpha;
  rand uvm_reg_field clk_div;

  function new(string name = "spi_ctrl_reg");
    super.new(name, 32, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    spi_en  = uvm_reg_field::type_id::create("spi_en");
    cpol    = uvm_reg_field::type_id::create("cpol");
    cpha    = uvm_reg_field::type_id::create("cpha");
    clk_div = uvm_reg_field::type_id::create("clk_div");
    spi_en.configure(this, 1, 0, "RW", 0, 1'h0, 1, 1, 0);
    cpol.configure(this, 1, 1, "RW", 0, 1'h0, 1, 1, 0);
    cpha.configure(this, 1, 2, "RW", 0, 1'h0, 1, 1, 0);
    clk_div.configure(this, 8, 8, "RW", 0, 8'h02, 1, 1, 0);
  endfunction
endclass

class spi_reg_block extends uvm_reg_block;
  rand spi_ctrl_reg ctrl;

  function new(string name = "spi_reg_block");
    super.new(name, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    ctrl = spi_ctrl_reg::type_id::create("ctrl");
    ctrl.configure(this);
    ctrl.build();
    default_map = create_map("default_map", 'h4003_0000, 4, UVM_LITTLE_ENDIAN);
    default_map.add_reg(ctrl, 'h00, "RW");
  endfunction
endclass

module spi_sva_mon(input logic clk, input logic rst_n, input logic cs_active, input logic sclk_en);
  ap_cs_to_sclk: assert property (@(posedge clk) disable iff (!rst_n) cs_active |-> ##[1:3] sclk_en);
endmodule
""",
    "10_i2c_ctrl_ral_sva.sv": """\
class i2c_prescale_reg extends uvm_reg;
  rand uvm_reg_field prescale_lo;
  rand uvm_reg_field prescale_hi;
  rand uvm_reg_field core_en;

  function new(string name = "i2c_prescale_reg");
    super.new(name, 32, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    prescale_lo = uvm_reg_field::type_id::create("prescale_lo");
    prescale_hi = uvm_reg_field::type_id::create("prescale_hi");
    core_en     = uvm_reg_field::type_id::create("core_en");
    prescale_lo.configure(this, 8, 0, "RW", 0, 8'hFF, 1, 1, 0);
    prescale_hi.configure(this, 8, 8, "RW", 0, 8'h00, 1, 1, 0);
    core_en.configure(this, 1, 16, "RW", 0, 1'h0, 1, 1, 0);
  endfunction
endclass

class i2c_reg_block extends uvm_reg_block;
  rand i2c_prescale_reg prer;

  function new(string name = "i2c_reg_block");
    super.new(name, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    prer = i2c_prescale_reg::type_id::create("prer");
    prer.configure(this);
    prer.build();
    default_map = create_map("default_map", 'h4004_0000, 4, UVM_LITTLE_ENDIAN);
    default_map.add_reg(prer, 'h00, "RW");
  endfunction
endclass

module i2c_sva_mon(input logic clk, input logic rst_n, input logic start_cmd, input logic bus_busy, input logic stop_cmd, input logic bus_idle);
  ap_i2c_start: assert property (@(posedge clk) disable iff (!rst_n) start_cmd |-> ##[1:4] bus_busy);
  ap_i2c_stop:  assert property (@(posedge clk) disable iff (!rst_n) stop_cmd  |-> ##[1:2] bus_idle);
endmodule
""",
    "11_plic_irq_ral_sva.sv": """\
class plic_prio_reg extends uvm_reg;
  rand uvm_reg_field prio_lvl;
  rand uvm_reg_field irq_en;
  uvm_reg_field      claim_id;

  function new(string name = "plic_prio_reg");
    super.new(name, 32, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    prio_lvl = uvm_reg_field::type_id::create("prio_lvl");
    irq_en   = uvm_reg_field::type_id::create("irq_en");
    claim_id = uvm_reg_field::type_id::create("claim_id");
    prio_lvl.configure(this, 3, 0, "RW", 0, 3'h1, 1, 1, 0);
    irq_en.configure(this, 1, 4, "RW", 0, 1'h0, 1, 1, 0);
    claim_id.configure(this, 6, 8, "RO", 1, 6'h0, 1, 0, 0);
  endfunction
endclass

class plic_reg_block extends uvm_reg_block;
  rand plic_prio_reg src0;

  function new(string name = "plic_reg_block");
    super.new(name, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    src0 = plic_prio_reg::type_id::create("src0");
    src0.configure(this);
    src0.build();
    default_map = create_map("default_map", 'h0C00_0000, 4, UVM_LITTLE_ENDIAN);
    default_map.add_reg(src0, 'h04, "RW");
  endfunction
endclass

module plic_sva_mon(input logic clk, input logic rst_n, input logic irq_pend, input logic eip_out);
  ap_plic_eip: assert property (@(posedge clk) disable iff (!rst_n) irq_pend |-> ##[1:3] eip_out);
endmodule
""",
    "12_wdog_tmr_ral_sva.sv": """\
class wdog_ctrl_reg extends uvm_reg;
  rand uvm_reg_field wdog_en;
  rand uvm_reg_field rst_en;
  rand uvm_reg_field reload_val;

  function new(string name = "wdog_ctrl_reg");
    super.new(name, 32, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    wdog_en    = uvm_reg_field::type_id::create("wdog_en");
    rst_en     = uvm_reg_field::type_id::create("rst_en");
    reload_val = uvm_reg_field::type_id::create("reload_val");
    wdog_en.configure(this, 1, 0, "RW", 0, 1'h0, 1, 1, 0);
    rst_en.configure(this, 1, 1, "RW", 0, 1'h0, 1, 1, 0);
    reload_val.configure(this, 16, 16, "RW", 0, 16'hFFFF, 1, 1, 0);
  endfunction
endclass

class wdog_reg_block extends uvm_reg_block;
  rand wdog_ctrl_reg ctrl;

  function new(string name = "wdog_reg_block");
    super.new(name, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    ctrl = wdog_ctrl_reg::type_id::create("ctrl");
    ctrl.configure(this);
    ctrl.build();
    default_map = create_map("default_map", 'h4005_0000, 4, UVM_LITTLE_ENDIAN);
    default_map.add_reg(ctrl, 'h00, "RW");
  endfunction
endclass

module wdog_sva_mon(input logic clk, input logic rst_n, input logic expire_pulse, input logic sys_rst_req);
  ap_wdog_rst: assert property (@(posedge clk) disable iff (!rst_n) expire_pulse |-> ##[1:2] sys_rst_req);
endmodule
""",
    # ---------------- Category C: Synthesizable RTL + C2RTL Pairs (13..20) ----------------
    "13_alu32_pipe2.sv": Path(__file__).resolve().parent.parent.joinpath("testdata/sv/alu_pipe_rtl.sv").read_text(),
    "13_alu32_pipe2_golden.hpp": Path(__file__).resolve().parent.parent.joinpath("testdata/sc/alu_golden_c2rtl.hpp").read_text(),
    "14_mac16_pipe2.sv": """\
module mac16_pipe2 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [15:0] a,
  input  logic [15:0] b,
  input  logic [31:0] c,
  output logic [31:0] result
);
  logic [31:0] prod_s1;
  logic [31:0] c_s1;
  logic [31:0] prod_comb;

  always_comb begin
    prod_comb = a * b;
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      prod_s1 <= 32'd0;
      c_s1    <= 32'd0;
    end else begin
      prod_s1 <= prod_comb;
      c_s1    <= c;
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 32'd0;
    end else begin
      result <= prod_s1 + c_s1;
    end
  end
endmodule
""",
    "14_mac16_pipe2_golden.hpp": """\
#pragma once
#include <cstdint>

// svcc:c2rtl
uint32_t mac16_golden(uint16_t a, uint16_t b, uint32_t c) {
  uint32_t prod = uint32_t(a) * b;
  uint32_t sum = prod + c;
  return sum;
}
""",
    "15_crc8_step_comb.sv": """\
module crc8_step_comb (
  input  logic       clk,
  input  logic       rst_n,
  input  logic [7:0] crc_in,
  input  logic [7:0] data_in,
  output logic [7:0] result
);
  logic [7:0] mixed;

  always_comb begin
    mixed = crc_in ^ data_in;
    if ((mixed & 8'd1) == 8'd1) begin
      result = (mixed >> 1) ^ 8'h8C;
    end else begin
      result = (mixed >> 1);
    end
  end
endmodule
""",
    "15_crc8_step_comb_golden.hpp": """\
#pragma once
#include <cstdint>

// svcc:c2rtl
uint8_t crc8_step_golden(uint8_t crc_in, uint8_t data_in) {
  uint8_t mixed = crc_in ^ data_in;
  uint8_t out = mixed >> 1U;
  if ((mixed & 1U) == 1U) {
    out = (mixed >> 1U) ^ 0x8CU;
  }
  return out;
}
""",
    "16_gray_enc16_pipe1.sv": """\
module gray_enc16_pipe1 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [15:0] bin_in,
  output logic [15:0] result
);
  logic [15:0] gray_comb;

  always_comb begin
    gray_comb = bin_in ^ (bin_in >> 1);
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 16'd0;
    end else begin
      result <= gray_comb;
    end
  end
endmodule
""",
    "16_gray_enc16_pipe1_golden.hpp": """\
#pragma once
#include <cstdint>

// svcc:c2rtl
uint16_t gray_enc16_golden(uint16_t bin_in) {
  uint16_t g = bin_in ^ (bin_in >> 1U);
  return g;
}
""",
    "17_sat_sub16_pipe1.sv": """\
module sat_sub16_pipe1 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [15:0] x,
  input  logic [15:0] y,
  output logic [15:0] result
);
  logic [15:0] diff_comb;

  always_comb begin
    if (x >= y) begin
      diff_comb = x - y;
    end else begin
      diff_comb = 16'd0;
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 16'd0;
    end else begin
      result <= diff_comb;
    end
  end
endmodule
""",
    "17_sat_sub16_pipe1_golden.hpp": """\
#pragma once
#include <cstdint>

// svcc:c2rtl
uint16_t sat_sub16_golden(uint16_t x, uint16_t y) {
  uint16_t d = 0U;
  if (x >= y) {
    d = x - y;
  }
  return d;
}
""",
    "18_shifter32_pipe2.sv": """\
module shifter32_pipe2 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [31:0] din,
  input  logic [4:0]  shamt,
  input  logic        dir_right,
  output logic [31:0] result
);
  logic [31:0] din_s1;
  logic [4:0]  shamt_s1;
  logic        dir_s1;
  logic [31:0] shifted_comb;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      din_s1   <= 32'd0;
      shamt_s1 <= 5'd0;
      dir_s1   <= 1'd0;
    end else begin
      din_s1   <= din;
      shamt_s1 <= shamt;
      dir_s1   <= dir_right;
    end
  end

  always_comb begin
    if (dir_s1) begin
      shifted_comb = din_s1 >> shamt_s1;
    end else begin
      shifted_comb = din_s1 << shamt_s1;
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 32'd0;
    end else begin
      result <= shifted_comb;
    end
  end
endmodule
""",
    "18_shifter32_pipe2_golden.hpp": """\
#pragma once
#include <cstdint>
#include <systemc>

// svcc:c2rtl
uint32_t shifter32_golden(uint32_t din, sc_dt::sc_uint<5> shamt, bool dir_right) {
  uint32_t out = din << shamt;
  if (dir_right) {
    out = din >> shamt;
  }
  return out;
}
""",
    "19_minmax32_pipe1.sv": """\
module minmax32_pipe1 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic        pick_max,
  input  logic [31:0] a,
  input  logic [31:0] b,
  output logic [31:0] result
);
  logic [31:0] sel_comb;

  always_comb begin
    sel_comb = a;
    if (pick_max) begin
      if (b > a) begin
        sel_comb = b;
      end
    end else begin
      if (b < a) begin
        sel_comb = b;
      end
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 32'd0;
    end else begin
      result <= sel_comb;
    end
  end
endmodule
""",
    "19_minmax32_pipe1_golden.hpp": """\
#pragma once
#include <cstdint>

// svcc:c2rtl
uint32_t minmax32_golden(bool pick_max, uint32_t a, uint32_t b) {
  uint32_t out = a;
  if (pick_max) {
    if (b > a) {
      out = b;
    }
  } else {
    if (b < a) {
      out = b;
    }
  }
  return out;
}
""",
    "20_arb_prio4_comb.sv": """\
module arb_prio4_comb (
  input  logic       clk,
  input  logic       rst_n,
  input  logic [3:0] req,
  output logic [3:0] result
);
  always_comb begin
    result = 4'd0;
    if ((req & 4'd8) != 4'd0) begin
      result = 4'd8;
    end
    if ((req & 4'd4) != 4'd0) begin
      result = 4'd4;
    end
    if ((req & 4'd2) != 4'd0) begin
      result = 4'd2;
    end
    if ((req & 4'd1) != 4'd0) begin
      result = 4'd1;
    end
  end
endmodule
""",
    "20_arb_prio4_comb_golden.hpp": """\
#pragma once
#include <systemc>

// svcc:c2rtl
sc_dt::sc_uint<4> arb_prio4_golden(sc_dt::sc_uint<4> req) {
  sc_dt::sc_uint<4> gnt = 0U;
  if ((req & 8U) != 0U) {
    gnt = 8U;
  }
  if ((req & 4U) != 0U) {
    gnt = 4U;
  }
  if ((req & 2U) != 0U) {
    gnt = 2U;
  }
  if ((req & 1U) != 0U) {
    gnt = 1U;
  }
  return gnt;
}
""",
}


def main() -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    for fname, content in FILES.items():
        (ROOT / fname).write_text(content, encoding="utf-8")
    print(f"Materialized {len(FILES)} corpus files in {ROOT}")


if __name__ == "__main__":
    main()
