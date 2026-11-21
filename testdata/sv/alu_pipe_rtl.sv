// Synthesizable 2-stage pipelined 32-bit ALU in IEEE 1800-2023 SystemVerilog RTL
module alu_pipe (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [1:0]  op,
  input  logic [31:0] a,
  input  logic [31:0] b,
  output logic [31:0] result
);
  logic [1:0]  op_s1;
  logic [31:0] a_s1;
  logic [31:0] b_s1;
  logic [31:0] alu_comb;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      op_s1 <= 2'd0;
      a_s1  <= 32'd0;
      b_s1  <= 32'd0;
    end else begin
      op_s1 <= op;
      a_s1  <= a;
      b_s1  <= b;
    end
  end

  always_comb begin
    alu_comb = a_s1 + b_s1;
    if (op_s1 == 2'd1) begin
      alu_comb = a_s1 - b_s1;
    end
    if (op_s1 == 2'd2) begin
      alu_comb = a_s1 & b_s1;
    end
    if (op_s1 == 2'd3) begin
      alu_comb = a_s1 ^ b_s1;
    end
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      result <= 32'd0;
    end else begin
      result <= alu_comb;
    end
  end
endmodule
