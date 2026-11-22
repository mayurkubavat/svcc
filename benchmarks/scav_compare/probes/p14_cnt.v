module p14_cnt(
  input clk,
  input rst,
  input en,
  input load,
  input [7:0] d,
  output reg [7:0] q,
  output tc
);
  always @(posedge clk) begin
    if (rst) begin
      q <= 8'd0;
    end
    else if (load) begin
      q <= d;
    end
    else if (en) begin
      q <= q + 8'd1;
    end
  end
  assign tc = (q == 8'hFF) & en;
endmodule
