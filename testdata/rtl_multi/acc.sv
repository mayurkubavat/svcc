// Accumulator instantiated twice by top.sv.
module acc #(parameter int W = 8) (input logic clk, input logic rst, input logic [W-1:0] d, output logic [W-1:0] s);
  always_ff @(posedge clk) if (rst) s <= '0; else s <= s + d;
endmodule
