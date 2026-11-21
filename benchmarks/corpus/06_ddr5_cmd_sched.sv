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
