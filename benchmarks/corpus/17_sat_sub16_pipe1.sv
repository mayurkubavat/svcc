module sat_sub16_pipe1 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [15:0] x,
  input  logic [15:0] y,
  output logic [15:0] result
);
  logic [15:0] diff_comb;

  always_comb begin
    if (x >= y) begin
      diff_comb = x - y;
    end else begin
      diff_comb = 16'd0;
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 16'd0;
    end else begin
      result <= diff_comb;
    end
  end
endmodule
