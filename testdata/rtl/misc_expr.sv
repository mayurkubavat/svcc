// Conditional operator nesting, inside, system functions, compound assignments, enum math.
module misc_expr (
  input  logic [7:0]  a,
  input  logic [7:0]  b,
  input  logic [1:0]  s,
  output logic [7:0]  y,
  output logic        in_set,
  output logic [3:0]  ones,
  output logic        onehot,
  output logic        onehot0,
  output logic [7:0]  cmp,
  output logic [7:0]  comp,
  output logic [31:0] bits,
  output logic [7:0]  wr
);
  localparam logic [7:0] K [4] = '{8'd1, 8'd2, 8'd4, 8'd8};
  assign y       = s[1] ? (s[0] ? a : b) : (s[0] ? a + b : a - b);
  assign in_set  = a inside {8'd3, 8'd7, [8'd20:8'd40], 8'd99};
  assign ones    = $countones(a);
  assign onehot  = $onehot(a);
  assign onehot0 = $onehot0(b);
  assign cmp     = {a == b, a != b, a < b, a <= b, a > b, a >= b, a === b, a !== b};
  always_comb begin
    comp = a;
    comp += b;
    comp ^= 8'h3c;
    comp <<= 1;
    comp |= {7'd0, s[0]};
    comp++;
  end
  assign bits = $bits(a) + $bits(s);
  assign wr   = K[s] + (8'd200 + 8'd100);
endmodule
