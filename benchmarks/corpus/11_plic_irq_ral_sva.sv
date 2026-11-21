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
