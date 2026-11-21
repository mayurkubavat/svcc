module minmax32_pipe1 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic        pick_max,
  input  logic [31:0] a,
  input  logic [31:0] b,
  output logic [31:0] result
);
  logic [31:0] sel_comb;

  always_comb begin
    sel_comb = a;
    if (pick_max) begin
      if (b > a) begin
        sel_comb = b;
      end
    end else begin
      if (b < a) begin
        sel_comb = b;
      end
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 32'd0;
    end else begin
      result <= sel_comb;
    end
  end
endmodule
