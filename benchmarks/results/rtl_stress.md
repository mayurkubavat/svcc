# Typed RTL path on real designs

Stress run of `svcc rtl2sc` and `svcc rtl-cosim` on 15 open-source designs, 2,184 modules,
each module tried as a top with its default parameters. Three stages: convert, compile the
generated header with g++ against SystemC, simulate against Verilator for 3,000 random steps.

Tools: SystemC `development` 9767bff (C++17), Verilator 5.052, svlang 478a0d1 (the pinned
revision). Corpus: the pinned checkouts of the svlang front-end census (picorv32, serv,
biriscv, verilog-axi, scr1, tech_cells_generic, common_cells, axi, cvfpu, ibex, cv32e40p,
cv32e40x, cva6, caliptra-rtl, opentitan, basejump_stl), kept outside this repository. No
third-party source is copied here; the corpus cases added to `testdata/rtl` were written for
this repository.

Reproduce:

```
python3 benchmarks/rtl_stress/run_stress.py --census-tool <svlang>/tools/quality \
    --corpus <corpus dir> --stage cosim --steps 3000 --out rtl_stress.jsonl
python3 benchmarks/rtl_stress/run_stress.py --report rtl_stress.jsonl
```

## Summary

| | Modules |
| :--- | ---: |
| Tried | 2,184 |
| Rejected by svlang before lowering | 217 |
| Rejected by svcc with a located error | 763 |
| Out of memory (2 GB cap) or timeout (120 s) | 13 |
| Converted | 1,191 |
| Generated C++ compiles against SystemC | 1,191 |
| Matches Verilator | 1,170 |
| Differs from Verilator | 14 |
| Comparison not built or aborted | 7 |

Many rejections are not real gaps: a module tried alone with placeholder defaults
(`parameter type T = logic`, a width of 0 or -1, an unconnected interface port) is not a
valid top. The census that owns this corpus counts 557 such modules.

## Defects found and fixed in svcc during this run

Each of these produced wrong SystemC without any error, and each now has a corpus case.

1. **Array write-back.** A process holding a local copy of an array wrote every element back,
   including elements driven by other processes. Result: lost updates, or a SystemC
   multiple-driver abort. (`array_elems.sv`)
2. **`.*` connections.** Only the first port of a wildcard connection was bound; the rest
   were tied to 0 or left open. This was 57 mismatching Caliptra modules. (`hier_param.sv`)
3. **Clock copies.** A register clocked by `assign gclk = clk;` or by a buffer instance ran
   one delta cycle after the registers on `clk` and sampled their new values.
   (`array_elems.sv`)
4. Compile failures: an edge on an internal signal used a port-only call; an instance named
   like its module; a module named like a C library function (`read`, `sync`).

Two fail-closed checks were added because of what the run showed: a signal that is read (or an
output port) with no driver is an error, and so is a constant select outside the declared
range. Both used to read 0 silently.

## The 14 remaining differences

| Modules | Cause | Whose problem |
| ---: | :--- | :--- |
| 3 | `parameter logic [W-1:0] P = '1` is evaluated as 1, not all ones (`lfsr`, `cc_lfsr`) | svlang |
| 2 | Clock divider that relies on a blocking assignment in an edge-triggered block taking effect before the nonblocking updates of the same edge (`cc_clk_int_div`) | svcc mapping limit |
| 2 | Register on a clock that passes through a clock-gating cell runs one delta after registers on the root clock (`generic_fifo`) | svcc mapping limit |
| 1 | Recursive module: Verilator builds the top level without its children, so the reference is wrong and svcc's value is the plausible one (`popcount`) | reference |
| 1 | Source differs under `ifndef VERILATOR` (`prim_pad_attr`) | reference |
| 1 | Select with a computed index that is out of range because the default parameter is a placeholder (`stream_mux`, `N_INP = 0`) | undefined in two-state |
| 4 | Not yet explained: `fpnew_cast_multi` (differs at time zero) and `fpnew_opgroup_block`, each present in two designs | open |

## Defects found in svlang

All three are in svlang's elaborated design model, are present on svlang main (0.5.0,
fa379f2), and also change what `svlang netlist` produces, so they matter beyond svcc.

