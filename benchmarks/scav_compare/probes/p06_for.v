module p06_for(
  input [15:0] d,
  output reg [4:0] cnt
);
  integer i;
  always @(*) begin
    cnt = 0;
    for (i = 0; i < 16; i = i + 1)
      cnt = cnt + d[i];
  end
endmodule
