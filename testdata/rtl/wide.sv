// Vectors wider than 64 bits.
module wide (
  input  logic          clk,
  input  logic [127:0]  a,
  input  logic [127:0]  b,
  input  logic [6:0]    sh,
  input  logic [63:0]   lo64,
  output logic [127:0]  sum,
  output logic [127:0]  bw,
  output logic [127:0]  shl,
  output logic [127:0]  shr,
  output logic [63:0]   mid,
  output logic          eq,
  output logic          lt,
  output logic [127:0]  cat,
  output logic [199:0]  ext,
  output logic [127:0]  q,
  output logic          anybit,
  output logic [64:0]   s65
);
  assign sum    = a + b;
  assign bw     = (a & ~b) ^ {2{lo64}};
  assign shl    = a << sh;
  assign shr    = a >> sh;
  assign mid    = a[95:32];
  assign eq     = a == b;
  assign lt     = a < b;
  assign cat    = {lo64, a[127:64]};
  assign ext    = {a, b[71:0]} + 200'd1;
  assign anybit = |a[100:70];
  assign s65    = lo64 + a[63:0];
  always_ff @(posedge clk) q <= q + a - 128'd1;
endmodule
