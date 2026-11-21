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
