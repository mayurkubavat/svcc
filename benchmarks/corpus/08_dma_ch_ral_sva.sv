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
