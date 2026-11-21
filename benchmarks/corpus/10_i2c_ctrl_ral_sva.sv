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
