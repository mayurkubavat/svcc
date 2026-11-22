// Unpacked array ports, whole-array assignment, assignment patterns, array ports through hierarchy.
module arr_stage #(parameter int N = 4) (
  input  logic       clk,
  input  logic       en,
  input  logic [7:0] d [N],
  output logic [7:0] q [N]
);
  always_ff @(posedge clk) if (en) q <= d;
endmodule

module array_ports (
  input  logic        clk,
  input  logic        rst,
  input  logic        en,
  input  logic [1:0]  sel,
  input  logic [7:0]  in_a [4],
  input  logic [3:0]  grid_i [2][3],
  output logic [7:0]  out_q [4],
  output logic [7:0]  picked,
  output logic [9:0]  total,
  output logic [3:0]  grid_o [2][3],
  output logic [7:0]  held [4]
);
  logic [7:0] stage_q [4];
  logic [7:0] next   [4];

  arr_stage #(.N(4)) u_stage (.clk(clk), .en(en), .d(in_a), .q(stage_q));

  always_comb begin
    next = stage_q;
    next[sel] = stage_q[sel] + 8'd1;
  end

  always_ff @(posedge clk) begin
    if (rst) held <= '{default: 8'hAA};
    else if (en) held <= next;
  end

  assign out_q  = stage_q;
  assign picked = in_a[sel] ^ stage_q[2'd3 - sel];

  always_comb begin
    total = '0;
    for (int i = 0; i < 4; i++) total += in_a[i];
  end

  always_comb begin
    for (int r = 0; r < 2; r++)
      for (int c = 0; c < 3; c++)
        grid_o[r][c] = grid_i[1 - r][c] + 4'(c);
  end
endmodule
