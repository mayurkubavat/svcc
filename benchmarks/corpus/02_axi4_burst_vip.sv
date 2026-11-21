// IEEE 1800-2023 + IEEE 1800.2-2020 AXI4 Burst Sequence Item, Clocking #1step, Driver fork/join_any, & Env
typedef enum bit [1:0] {
  AXI_FIXED = 2'd0,
  AXI_INCR  = 2'd1,
  AXI_WRAP  = 2'd2
} axi_burst_e;

interface axi4_if(input logic aclk);
  logic awready;
  logic awvalid;
  clocking cb @(posedge aclk);
    default input #1step output #0;
    input awready;
    output awvalid;
  endclocking
endinterface

class axi4_item extends uvm_sequence_item;
  rand axi_burst_e burst;
  randc bit [2:0]  awid;
  rand bit [7:0]   awlen;
  rand bit [31:0]  awaddr;

  `uvm_object_utils(axi4_item)

  constraint c_axi4_burst {
    (awaddr & 32'hF) == 32'h0;
    burst dist { AXI_FIXED := 10, [AXI_INCR : AXI_WRAP] :/ 90 };
    (burst == AXI_WRAP) -> (awlen inside {8'd1, 8'd3, 8'd7, 8'd15});
    soft awlen == 8'd3;
  }

  function new(string name = "axi4_item");
    super.new(name);
  endfunction
endclass

class axi4_driver extends uvm_driver #(axi4_item);
  uvm_analysis_port #(axi4_item) item_ap;

  `uvm_component_utils(axi4_driver)

  function new(string name = "axi4_driver", uvm_component parent = null);
    super.new(name, parent);
  endfunction

  virtual task run_phase(uvm_phase phase);
    fork
      begin
        @(cb);
      end
      begin
        #250ns;
      end
    join_any
    disable fork;
  endtask
endclass

class axi4_scoreboard extends uvm_subscriber #(axi4_item);
  `uvm_component_utils(axi4_scoreboard)

  function new(string name = "axi4_scoreboard", uvm_component parent = null);
    super.new(name, parent);
  endfunction
endclass

class axi4_env extends uvm_env;
  axi4_driver drv;
  axi4_scoreboard scb;

  `uvm_component_utils(axi4_env)

  function new(string name = "axi4_env", uvm_component parent = null);
    super.new(name, parent);
  endfunction

  virtual function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    drv = axi4_driver::type_id::create("drv", this);
    scb = axi4_scoreboard::type_id::create("scb", this);
  endfunction

  virtual function void connect_phase(uvm_phase phase);
    super.connect_phase(phase);
    drv.item_ap.connect(scb.analysis_export);
  endfunction
endclass
