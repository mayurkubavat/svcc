module p03_param #(parameter WIDTH = 12, parameter INIT = 5) (
  input clk,
  input rst,
  input en,
  output reg [WIDTH-1:0] q
);
  localparam MAXV = 1000;
  always @(posedge clk) begin
    if (rst) begin
      q <= INIT;
    end
    else if (en) begin
      if (q == MAXV) begin
        q <= 0;
      end
      else begin
        q <= q + 1;
      end
    end
  end
endmodule
