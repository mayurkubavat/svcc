// Gated clocks kept as bits of a packed array, one gating cell per bank, as in VeeR's branch
// history table: each bank's registers follow their own bit back to its gating cell.
module cv_ckg (input logic clk_i, input logic en_i, output logic clk_o);
  logic en_l;
  always_latch if (!clk_i) en_l = en_i;
  assign clk_o = clk_i & en_l;
endmodule
module cv_dff (input logic clk, input logic [1:0] d, output logic [1:0] q);
  always_ff @(posedge clk) q <= d;
endmodule
module clock_vector (input logic clk, input logic [1:0][1:0] en, input logic [1:0] d, output logic [1:0][1:0][1:0] q);
  logic [1:0][1:0] gclk;
  for (genvar i = 0; i < 2; i++) begin : BANKS
    for (genvar k = 0; k < 2; k++) begin : GRP
      cv_ckg cg (.clk_i(clk), .en_i(en[i][k]), .clk_o(gclk[i][k]));
      cv_dff f (.clk(gclk[i][k]), .d(d), .q(q[i][k]));
    end
  end
endmodule