1. **Generate block silently dropped.** `if (|PARAM) begin ... end else begin ... end`
   elaborates neither branch and reports nothing. Seen in serv (`|WITH_CSR`). svcc now
   usually catches the consequence (an output with no driver), not the cause.
2. **Fill literal as a sized parameter default.** `parameter logic [W-1:0] P = '1` gives 1.
   The text hierarchy (`svlang hier`) shows the right value; the object model does not.
3. **Select on a struct member with its own range.** For a member declared
   `logic [23:16] dyn`, `s.dyn[22:16]` is bound against `[7:0]`, so it is out of range.
   svlang's netlist yields x; svcc now rejects the constant form, but a computed index on
   such a member is still wrong. Seen in OpenTitan `i3c_cmd_decode`.

Minimal reproductions:

```systemverilog
module g1 #(parameter WITH_CSR = 1) (input wire t, output wire [5:0] y);
  if (|WITH_CSR) begin : gen_csr assign y = t ? 6'b100010 : 6'b100001; end
  else           begin : gen_no  assign y = 6'd0; end
endmodule

module p1 #(parameter int W = 8, parameter logic [W-1:0] RV = '1) (output logic [7:0] a);
  assign a = RV;   // expected 8'hff
endmodule

package pk; typedef struct packed { logic [23:16] dyn; logic [7:0] lo; } e_t; endpackage
module m1 (input pk::e_t e, output logic [6:0] y);
  assign y = e.dyn[22:16];
endmodule
```

## What to do next, in order

1. **Fix the three svlang defects.** They silently change designs for every svlang consumer.
2. **Derived clocks.** Registers on a gated or divided clock run a delta late relative to the
   root clock. Plain copies are handled; a gating cell (latch plus AND) is not. This is the
   one known place where converted code can be wrong without a diagnostic.
3. **Unpacked structs and struct-typed ports through `parameter type`.** The largest real
   subset gap: most of the axi and cva6 rejections, once instantiated with real types.
4. **Hierarchical references and interfaces** (34 and 78 modules).
5. **Arrays local to a process and nested unpacked arrays** (28 modules), `foreach`, tasks.
6. **Array port connections** other than a whole array (about 140 modules), and **signals written by blocks with different triggers** (about 38, among them dual-clock RAM models).
7. **Conversion cost.** Three OpenTitan modules exceed 120 s (`pinmux`, `pinmux_reg_top`,
   `earlgrey_pd_aon`) and ten need more than 2 GB, mostly chip tops and modules whose default
   width is a placeholder.
8. The two unexplained cvfpu modules.

## Result tables (generated by `run_stress.py --report`)

svcc at the commit that adds this file, stage `cosim`, 3000 cosim steps. Every module of each design is tried as a top with its default parameters.

| Design | Modules | svlang rejects | svcc rejects (subset) | Crash / timeout | Converted | C++ compiles | Matches Verilator | Mismatch | Cosim not built |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| axi | 118 | 46 | 65 | 0 | 7 | 7 | 7 | 0 | 0 |
| basejump_stl | 314 | 13 | 273 | 2 | 26 | 26 | 25 | 0 | 1 |
| biriscv | 46 | 3 | 7 | 0 | 36 | 36 | 36 | 0 | 0 |
| caliptra-rtl | 401 | 36 | 108 | 0 | 257 | 257 | 257 | 0 | 0 |
| common_cells | 200 | 4 | 17 | 0 | 179 | 179 | 175 | 4 | 0 |
| cv32e40p | 123 | 3 | 19 | 3 | 98 | 98 | 90 | 7 | 1 |
| cv32e40x | 42 | 9 | 5 | 0 | 28 | 28 | 28 | 0 | 0 |
| cva6 | 84 | 26 | 51 | 0 | 7 | 7 | 7 | 0 | 0 |
| cvfpu | 12 | 0 | 2 | 0 | 10 | 10 | 6 | 2 | 2 |
| ibex | 30 | 7 | 3 | 0 | 20 | 20 | 19 | 0 | 1 |
| opentitan | 615 | 21 | 169 | 8 | 417 | 417 | 416 | 1 | 0 |
| picorv32 | 21 | 8 | 5 | 0 | 8 | 8 | 8 | 0 | 0 |
| scr1 | 38 | 1 | 8 | 0 | 29 | 29 | 29 | 0 | 0 |
| serv | 73 | 40 | 13 | 0 | 20 | 20 | 19 | 0 | 1 |
| tech_cells_generic | 12 | 0 | 2 | 0 | 10 | 10 | 10 | 0 | 0 |
| verilog-axi | 55 | 0 | 16 | 0 | 39 | 39 | 38 | 0 | 1 |
| **Total** | 2184 | 217 | 763 | 13 | 1191 | 1191 | 1170 | 14 | 7 |

