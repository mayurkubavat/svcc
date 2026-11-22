// Module-level functions: automatic, with loops, locals, and calls to other functions.
module funcs (
  input  logic [7:0]  a,
  input  logic [7:0]  b,
  input  logic [15:0] w,
  output logic [3:0]  pc,
  output logic [7:0]  mx,
  output logic [7:0]  sat,
  output logic        par,
  output logic [7:0]  swapped
);
  function automatic logic [3:0] popcount8(input logic [7:0] v);
    logic [3:0] n;
    n = 4'd0;
    for (int i = 0; i < 8; i++) n = n + v[i];
    return n;
  endfunction

  function automatic logic [7:0] max8(input logic [7:0] x, input logic [7:0] y);
    max8 = (x > y) ? x : y;
  endfunction

  function automatic logic [7:0] sat_add(input logic [7:0] x, input logic [7:0] y);
    logic [8:0] s;
    s = x + y;
    if (s[8]) return 8'hff;
    return s[7:0];
  endfunction

  function automatic logic parity16(input logic [15:0] v);
    return ^v;
  endfunction

  assign pc  = popcount8(a ^ b);
  assign mx  = max8(max8(a, b), w[7:0]);
  assign sat = sat_add(a, b);
  assign par = parity16(w);
  always_comb swapped = max8({a[3:0], a[7:4]}, sat_add(b, 8'd1));
endmodule
