// Clock-gating cells: a latch transparent while the clock is low, ANDed with the clock.
// Registers on the gated clock become registers on the root clock with an enable, both in
// the module that gates and in instances whose clock port is gated.

// PULP / OpenTitan style cell.
module cg_cell (input logic clk_i, input logic en_i, input logic test_en_i, output logic clk_o);
  logic clk_en;
  always_latch begin
    if (clk_i == 1'b0) clk_en <= en_i | test_en_i;
  end
  assign clk_o = clk_i & clk_en;
endmodule

// VeeR style cell, wrapped once more, with an internal constant scan enable.
module cg_icg (input logic SE, input logic EN, input logic CK, output Q);
  logic en_ff;
  logic enable;
  assign enable = EN | SE;
  always @(CK, enable) begin
    if (!CK) en_ff = enable;
  end
  assign Q = CK & en_ff;
endmodule

module cg_hdr (input logic en, input logic clk, output logic l1clk);
  logic SE;
  assign SE = 0;
  cg_icg clkhdr (.*, .EN(en), .CK(clk), .Q(l1clk));
endmodule

// A register module that only sees its clock port.
module cg_dff #(parameter int W = 8) (
  input  logic         clk,
  input  logic         rst_l,
  input  logic [W-1:0] din,
  output logic [W-1:0] dout
);
  always_ff @(posedge clk or negedge rst_l)
    if (!rst_l) dout <= '0;
    else        dout <= din;
endmodule

// A level above that passes its clock to an instance.
module cg_dff_wrap (input logic clk, input logic rst_l, input logic [7:0] d, output logic [7:0] q);
  cg_dff #(.W(8)) u (.clk(clk), .rst_l(rst_l), .din(d), .dout(q));
endmodule

// A gating cell whose enable comes from a register inside its own module, as in a sleep unit.
module cg_sleep (input logic clk, input logic rst_l, input logic wake, output logic clk_gated);
  logic awake_q;
  always_ff @(posedge clk or negedge rst_l)
    if (!rst_l) awake_q <= 1'b1;
    else        awake_q <= wake | ~awake_q;
  cg_cell u_cg (.clk_i(clk), .en_i(awake_q), .test_en_i(1'b0), .clk_o(clk_gated));
endmodule

// Clock buffer (two inversions) and clock inverter cells.
module cg_buf (input logic clk_i, output logic clk_o);
  logic inv;
  assign inv = ~clk_i;
  assign clk_o = ~inv;
endmodule

module cg_inv (input logic clk_i, output logic clk_o);
  assign clk_o = ~clk_i;
endmodule

// A "clock" made by logic that only reads registers, here a pulse from a small FIFO-like
// valid flag. It changes after the nonblocking update in both languages, like a divider.
module cg_pulse_src (input logic clk, input logic rst_l, input logic push, output logic valid);
  logic full_q, take_q;
  always_ff @(posedge clk or negedge rst_l)
    if (!rst_l) begin full_q <= 1'b0; take_q <= 1'b0; end
    else begin full_q <= push | (full_q & ~take_q); take_q <= full_q; end
  assign valid = full_q & ~take_q;
endmodule

module clock_gating (
  input  logic       clk,
  input  logic       rst_l,
  input  logic       en_a,
  input  logic       en_b,
  input  logic       en_c,
  input  logic       scan,
  input  logic [7:0] d,
  output logic [7:0] q_local,
  output logic [7:0] q_child,
  output logic [7:0] q_wrap,
  output logic [7:0] q_nested,
  output logic [7:0] q_count,
  input  logic       wake,
  output logic [7:0] q_sleep,
  output logic [7:0] q_buf,
  output logic [7:0] q_inv,
  output logic [7:0] q_inv_child,
  output logic [7:0] q_pulse
);
  logic gclk_a, gclk_b, gclk_bc;
  logic en_q;

  cg_cell u_cg_a (.clk_i(clk), .en_i(en_a), .test_en_i(scan), .clk_o(gclk_a));
  cg_hdr  u_cg_b (.en(en_b & en_q), .clk(clk), .l1clk(gclk_b));
  // A gated clock gated again.
  cg_cell u_cg_bc (.clk_i(gclk_b), .en_i(en_c), .test_en_i(1'b0), .clk_o(gclk_bc));

  // The enable can come from a register on the root clock.
  always_ff @(posedge clk or negedge rst_l)
    if (!rst_l) en_q <= 1'b0;
    else        en_q <= ~en_q | d[0];

  // Register in this module on a gated clock, with an asynchronous reset.
  always_ff @(posedge gclk_a or negedge rst_l)
    if (!rst_l) q_local <= 8'h00;
    else        q_local <= q_local + d;

  // Registers in instances whose clock port is gated.
  cg_dff #(.W(8)) u_child (.clk(gclk_b), .rst_l(rst_l), .din(d ^ q_local), .dout(q_child));
  cg_dff_wrap     u_wrap  (.clk(gclk_a), .rst_l(rst_l), .d(q_child), .q(q_wrap));
  cg_dff #(.W(8)) u_nest  (.clk(gclk_bc), .rst_l(rst_l), .din(d + 8'd3), .dout(q_nested));

  // Gated clock out of a module whose enable is internal to it.
  logic gclk_s;
  cg_sleep u_sleep (.clk(clk), .rst_l(rst_l), .wake(wake), .clk_gated(gclk_s));
  cg_dff #(.W(8)) u_slp (.clk(gclk_s), .rst_l(rst_l), .din(d ^ 8'h5a), .dout(q_sleep));

  // A buffered clock, and an inverted one (registers on it use the falling edge).
  logic clk_b, clk_n;
  cg_buf u_buf (.clk_i(clk), .clk_o(clk_b));
  cg_inv u_inv (.clk_i(clk), .clk_o(clk_n));
  always_ff @(posedge clk_b or negedge rst_l)
    if (!rst_l) q_buf <= '0;
    else        q_buf <= q_local + 8'd1;
  always_ff @(posedge clk_n or negedge rst_l)
    if (!rst_l) q_inv <= '0;
    else        q_inv <= q_buf ^ d;
  cg_dff #(.W(8)) u_invc (.clk(clk_n), .rst_l(rst_l), .din(q_inv), .dout(q_inv_child));

  // Registers on a clock made from register outputs, in this module and through an instance.
  logic pulse;
  cg_pulse_src u_pulse (.clk(clk), .rst_l(rst_l), .push(d[1]), .valid(pulse));
  always_ff @(posedge pulse or negedge rst_l)
    if (!rst_l) q_pulse <= '0;
    else        q_pulse <= q_pulse + q_buf;

  // A counter on the root clock that reads the gated registers.
  always_ff @(posedge clk or negedge rst_l)
    if (!rst_l) q_count <= '0;
    else        q_count <= q_count + q_local[1:0] + q_child[0];
endmodule
