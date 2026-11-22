module p02_signed(
  input signed [7:0] a,
  input signed [7:0] b,
  output lt,
  output signed [7:0] sra,
  output signed [15:0] prod
);
  assign lt = a < b;
  assign sra = a >>> 2;
  assign prod = a * b;
endmodule
