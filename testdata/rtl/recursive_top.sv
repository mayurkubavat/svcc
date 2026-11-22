// A module that instantiates itself (a recursive adder tree) as the top level. The
// reference simulator builds such a top without its children, so rtl-cosim wraps it.
module recursive_top #(parameter int W = 8) (
  input  logic [W-1:0]       d,
  output logic [$clog2(W):0] n
);
  if (W == 1) begin : leaf
    assign n = d;
  end else begin : node
    logic [$clog2(W)-1:0] l, r;
    recursive_top #(.W(W / 2)) u_l (.d(d[W-1:W/2]), .n(l));
    recursive_top #(.W(W / 2)) u_r (.d(d[W/2-1:0]), .n(r));
    assign n = l + r;
  end
endmodule
