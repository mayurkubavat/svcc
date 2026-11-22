// Synchronous active-high reset, register without reset, negedge register.
module ff_sync (
  input  logic        clk,
  input  logic        rst,
  input  logic [15:0] d,
  input  logic        we,
  output logic [15:0] q,
  output logic [15:0] q_nr,
  output logic [15:0] q_neg,
  output logic [15:0] q2
);
  logic [15:0] stage;
  always_ff @(posedge clk) begin
    if (rst) q <= 16'h1234;
    else if (we) q <= d;
  end
  always_ff @(posedge clk) q_nr <= d ^ q;
  always_ff @(negedge clk) q_neg <= d + 16'd3;
  always @(posedge clk) begin
    stage <= q_nr;
    q2    <= stage;
  end
endmodule
