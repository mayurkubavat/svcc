module p05_mem(
  input clk,
  input we,
  input [3:0] waddr,
  input [3:0] raddr,
  input [7:0] wdata,
  output reg [7:0] rdata
);
  reg [7:0] mem [0:15];
  always @(posedge clk) begin
    if (we) begin
      mem[waddr] <= wdata;
    end
    rdata <= mem[raddr];
  end
endmodule
