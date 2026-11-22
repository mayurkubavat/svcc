module p01_comb_blocking(
  input [7:0] a,
  input [7:0] b,
  input [7:0] c,
  output reg [7:0] y
);
  reg [7:0] tmp;
  always @(*) begin
    tmp = a + b;
    y = tmp ^ c;
  end
endmodule
