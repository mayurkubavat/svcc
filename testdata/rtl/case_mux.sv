// case, casez with wildcards, unique/priority, default, nested if.
module case_mux (
  input  logic [2:0] sel,
  input  logic [7:0] d0, d1, d2, d3,
  input  logic [7:0] req,
  output logic [7:0] y,
  output logic [2:0] grant,
  output logic       valid,
  output logic [3:0] dec
);
  always_comb begin
    unique case (sel)
      3'd0:       y = d0;
      3'd1:       y = d1;
      3'd2, 3'd3: y = d2;
      3'd4:       y = d3;
      default:    y = 8'hff;
    endcase
  end

  always_comb begin
    grant = 3'd0;
    valid = 1'b1;
    casez (req)
      8'b???????1: grant = 3'd0;
      8'b??????10: grant = 3'd1;
      8'b?????100: grant = 3'd2;
      8'b????1000: grant = 3'd3;
      8'b???10000: grant = 3'd4;
      8'b??100000: grant = 3'd5;
      8'b?1000000: grant = 3'd6;
      8'b10000000: grant = 3'd7;
      default:     valid = 1'b0;
    endcase
  end

  always_comb begin
    dec = 4'b0000;
    if (sel[2]) begin
      if (sel[0]) dec = 4'b1000;
      else if (sel[1]) dec = 4'b0100;
    end else begin
      case (sel[1:0])
        2'b00: dec = 4'b0001;
        2'b01: dec = 4'b0010;
        default: ;
      endcase
    end
  end
endmodule
