typedef enum bit [2:0] {
  CHI_READ_SHARED = 3'd0,
  CHI_READ_UNIQUE = 3'd1,
  CHI_CLEAN_UNIQUE = 3'd2,
  CHI_WRITE_BACK = 3'd3
} chi_opcode_e;

class chi_req_item extends uvm_sequence_item;
  rand chi_opcode_e opcode;
  rand bit [6:0]    src_nid;
  rand bit [6:0]    tgt_nid;
  rand bit [3:0]    qos;
  rand bit [31:0]   line_addr;

  `uvm_object_utils(chi_req_item)

  constraint c_chi_coherency {
    unique { src_nid, tgt_nid };
    (line_addr & 32'h3F) == 32'h0;
    line_addr inside {[32'h8000_0000 : 32'h800F_FFFF], [32'h9000_1000 +/- 32'h40]};
    qos dist { 4'd0 := 20, [4'd1 : 4'd7] := 10, [4'd8 : 4'd15] :/ 40 };
    (opcode == CHI_WRITE_BACK) -> (qos >= 4'd4);
    soft qos == 4'd8;
  }

  covergroup cg_chi;
    cp_qos: coverpoint qos {
      bins low_pri  = {[0 : 3]};
      bins med_pri  = {[4 : 11]};
      bins high_pri = {[12 : 15]};
    }
  endgroup

  function new(string name = "chi_req_item");
    super.new(name);
    cg_chi = new();
  endfunction
endclass
