module p17_wrap(
  input [7:0] a,
  input [7:0] b,
  input [7:0] c,
  output lt
);
  assign lt = (a - b) < c;
endmodule
