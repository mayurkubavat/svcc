// Parameterized modules, two specializations, expressions on ports, localparam, $clog2.
module add_reg #(
  parameter int W = 8,
  parameter bit REGISTERED = 1'b1
) (
  input  logic         clk,
  input  logic         rst_n,
  input  logic [W-1:0] a,
  input  logic [W-1:0] b,
  output logic [W:0]   s
);
  localparam int SW = W + 1;
  logic [SW-1:0] sum;
  assign sum = a + b;
  if (REGISTERED) begin : g_reg
    always_ff @(posedge clk or negedge rst_n)
      if (!rst_n) s <= '0;
      else        s <= sum;
  end else begin : g_comb
    assign s = sum;
  end
endmodule

module wild_add (
  input  logic       clk,
  input  logic       rst_n,
  input  logic [7:0] wa,
  input  logic [7:0] wb,
  output logic [8:0] s
);
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) s <= '0;
    else        s <= wa + wb;
endmodule

module hier_param (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [7:0]  a,
  input  logic [7:0]  b,
  input  logic [15:0] c,
  output logic [8:0]  s8,
  output logic [16:0] s16,
  output logic [3:0]  lg,
  output logic        cout,
  output logic [8:0]  wsum
);
  localparam int DEPTH = 12;
  logic [16:0] s16_i;
  add_reg #(.W(8)) u_add8 (.clk(clk), .rst_n(rst_n), .a(a), .b(b), .s(s8));
  add_reg #(.W(16), .REGISTERED(1'b0)) u_add16 (
    .clk, .rst_n, .a({a, b}), .b(c + 16'd1), .s(s16_i));
  // Wildcard and implicit named connections.
  logic [7:0] wa, wb;
  logic [8:0] ws;
  assign wa = a ^ 8'h0f;
  assign wb = b;
  wild_add u_wild (.*, .s(ws));
  assign wsum = ws;
  assign s16  = s16_i;
  assign cout = s16_i[16];
  assign lg   = $clog2(DEPTH);
endmodule
