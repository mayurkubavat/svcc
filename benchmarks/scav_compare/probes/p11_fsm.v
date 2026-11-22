module p11_fsm(
  input clk,
  input rst_n,
  input start,
  input done,
  output reg busy,
  output reg [1:0] state
);
  localparam IDLE = 2'd0, RUN = 2'd1, FIN = 2'd2;
  reg [1:0] nstate;
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      state <= IDLE;
    end
    else begin
      state <= nstate;
    end
  end
  always @(*) begin
    nstate = state;
    busy = 1'b0;
    case (state)
      IDLE: if (start) nstate = RUN;
      RUN: begin busy = 1'b1; if (done) nstate = FIN; end
      FIN: nstate = IDLE;
      default: nstate = IDLE;
    endcase
  end
endmodule
