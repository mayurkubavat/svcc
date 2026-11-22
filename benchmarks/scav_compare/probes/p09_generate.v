module p09_generate #(parameter N = 8) (
  input [N-1:0] a,
  input [N-1:0] b,
  output [N-1:0] y
);
  genvar g;
  generate
    for (g = 0; g < N; g = g + 1) begin : bitg
      assign y[g] = a[g] ^ b[N-1-g];
    end
  endgenerate
endmodule
