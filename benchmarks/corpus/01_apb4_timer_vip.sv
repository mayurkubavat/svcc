// Canonical IEEE 1800-2023 + IEEE 1800.2-2020 APB4 Timer Sequence Item & Covergroup fixture
class apb_timer_item extends uvm_sequence_item;
  rand bit [31:0] paddr;
  rand bit [31:0] pwdata;
  rand bit [3:0]  pstrb;
  rand bit [7:0]  burst_len;
  rand bit        pwrite;
  bit [31:0]      seq_tag;

  `uvm_object_utils_begin(apb_timer_item)
    `uvm_field_int(paddr, UVM_ALL_ON)
    `uvm_field_int(pwdata, UVM_ALL_ON)
    `uvm_field_int(pstrb, UVM_ALL_ON)
    `uvm_field_int(burst_len, UVM_ALL_ON)
    `uvm_field_int(pwrite, UVM_ALL_ON)
    `uvm_field_int(seq_tag, UVM_NOCOMPARE)
  `uvm_object_utils_end

  constraint c_apb_protocol {
    (paddr & 32'h3) == 32'h0;
    paddr inside {[32'h4000_0000 : 32'h4000_0FFF], [32'h4001_0100 +/- 32'h10]};
    pwrite -> (pstrb != 4'h0);
    (!pwrite) -> (pstrb == 4'h0);
    burst_len inside {8'd1, 8'd4, 8'd8, 8'd16};
    soft burst_len == 8'd4;
  }

  covergroup cg_apb_timer;
    cp_addr: coverpoint paddr {
      bins timer0_regs = {[32'h4000_0000 : 32'h4000_07FF]};
      bins timer1_regs = {[32'h4000_0800 : 32'h4000_0FFF]};
    }
    cp_dir: coverpoint pwrite {
      bins rd = {0};
      bins wr = {1};
    }
    cp_burst: coverpoint burst_len {
      bins single = {1};
      bins incr4  = {4};
      bins incr8  = {8};
      bins incr16 = {16};
    }
    cx_dir_addr: cross cp_addr, cp_dir;
  endgroup

  function new(string name = "apb_timer_item");
    super.new(name);
    cg_apb_timer = new();
  endfunction
endclass
