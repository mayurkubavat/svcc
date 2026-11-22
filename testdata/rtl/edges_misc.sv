// Edges on internal signals and expressions, latches with nonblocking assignments,
// an instance named like its module, and a long replication.
package em_pkg;
  typedef struct packed {
    logic       rst_n;
    logic [3:0] seed;
  } ctl_t;
endpackage

module pulse_gen (input logic clk, input logic rst_n, input logic d, output logic q);
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) q <= 1'b0;
    else        q <= d;
endmodule

module edges_misc (
  input  logic          clk,
  input  em_pkg::ctl_t  ctl,
  input  logic          d,
  input  logic          le,
  input  logic [7:0]    din,
  output logic [3:0]    slow_cnt,
  output logic [3:0]    cnt,
  output logic          q,
  output logic [7:0]    latched,
  output logic [7:0]    latched_nb,
  output logic [149:0]  fill,
  output logic [7:0]    halves
);
  // Clock divider: the second register is clocked by an internal signal.
  logic div;
  always_ff @(posedge clk or negedge ctl.rst_n)
    if (!ctl.rst_n) div <= 1'b0;
    else            div <= ~div;
  always_ff @(posedge div or negedge ctl.rst_n)
    if (!ctl.rst_n) slow_cnt <= ctl.seed;
    else            slow_cnt <= slow_cnt + 4'd1;

  // Reset taken from a struct member; negative-edge clock.
  always_ff @(negedge clk or negedge ctl.rst_n)
    if (!ctl.rst_n) cnt <= '0;
    else            cnt <= cnt + {3'd0, d};

  // Instance with the same name as its module.
  pulse_gen pulse_gen (.clk(clk), .rst_n(ctl.rst_n), .d(d), .q(q));

  always_latch if (le) latched = din;
  always_latch begin
    if (le) latched_nb <= ~din;
  end

  assign fill = {75{d, le}};

  // Two blocks with the same expression edge driving parts of one vector.
  always_ff @(posedge clk or negedge ctl.rst_n)
    if (!ctl.rst_n) halves[3:0] <= '0;
    else            halves[3:0] <= din[3:0];
  always_ff @(posedge clk or negedge ctl.rst_n)
    if (!ctl.rst_n) halves[7:4] <= 4'hf;
    else if (d)     halves[7:4] <= din[7:4];
endmodule
