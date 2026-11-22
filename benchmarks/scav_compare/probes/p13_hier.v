module p13_add #(parameter W = 4) (
  input [W-1:0] a,
  input [W-1:0] b,
  output [W:0] s
);
  assign s = a + b;
endmodule
module p13_hier(
  input [7:0] a,
  input [7:0] b,
  input [7:0] c,
  output [9:0] s
);
  wire [8:0] t;
  p13_add #(.W(8)) u0 (.a(a), .b(b), .s(t));
  p13_add #(.W(9)) u1 (.a(t), .b({1'b0, c}), .s(s));
endmodule