## Rejected by svcc with a located error (subset gaps)

| Modules | Message | Example |
| ---: | :--- | :--- |
| 317 | port '…' has a type that is not a packed integral type (or an unpacked array of one) | common_cells/cc_clk_or_tree |
| 240 | cannot resolve hierarchical or member reference '…' | axi/axi_atop_filter |
| 214 | signal '…' has non-synthesizable type '…' | serv/serv_rf_ram |
| 188 | variable '…' has an unsupported type | picorv32/spiflash |
| 129 | array port '…' is left unconnected | common_cells/clk_or_tree |
| 62 | output port '…' is never driven | serv/serv_rf_if |
| 45 | signal '…' is read but never driven | serv/serv_ctrl |
| 34 | hierarchical reference to '…' is not supported | ibex/ibex_ex_block |
| 29 | a signal is driven by processes with different triggers (ff_field_storage, ff_field_storage_1, ff_field_storage_2, ff_field_storage_3, ff_field_storage_4, ff_fi | caliptra-rtl/aes_clp_reg |
| 28 | indexing of local or nested unpacked arrays is not supported | opentitan/aon_timer |
| 27 | '…' is not a signal that can be used here | picorv32/spiflash |
| 24 | constant select is outside the declared range (it would read x) | cvfpu/fpnew_opgroup_multifmt_slice |
| 23 | parameter '…' has no constant value here | axi/axi_lite_regs |
| 20 | whole unpacked arrays cannot be used in an expression | opentitan/aes |
| 18 | undeclared identifier '…' | ibex/ibex_tracer |
| 15 | array port '…' must be connected to a whole array of the same shape and element type | opentitan/edn |
| 12 | packed dimension requires constant bounds | opentitan/entropy_src |
| 9 | expression statement is not an assignment (task calls are not supported) | picorv32/picorv32 |
| 9 | part-select [N:N] is reversed: the range is declared [N:N] | verilog-axi/axi_adapter |
| 9 | array assignment needs a whole array of the same shape or an assignment pattern on the right | common_cells/cc_cb_filter |
| 9 | untyped assignment pattern requires an explicit type prefix or assignment context | opentitan/edn |
| 7 | inout or ref port '…' is not supported | picorv32/spiflash |
| 7 | zero replication count is only legal inside a non-empty concatenation | serv/serv_rf_ram |
| 7 | a signal is driven by processes with different triggers (ff_ram, ff_ram_1); this needs a single process in SystemC | biriscv/dcache |
| 7 | constant index is out of range | axi/axi_lite_lfsr |

## Rejected by svlang before lowering

| Modules | Message | Example |
| ---: | :--- | :--- |
| 78 | top-level module '…' has unconnected interface port '…' | axi/axi_atop_filter_intf |
| 64 | unknown module '…' | picorv32/hx8kdemo |
| 16 | use of undeclared identifier '…' | picorv32/hx8kdemo |
| 13 | indexed part-select width must be positive, not N'd0 | cva6/csr_regfile |
| 10 | :N:N: top module '…' is not defined | biriscv/RAM16X1D |
| 9 | DPI export '…' must be declared in the same scope | ibex/ibex_controller |
| 7 | unknown macro or directive `error | picorv32/hx8kdemo |
| 7 | unexpected `endmodule` | picorv32/hx8kdemo |
| 7 | unexpected `"…"`; expected an item | picorv32/hx8kdemo |
| 7 | redefinition of '…' | cva6/cva6 |
| 7 | no member named '…' in '…' | opentitan/bkdr_loader |
| 6 | cannot find include file "…" | serv/ice40_pll |
| 5 | port '…' does not exist in '…' | cva6/std_cache_subsystem |
| 5 | parameter '…' does not exist in '…' | cva6/std_cache_subsystem |
| 3 | indexed part-select width must be positive, not N | cva6/cva6_mmu |

## Crashes and timeouts

| Modules | Message | Example |
| ---: | :--- | :--- |
| 10 | note: run with `RUST_BACKTRACE=N` environment variable to display a backtrace | cv32e40p/stream_arbiter |
| 10 | memory allocation of N bytes failed | cv32e40p/stream_arbiter |
| 3 | (no message) | opentitan/pinmux |

## Mismatch against Verilator (14)

- `common_cells/cc_clk_int_div`: error: [svcc/cosim] 'cc_clk_int_div' differs from Verilator (work dir work/common_cells/cc_clk_int_div):
- `common_cells/cc_lfsr`: error: [svcc/cosim] 'cc_lfsr' differs from Verilator (work dir work/common_cells/cc_lfsr):
- `common_cells/clk_int_div`: error: [svcc/cosim] 'clk_int_div' differs from Verilator (work dir work/common_cells/clk_int_div):
- `common_cells/lfsr`: error: [svcc/cosim] 'lfsr' differs from Verilator (work dir work/common_cells/lfsr):
- `cvfpu/fpnew_cast_multi`: error: [svcc/cosim] 'fpnew_cast_multi' differs from Verilator (work dir work/cvfpu/fpnew_cast_multi):
- `cvfpu/fpnew_opgroup_block`: error: [svcc/cosim] 'fpnew_opgroup_block' differs from Verilator (work dir work/cvfpu/fpnew_opgroup_block):
- `cv32e40p/lfsr`: error: [svcc/cosim] 'lfsr' differs from Verilator (work dir work/cv32e40p/lfsr):
- `cv32e40p/popcount`: error: [svcc/cosim] 'popcount' differs from Verilator (work dir work/cv32e40p/popcount):
- `cv32e40p/stream_mux`: error: [svcc/cosim] 'stream_mux' differs from Verilator (work dir work/cv32e40p/stream_mux):
- `cv32e40p/generic_fifo`: error: [svcc/cosim] 'generic_fifo' differs from Verilator (work dir work/cv32e40p/generic_fifo):
- `cv32e40p/generic_fifo_adv`: error: [svcc/cosim] 'generic_fifo_adv' differs from Verilator (work dir work/cv32e40p/generic_fifo_adv):
- `cv32e40p/fpnew_cast_multi`: error: [svcc/cosim] 'fpnew_cast_multi' differs from Verilator (work dir work/cv32e40p/fpnew_cast_multi):
- `cv32e40p/fpnew_opgroup_block`: error: [svcc/cosim] 'fpnew_opgroup_block' differs from Verilator (work dir work/cv32e40p/fpnew_opgroup_block):
- `opentitan/prim_pad_attr`: error: [svcc/cosim] 'prim_pad_attr' differs from Verilator (work dir work/opentitan/prim_pad_attr):

## Simulation aborted (1)

- `ibex/ibex_register_file_latch`: error: [svcc/cosim] 'ibex_register_file_latch' differs from Verilator (work dir work/ibex/ibex_register_file_latch):

## Cosim executable not built (5)

- `serv/servant_ram`: error: [svcc/cosim] build failed in work/serv/servant_ram:
- `verilog-axi/axi_dma_wr`: error: [svcc/cosim] build failed in work/verilog-axi/axi_dma_wr:
- `cvfpu/fpnew_divsqrt_th_64_multi`: error: [svcc/cosim] build failed in work/cvfpu/fpnew_divsqrt_th_64_multi:
- `cvfpu/fpnew_fma_multi`: error: [svcc/cosim] build failed in work/cvfpu/fpnew_fma_multi:
- `cv32e40p/fpnew_fma_multi`: error: [svcc/cosim] build failed in work/cv32e40p/fpnew_fma_multi:

## Verilator rejects the source (1)

- `basejump_stl/bsg_rr_f2f_middle`: error: [svcc/cosim] build failed in work/basejump_stl/bsg_rr_f2f_middle:
