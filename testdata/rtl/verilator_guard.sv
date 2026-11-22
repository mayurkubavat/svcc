// Source text that tests the VERILATOR macro. svcc does not define it; rtl-cosim also
// undefines it for the reference model, so both models are built from the same text.
module verilator_guard (
  input  logic [3:0] a,
  output logic [3:0] y
);
`ifdef VERILATOR
  assign y = ~a;
`else
  assign y = a ^ 4'b0101;
`endif
endmodule
