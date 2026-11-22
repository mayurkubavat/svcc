// Blocking assignments read back inside the same always_comb, loops, partial assigns.
module blocking_comb (
  input  logic [7:0]  a,
  input  logic [7:0]  b,
  input  logic [1:0]  mode,
  output logic [7:0]  y,
  output logic [3:0]  ones,
  output logic [7:0]  rev,
  output logic [7:0]  pri,
  output logic [15:0] acc
);
  logic [7:0] t;
  always_comb begin
    t = a;
    if (mode[0]) t = t + b;
    if (mode[1]) t = t ^ 8'h5a;
    y = t;
  end

  always_comb begin
    ones = 4'd0;
    for (int i = 0; i < 8; i++) begin
      ones = ones + a[i];
      rev[i] = b[7 - i];
    end
  end

  always_comb begin
    pri = 8'd0;
    for (int i = 7; i >= 0; i--)
      if (a[i]) pri = i[7:0] + 8'd1;
  end

  always_comb begin
    acc = 16'd0;
    acc[7:0] = a;
    acc[15:8] = acc[7:0] + b;
    acc[0] = mode[0];
  end
endmodule
