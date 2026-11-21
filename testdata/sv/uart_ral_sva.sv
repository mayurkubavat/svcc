// IEEE 1800-2023 + IEEE 1800.2-2020 UART RAL Register Block + Pipelined SVA Handshake Assertions
class uart_ctrl_reg extends uvm_reg;
  rand uvm_reg_field tx_en;
  rand uvm_reg_field rx_en;
  rand uvm_reg_field baud_div;
  uvm_reg_field      tx_busy_ro;

  function new(string name = "uart_ctrl_reg");
    super.new(name, 32, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    tx_en      = uvm_reg_field::type_id::create("tx_en");
    rx_en      = uvm_reg_field::type_id::create("rx_en");
    baud_div   = uvm_reg_field::type_id::create("baud_div");
    tx_busy_ro = uvm_reg_field::type_id::create("tx_busy_ro");
    tx_en.configure(this, 1, 0, "RW", 0, 1'h0, 1, 1, 0);
    rx_en.configure(this, 1, 1, "RW", 0, 1'h0, 1, 1, 0);
    baud_div.configure(this, 12, 4, "RW", 0, 12'h01A, 1, 1, 0);
    tx_busy_ro.configure(this, 1, 16, "RO", 1, 1'h0, 1, 0, 0);
  endfunction
endclass

class uart_reg_block extends uvm_reg_block;
  rand uart_ctrl_reg ctrl;

  function new(string name = "uart_reg_block");
    super.new(name, UVM_NO_COVERAGE);
  endfunction

  virtual function void build();
    ctrl = uart_ctrl_reg::type_id::create("ctrl");
    ctrl.configure(this);
    ctrl.build();
    default_map = create_map("default_map", 'h4002_0000, 4, UVM_LITTLE_ENDIAN);
    default_map.add_reg(ctrl, 'h00, "RW");
  endfunction
endclass

module uart_sva_checker(input logic clk, input logic rst_n, input logic tx_start, input logic tx_busy, input logic tx_done);
  ap_tx_start_busy: assert property (@(posedge clk) disable iff (!rst_n) tx_start |-> ##[1:4] tx_busy);
  cp_tx_complete:   cover property  (@(posedge clk) disable iff (!rst_n) tx_busy  |-> ##[2:8] tx_done);
endmodule
