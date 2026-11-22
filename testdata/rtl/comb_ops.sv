// Operators with mixed widths and signedness (IEEE 1800-2023 11.6, 11.8).
module comb_ops (
  input  logic [7:0]         a,
  input  logic [7:0]         b,
  input  logic signed [7:0]  sa,
  input  logic signed [7:0]  sb,
  input  logic [3:0]         sh,
  input  logic [15:0]        w,
  output logic [8:0]         sum9,
  output logic [7:0]         diff,
  output logic [15:0]        prod,
  output logic [7:0]         quot,
  output logic [7:0]         rem,
  output logic [7:0]         bw,
  output logic [7:0]         shl,
  output logic [7:0]         shr,
  output logic signed [7:0]  ashr,
  output logic signed [15:0] sprod,
  output logic signed [7:0]  sdiv,
  output logic               lt_u,
  output logic               lt_s,
  output logic               lt_mixed,
  output logic [5:0]         red,
  output logic               logic_ops,
  output logic [15:0]        ext,
  output logic [7:0]         avg,
  output logic [7:0]         neg
);
  assign sum9      = a + b;
  assign diff      = a - b;
  assign prod      = a * b;
  assign quot      = a / b;
  assign rem       = a % b;
  assign bw        = (a & b) | (a ^ ~b) | (a ~^ 8'h0f);
  assign shl       = a << sh;
  assign shr       = a >> sh;
  assign ashr      = sa >>> sh;
  assign sprod     = sa * sb;
  assign sdiv      = sa / sb;
  assign lt_u      = a < b;
  assign lt_s      = sa < sb;
  assign lt_mixed  = sa < b;
  assign red       = {&a, |a, ^a, ~&a, ~|a, ~^a};
  assign logic_ops = (a && b) || !(w != 16'd0) || (a == b);
  assign ext       = sa + w;
  assign avg       = (a + b) >> 1;
  assign neg       = -a;
endmodule
