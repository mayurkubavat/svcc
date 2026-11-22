module p12_ops(
  input [15:0] a,
  input [3:0] sh,
  input [1:0] m,
  output [15:0] y,
  output par,
  output allz
);
  assign y = (m == 2'd0) ? (a << sh) : (m == 2'd1) ? (a >> sh) : (m == 2'd2) ? ~a : (a & 16'h0FF0);
  assign par = ^a;
  assign allz = ~|a;
endmodule
