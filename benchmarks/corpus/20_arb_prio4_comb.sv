module arb_prio4_comb (
  input  logic       clk,
  input  logic       rst_n,
  input  logic [3:0] req,
  output logic [3:0] result
);
  always_comb begin
    result = 4'd0;
    if ((req & 4'd8) != 4'd0) begin
      result = 4'd8;
    end
    if ((req & 4'd4) != 4'd0) begin
      result = 4'd4;
    end
    if ((req & 4'd2) != 4'd0) begin
      result = 4'd2;
    end
    if ((req & 4'd1) != 4'd0) begin
      result = 4'd1;
    end
  end
endmodule
