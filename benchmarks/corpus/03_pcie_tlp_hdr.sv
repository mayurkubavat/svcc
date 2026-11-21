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
