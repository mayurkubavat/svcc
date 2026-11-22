// Packed struct and packed array ports and variables.
package bus_pkg;
  typedef struct packed {
    logic        valid;
    logic [2:0]  id;
    logic [7:0]  data;
  } req_t;
endpackage

module structs (
  input  logic                 clk,
  input  logic                 rst_n,
  input  bus_pkg::req_t        req_i,
  input  logic [3:0][7:0]      lanes,
  input  logic [1:0]           lane_sel,
  output bus_pkg::req_t        req_q,
  output logic [7:0]           lane,
  output logic [3:0][7:0]      lanes_rev,
  output logic [11:0]          flat
);
  bus_pkg::req_t nxt;
  always_comb begin
    nxt = req_i;
    if (req_i.valid && req_i.id == 3'd7) nxt.data = ~req_i.data;
    nxt.id = req_i.id + 3'd1;
  end
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) req_q <= '0;
    else if (nxt.valid) req_q <= nxt;
  assign lane = lanes[lane_sel];
  always_comb for (int i = 0; i < 4; i++) lanes_rev[i] = lanes[3 - i];
  assign flat = req_q;
endmodule
