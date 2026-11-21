module crc8_step_comb (
  input  logic       clk,
  input  logic       rst_n,
  input  logic [7:0] crc_in,
  input  logic [7:0] data_in,
  output logic [7:0] result
);
  logic [7:0] mixed;

  always_comb begin
    mixed = crc_in ^ data_in;
    if ((mixed & 8'd1) == 8'd1) begin
      result = (mixed >> 1) ^ 8'h8C;
    end else begin
      result = (mixed >> 1);
    end
  end
endmodule
