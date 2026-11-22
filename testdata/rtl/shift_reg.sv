// Loops with nonblocking assignments, unpacked array of registers, blocking temporaries in always_ff.
module shift_reg #(parameter int STAGES = 4) (
  input  logic       clk,
  input  logic       rst,
  input  logic       en,
  input  logic [7:0] din,
  output logic [7:0] dout,
  output logic [9:0] total,
  output logic [7:0] crc
);
  logic [7:0] pipe [STAGES];
  logic [9:0] sum_c;
  always_ff @(posedge clk) begin
    if (rst) begin
      for (int i = 0; i < STAGES; i++) pipe[i] <= 8'd0;
    end else if (en) begin
      pipe[0] <= din;
      for (int i = 1; i < STAGES; i++) pipe[i] <= pipe[i-1];
    end
  end
  assign dout = pipe[STAGES-1];

  always_comb begin
    sum_c = '0;
    for (int i = 0; i < STAGES; i++) sum_c += pipe[i];
  end
  always_ff @(posedge clk) total <= sum_c;

  // Blocking temporary inside a clocked block.
  logic [7:0] crc_t;
  always_ff @(posedge clk) begin
    if (rst) crc <= 8'hff;
    else if (en) begin
      crc_t = crc;
      for (int i = 0; i < 8; i++) begin
        if (crc_t[7] ^ din[i]) crc_t = {crc_t[6:0], 1'b0} ^ 8'h07;
        else                   crc_t = {crc_t[6:0], 1'b0};
      end
      crc <= crc_t;
    end
  end
endmodule
