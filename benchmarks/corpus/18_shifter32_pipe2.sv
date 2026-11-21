module shifter32_pipe2 (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [31:0] din,
  input  logic [4:0]  shamt,
  input  logic        dir_right,
  output logic [31:0] result
);
  logic [31:0] din_s1;
  logic [4:0]  shamt_s1;
  logic        dir_s1;
  logic [31:0] shifted_comb;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      din_s1   <= 32'd0;
      shamt_s1 <= 5'd0;
      dir_s1   <= 1'd0;
    end else begin
      din_s1   <= din;
      shamt_s1 <= shamt;
      dir_s1   <= dir_right;
    end
  end

  always_comb begin
    if (dir_s1) begin
      shifted_comb = din_s1 >> shamt_s1;
    end else begin
      shifted_comb = din_s1 << shamt_s1;
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 32'd0;
    end else begin
      result <= shifted_comb;
    end
  end
endmodule
