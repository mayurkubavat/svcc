// Two-process FSM with a typedef enum state and a package.
package fsm_pkg;
  typedef enum logic [1:0] {IDLE = 2'd0, LOAD = 2'd1, RUN = 2'd2, DONE = 2'd3} state_t;
  localparam logic [3:0] LIMIT = 4'd9;
  function automatic logic [3:0] bump(input logic [3:0] v, input logic by2);
    if (by2) return v + 4'd2;
    return v + 4'd1;
  endfunction
endpackage

module fsm_enum
  import fsm_pkg::*;
(
  input  logic       clk,
  input  logic       rst_n,
  input  logic       start,
  input  logic       by2,
  input  logic       stop,
  output logic       busy,
  output logic       done,
  output logic [3:0] cnt,
  output logic [1:0] state_o
);
  state_t state, next;
  logic [3:0] cnt_n;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      state <= IDLE;
      cnt   <= '0;
    end else begin
      state <= next;
      cnt   <= cnt_n;
    end
  end

  always_comb begin
    next  = state;
    cnt_n = cnt;
    done  = 1'b0;
    case (state)
      IDLE: if (start) next = LOAD;
      LOAD: begin
        cnt_n = 4'd0;
        next  = RUN;
      end
      RUN: begin
        cnt_n = bump(cnt, by2);
        if (stop) next = IDLE;
        else if (cnt >= LIMIT) next = DONE;
      end
      DONE: begin
        done = 1'b1;
        next = IDLE;
      end
      default: next = IDLE;
    endcase
  end

  assign busy    = state != IDLE;
  assign state_o = state;
endmodule
