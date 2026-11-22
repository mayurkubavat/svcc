module p04_wide(
  input [127:0] a,
  input [127:0] b,
  output [127:0] s,
  output [127:0] x
);
  assign s = a + b;
  assign x = a ^ {b[63:0], b[127:64]};
endmodule
