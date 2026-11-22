module p15_seq_blocking(
  input clk,
  input [7:0] a,
  input [7:0] b,
  output reg [7:0] q
);
  reg [7:0] t;
  always @(posedge clk) begin
    t = a & b;
    q <= t + 8'd1;
  end
endmodule
