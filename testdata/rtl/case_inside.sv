// case inside with value lists, ranges and wildcard items.
module case_inside (
  input  logic [7:0] op,
  input  logic [3:0] n,
  output logic [2:0] cls,
  output logic       is_small
);
  always_comb begin
    case (op) inside
      8'h00, 8'hff:        cls = 3'd0;
      [8'h01:8'h0f]:       cls = 3'd1;
      8'b0001_????:        cls = 3'd2;
      8'b1???_???0, 8'h21: cls = 3'd3;
      [8'h40:8'h7f]:       cls = 3'd4;
      default:             cls = 3'd7;
    endcase
  end
  always_comb begin
    unique case (n) inside
      [4'd0:4'd3]: is_small = 1'b1;
      default:     is_small = 1'b0;
    endcase
  end
endmodule
