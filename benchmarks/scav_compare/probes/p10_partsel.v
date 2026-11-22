module p10_partsel(
  input [31:0] d,
  input [1:0] sel,
  output [7:0] byte_o,
  output [15:0] rep,
  output [31:0] swapped
);
  assign byte_o = d[sel*8 +: 8];
  assign rep = {4{d[3:0]}};
  assign swapped = {d[7:0], d[15:8], d[23:16], d[31:24]};
endmodule
