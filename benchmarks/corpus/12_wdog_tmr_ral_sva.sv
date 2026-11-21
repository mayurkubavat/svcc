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
