// Two-file design for `svcc rtl-prove top.sv acc.sv`: the parameter override -G W=12
// changes both the widths and the constant added to the output.
module top #(parameter int W = 8) (input logic clk, input logic rst, input logic [W-1:0] a, b, output logic [W-1:0] y);
  logic [W-1:0] sa, sb;
  acc #(.W(W)) u0 (.clk, .rst, .d(a), .s(sa));
  acc #(.W(W)) u1 (.clk, .rst, .d(b), .s(sb));
  assign y = ((sa + sb) >> 1) + W[7:0];
endmodule
