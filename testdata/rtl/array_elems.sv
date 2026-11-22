// Array elements driven by different processes (each element has its own writer), a chain
// through the elements, and a latch-gated clock reaching a register through logic.
module array_elems (
  input  logic       clk,
  input  logic       gate_en,
  input  logic [3:0] a,
  input  logic [3:0] b,
  output logic [3:0] y,
  output logic [3:0] q,
  output logic [3:0] p1
);
  logic [3:0] stage [3];
  assign stage[0] = a ^ b;
  always_comb stage[1] = stage[0] + 4'd1;
  assign stage[2][1:0] = stage[1][3:2];
  assign stage[2][3:2] = stage[0][1:0];
  assign y = stage[2];

  logic en_q;
  logic gclk;
  always_latch if (!clk) en_q = gate_en;
  assign gclk = clk & en_q;
  always_ff @(posedge gclk) q <= q + a;

  // A register on a buffered copy of the clock must sample the same values as one on the
  // clock itself.
  logic       clk_buf;
  logic [3:0] p0;
  clk_buffer u_buf (.clk_i(clk), .clk_o(clk_buf));
  always_ff @(posedge clk) p0 <= p0 + b;
  always_ff @(posedge clk_buf) p1 <= p0;
endmodule

module clk_buffer (input logic clk_i, output logic clk_o);
  logic mid;
  assign mid   = clk_i;
  assign clk_o = mid;
endmodule
