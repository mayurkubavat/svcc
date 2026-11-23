# Who is right when svcc and Verilator differ

The M5.2 stress run (svcc 16a53ca, svlang 0.5.1, Verilator 5.052) left five modules where
the generated SystemC and Verilator disagree. Each was settled three ways:

1. **Third simulator.** `benchmarks/rtl_stress/adjudicate.py` replays the exact stimulus of the
   failing `rtl-cosim` run (same harness, same seed) with a trace of every input and both
   models' outputs, then drives the same input sequence into Icarus Verilog 14 (devel). Sources
   Icarus cannot parse (fpnew) go through yosys 0.70 with the slang front end into a netlist
   first, and Icarus simulates that netlist. Icarus is 4-state, so an `x` in its output marks an
   output that the source leaves unspecified for that input.
2. **The language reference**, IEEE 1800-2023, for the construct the difference comes from.
3. **The IEEE 754 / RISC-V reference** for the floating-point results (canonical NaN
   `0x7ff8000000000000` for binary64).

| Module (copies) | Difference | Third simulator | Cause | Wrong side |
| :--- | :--- | :--- | :--- | :--- |
| `fpnew_cast_multi` (cvfpu, cv32e40p) | result: SystemC `0x7ff0000000000000` (+Inf), Verilator `0x7ff8000000000000` (qNaN) | inconclusive (`x`; the step drives `op_i = CLASSIFY`, `rnd_mode_i = DYN`) | `localparam logic [MAN_BITS-1:0] QNAN_MANTISSA = 2**(MAN_BITS-1)` evaluated at 32 bits by svlang 0.5.1, so the qNaN mantissa became 0. IEEE 1800 11.6.1: the operands of `**` take the width of the assignment context (52 bits), so `2**51` must not wrap. | **svcc** (svlang). Fixed by svlang af574c9, not yet on crates.io: with it both copies match Verilator over 3,000 steps. |
| `fpnew_fma_multi` (cvfpu, cv32e40p), first difference | result: SystemC +Inf, Verilator qNaN | **agrees with Verilator** (`0x7ff8000000000000`) | Same `QNAN_MANTISSA = 2**(MAN_BITS-1)` defect. At that step the addend is not NaN-boxed (`is_boxed_i[FP64][2] = 0`), so the classifier marks it NaN and the RTL selects the canonical qNaN. | **svcc** (svlang). Fixed by af574c9. |
| `fpnew_fma_multi`, after af574c9 | e.g. SystemC `0x0`, Verilator `0x7ff8000000000000` at step 20; `...ff00` vs `...ff7e` at step 16 | `x` on `result_o` and `status_o` | The stimulus drives `src_fmt_i` with 5 or 7, which is not a member of `fp_format_e` (5 formats). `info_q[src_fmt_q]` and the other per-format arrays are then read out of bounds, and IEEE 1800 7.4.6 / 11.5.1 make that read return x for a 4-state type. Any 2-state value is a legal resolution. | **Neither.** Harness stimulus: enum-typed ports should be driven only with enumerated values. |
| `stream_mux` (cv32e40p) | `inp_ready_o`, `oup_data_o`, `oup_valid_o`: SystemC 0, Verilator 1 | `inp_ready_o` **agrees with SystemC** (0); the two read outputs are `x` | Default `N_INP = 0` gives `[-1:0]` ranges, and `inp_sel_i = 2` is out of range. The write `inp_ready_o[inp_sel_i] = oup_ready_i` must leave the vector unchanged (IEEE 1800 11.5.1; Icarus ignores it); Verilator writes a bit anyway (bit 0, i.e. index -1). The reads `inp_data_i[2]`, `inp_valid_i[2]` return x (4-state), so either 2-state value is legal. | **Verilator** on the write (verilator/verilator#9329); neither on the reads. Degenerate configuration (the RTL asserts `N_INP >= 1` outside Verilator). |

Summary: of the five differing modules, svlang caused two (both copies of the
`2**N`-in-a-sized-parameter defect; svlang fixed it upstream), Verilator caused one write on
an out-of-range index, and the rest come from driving an enum port with values outside the enum,
where IEEE 1800 leaves the output unspecified. The yosys-slang + Icarus reference agreed with
Verilator wherever it gave a value, and with svcc on the `stream_mux` write. It did not disagree
with both 2-state models on any output it fully specified.

Reproduce (paths for the linux box):

```
python3 benchmarks/rtl_stress/adjudicate.py --tools ~/Tools/oss-cad-suite/bin \
  --census-tool <svlang>/tools/quality --corpus <rtl-census> \
  --design cvfpu --module fpnew_fma_multi --work <rtl-cosim work dir of the failing run> \
  --out <dir> --define VERILATOR --via-yosys
```

`--define VERILATOR` only removes the elaboration-time assertions these sources guard with
`ifndef VERILATOR`. Icarus refuses `$fatal("msg")` without a finish number.

## Proof failures are not design differences

The proof run on the matched modules (`rtl_prove_corpus.md`) found no functional difference.
Its three `DIFFERENCE FOUND` results (`axi_interconnect`, `axil_interconnect`,
`axil_crossbar_addr`, all verilog-axi) stop before any solver call: the note says the start
values differ for `svlang_latch_0` and `svlang_latch_1`. These are the module-level loop
variables `integer i, j`. The `always @*` block writes them only inside an `if`, so svlang
models them as latches. Every read of them follows a write in the same block, so their stored
value never reaches an output. The check in `svcc-equiv/src/pair.rs` that start values agree
is stricter than it needs to be for such states: a precision gap in the proof, not a bug in the
translation.
