module p08_function(
  input [7:0] a,
  input [7:0] b,
  output [7:0] y
);
  function [7:0] maxf;
    input [7:0] x;
    input [7:0] z;
    begin
      if (x > z) begin
        maxf = x;
      end
      else begin
        maxf = z;
      end
    end
  endfunction
  assign y = maxf(a, b);
endmodule
