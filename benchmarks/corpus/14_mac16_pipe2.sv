module mac16_pipe2 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [15:0] a,
  input  logic [15:0] b,
  input  logic [31:0] c,
  output logic [31:0] result
);
  logic [31:0] prod_s1;
  logic [31:0] c_s1;
  logic [31:0] prod_comb;

  always_comb begin
    prod_comb = a * b;
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      prod_s1 <= 32'd0;
      c_s1    <= 32'd0;
    end else begin
      prod_s1 <= prod_comb;
      c_s1    <= c;
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 32'd0;
    end else begin
      result <= prod_s1 + c_s1;
    end
  end
endmodule
