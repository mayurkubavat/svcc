// Asynchronous active-low reset, enable, synchronous clear, up/down, terminal count.
module ff_counter (
  input  logic       clk,
  input  logic       rst_n,
  input  logic       en,
  input  logic       clr,
  input  logic       up,
  input  logic [7:0] load_val,
  input  logic       load,
  output logic [7:0] count,
  output logic       tc
);
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)
      count <= 8'd0;
    else if (clr)
      count <= '0;
    else if (load)
      count <= load_val;
    else if (en) begin
      if (up) count <= count + 8'd1;
      else    count <= count - 8'd1;
    end
  end
  assign tc = (count == 8'hff) & up | (count == 8'h00) & ~up;
endmodule
