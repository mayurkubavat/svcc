// Generate loops driving one vector bit by bit, and a generate ripple-carry chain.
module full_adder (input logic a, b, cin, output logic s, cout);
  assign s    = a ^ b ^ cin;
  assign cout = (a & b) | (cin & (a ^ b));
endmodule

module gen_bits #(parameter int N = 6) (
  input  logic [N-1:0] a,
  input  logic [N-1:0] b,
  input  logic         cin,
  output logic [N-1:0] sum,
  output logic         cout,
  output logic [N-1:0] gray,
  output logic [N-1:0] masked
);
  logic [N:0] c;
  assign c[0] = cin;
  for (genvar i = 0; i < N; i++) begin : g_fa
    full_adder u_fa (.a(a[i]), .b(b[i]), .cin(c[i]), .s(sum[i]), .cout(c[i+1]));
  end
  assign cout = c[N];
  for (genvar i = 0; i < N - 1; i++) begin : g_gray
    assign gray[i] = a[i] ^ a[i+1];
  end
  assign gray[N-1] = a[N-1];
  for (genvar i = 0; i < N; i++) begin : g_mask
    if (i % 2 == 0) begin : g_even
      assign masked[i] = a[i] & b[i];
    end else begin : g_odd
      always_comb masked[i] = a[i] | b[i];
    end
  end
endmodule
