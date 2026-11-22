// Memory as an unpacked array: synchronous write, asynchronous read, reset loop.
module regfile #(parameter int AW = 3, parameter int DW = 8) (
  input  logic          clk,
  input  logic          rst_n,
  input  logic          we,
  input  logic [AW-1:0] waddr,
  input  logic [DW-1:0] wdata,
  input  logic [AW-1:0] raddr0,
  input  logic [AW-1:0] raddr1,
  output logic [DW-1:0] rdata0,
  output logic [DW-1:0] rdata1,
  output logic [DW-1:0] rdata1_q
);
  logic [DW-1:0] mem [2**AW];
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      for (int i = 0; i < 2**AW; i++) mem[i] <= '0;
    end else if (we) begin
      mem[waddr] <= wdata;
    end
  end
  assign rdata0 = mem[raddr0];
  assign rdata1 = (we && waddr == raddr1) ? wdata : mem[raddr1];
  always_ff @(posedge clk) rdata1_q <= mem[raddr1];
endmodule
