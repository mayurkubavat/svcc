module gray_enc16_pipe1 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [15:0] bin_in,
  output logic [15:0] result
);
  logic [15:0] gray_comb;

  always_comb begin
    gray_comb = bin_in ^ (bin_in >> 1);
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 16'd0;
    end else begin
      result <= gray_comb;
    end
  end
endmodule
