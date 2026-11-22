// Signed arithmetic: sign extension, mixed signedness, casts, arithmetic shift.
module signed_arith (
  input  logic signed [7:0]   a,
  input  logic signed [7:0]   b,
  input  logic [7:0]          u,
  input  logic signed [3:0]   n,
  input  logic [2:0]          sh,
  output logic signed [15:0]  wide_sum,
  output logic signed [15:0]  mul,
  output logic [15:0]         mixed,
  output logic signed [8:0]   diff,
  output logic signed [7:0]   sra,
  output logic [7:0]          srl,
  output logic signed [7:0]   absv,
  output logic signed [7:0]   mn,
  output logic                neg,
  output logic signed [15:0]  cast_s,
  output logic [15:0]         cast_u,
  output logic signed [7:0]   modv,
  output logic signed [11:0]  acc
);
  assign wide_sum = a + b;
  assign mul      = a * b;
  assign mixed    = a + u;
  assign diff     = a - b;
  assign sra      = a >>> sh;
  assign srl      = a >> sh;
  assign absv     = (a < 0) ? -a : a;
  assign mn       = (a < b) ? a : b;
  assign neg      = a < 8'sd0;
  assign cast_s   = $signed(u) + n;
  assign cast_u   = $unsigned(a) + 16'd0;
  assign modv     = a % b;
  assign acc      = n * 12'sd3 - a;
endmodule
