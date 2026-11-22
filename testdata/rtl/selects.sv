// Bit, part and indexed part selects on both sides, concatenation and replication.
module selects (
  input  logic [31:0] d,
  input  logic [4:0]  idx,
  input  logic [1:0]  bsel,
  input  logic [7:0]  x,
  input  logic        bit_in,
  output logic        b,
  output logic [7:0]  byte_c,
  output logic [7:0]  byte_v,
  output logic [7:0]  byte_d,
  output logic [31:0] cat,
  output logic [31:0] rep,
  output logic [15:0] sext,
  output logic [3:0]  hi,
  output logic [3:0]  lo,
  output logic [31:0] upd,
  output logic [0:7]  asc
);
  assign b      = d[idx];
  assign byte_c = d[23:16];
  assign byte_v = d[bsel*8 +: 8];
  assign byte_d = d[(idx | 5'd7) -: 8];
  assign cat    = {x, d[7:0], 8'ha5, {4{bit_in}}, x[3:0]};
  assign rep    = {4{x}};
  assign sext   = {{8{x[7]}}, x};
  assign {hi, lo} = x + 8'd1;
  always_comb begin
    upd = d;
    upd[idx] = bit_in;
    upd[bsel*8 +: 8] = x;
    upd[31:28] = 4'hf;
  end
  always_comb begin
    asc = x;
    asc[0] = bit_in;
    asc[4:7] = d[3:0];
  end
endmodule
