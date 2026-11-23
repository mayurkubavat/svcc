# Typed RTL path on real designs

svcc `c77d547`, stage `prove-from`. Every module of each design is tried as a top with its default parameters.


## Proof of the generated C++ (`svcc rtl-prove`) on modules that match Verilator

Modules taken from the co-simulation results in `rtl_stress_m52_cosim.jsonl`.

| Design | Modules | Proved (induction) | Bounded only | Fail | Not covered | Error | Timeout |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| axi | 7 | 2 | 0 | 1 | 4 | 0 | 0 |
| basejump_stl | 25 | 19 | 1 | 4 | 1 | 0 | 0 |
| biriscv | 36 | 28 | 3 | 5 | 0 | 0 | 0 |
| caliptra-rtl | 260 | 146 | 34 | 8 | 72 | 0 | 0 |
| common_cells | 175 | 93 | 42 | 2 | 38 | 0 | 0 |
| cv32e40p | 94 | 49 | 5 | 6 | 34 | 0 | 0 |
| cv32e40x | 30 | 15 | 0 | 1 | 14 | 0 | 0 |
| cva6 | 7 | 3 | 0 | 0 | 4 | 0 | 0 |
| cvfpu | 7 | 3 | 0 | 2 | 2 | 0 | 0 |
| ibex | 20 | 6 | 3 | 0 | 11 | 0 | 0 |
| opentitan | 417 | 182 | 31 | 4 | 200 | 0 | 0 |
| picorv32 | 8 | 6 | 0 | 0 | 2 | 0 | 0 |
| scr1 | 29 | 16 | 2 | 4 | 7 | 0 | 0 |
| serv | 25 | 13 | 2 | 4 | 6 | 0 | 0 |
| tech_cells_generic | 10 | 7 | 0 | 1 | 2 | 0 | 0 |
| verilog-axi | 38 | 12 | 6 | 3 | 17 | 0 | 0 |
| **Total** | 1188 | 600 | 129 | 45 | 414 | 0 | 0 |

Of 600 proved modules, 314 have no registers to pair; the largest has 1026 paired registers. Median Z3 time for the generated-C++ side 28 ms, maximum 44137 ms.


### Not covered by the proof

| Modules | Reason | Example |
| ---: | :--- | :--- |
| 58 | btor2 write failed: Rejected("more than one clock (...); every register steps on each model step") | verilog-axi/axil_cdc |
| 35 | svlang netlist lowering failed: prim_onehot_check.sv: unsupported: immediate assertion on a non-constant value inside initial block is not supported | opentitan/adc_ctrl_reg_top |
| 26 | module 'X': [] method 'X': signal 'X' is used apart from its local copy 'X' | picorv32/picorv32_pcpi_fast_mul |
| 19 | svlang netlist lowering failed: prim_fifo_sync.sv: unsupported: $isunknown requires allow_four_state_ops <corpus>/opentit | opentitan/cheriot_access_check |
| 15 | svlang netlist lowering failed: prim_count.sv: unsupported: immediate assertion on a non-constant value inside initial block is not supported | opentitan/csrng_state_db |
| 13 | module 'X': [] method 'X': [svcc-front-sc/cxx] local 'X' copies signal 'X' but is not written back at the end of the method | verilog-axi/arbiter |
| 11 | svlang netlist lowering failed: prim_clock_mux2.sv: unsupported: $isunknown requires allow_four_state_ops ~/Silicon/DV/corpus/rtl-censu | opentitan/i3c_clock_buf_en |
| 10 | module 'X': [] method 'X': [svcc-front-sc/cxx] loop header that is not plain assignments to locals (or may be undefined) | common_cells/cc_majority_vote_filter |
| 10 | svlang netlist lowering failed: prim_util_memload.svh: unsupported: unsupported expression statement or task call inside initial block ~/Silicon | opentitan/i2c_core |
| 8 | module 'X': [] method 'X': [svcc-front-sc/cxx] sc_big shift by an amount that may be negative | scr1/scr1_pipe_mprf |
| 7 | svlang netlist lowering failed: caliptra_prim_count.sv: unsupported: immediate assertion on a non-constant value inside initial block is no | caliptra-rtl/caliptra_prim_count |
| 5 | svlang netlist lowering failed: cc_addr_decode_dync.sv: unsupported: $isunknown requires allow_four_state_ops | common_cells/cc_addr_decode |
| 4 | btor2 write failed: Rejected("more than one clock (...), and the clock of el2_exu_div_new_ | caliptra-rtl/el2_exu_div_new_1bit_fullshortq |
| 4 | svlang netlist lowering failed: prim_arbiter_fixed.sv: unsupported: $isunknown requires allow_four_state_ops <corpus>/ope | opentitan/i3c_targ_async_events |
| 4 | svlang netlist lowering failed: prim_mubi4_sync.sv: unsupported: $isunknown requires allow_four_state_ops | opentitan/prim_mubi4_sync |
| 3 | btor2 write failed: Rejected("more than one clock (...); every register steps | common_cells/edge_propagator |
| 3 | svlang netlist lowering failed: prim_arbiter_ppc.sv: unsupported: $isunknown requires allow_four_state_ops <corpus>/opent | opentitan/prim_arbiter_ppc |
| 3 | svlang netlist lowering failed: prim_arbiter_tree.sv: unsupported: $isunknown requires allow_four_state_ops <corpus>/open | opentitan/prim_arbiter_tree |
| 2 | svlang netlist lowering failed: tc_sram.sv: unsupported: unsupported expression statement or task call inside initial block ~/Silicon/DV/cor | tech_cells_generic/tc_sram |
| 2 | svlang netlist lowering failed: cc_multiaddr_decode.sv: unsupported: $isunknown requires allow_four_state_ops | common_cells/cc_multiaddr_decode |
| 2 | btor2 write failed: Rejected("more than one clock (...); every register steps on e | common_cells/cdc_2phase_clearable |
| 2 | svlang netlist lowering failed: caliptra_prim_onehot_check.sv: unsupported: immediate assertion on a non-constant value inside initial bloc | caliptra-rtl/caliptra_prim_onehot_check |
| 2 | svlang netlist lowering failed: caliptra_prim_util_memload.svh: unsupported: unsupported expression statement or task call inside initial blo | caliptra-rtl/caliptra_prim_ram_1p_adv |
| 2 | btor2 write failed: Rejected("more than one clock (...), and the clock of el2_ | caliptra-rtl/el2_exu_div_existing_1bit_cheapshortq |
| 2 | svlang netlist lowering failed: tlul_adapter_sram.sv: unsupported: $isunknown requires allow_four_state_ops <corpus>/open | opentitan/kmac_staterdwr |

### Why the failed proofs failed

| Modules | Cause |
| ---: | :--- |
| 27 | solver gave no answer (Z3 time limit) |
| 9 | construct not supported on the proof path |
| 6 | module has no outputs to compare |
| 3 | start values of paired states differ (no solver run) |

### Proof failed (difference or no answer) (45)

- `serv/serv_debug`: lowering ?, generated C++ ?. error: [svcc-equiv/pair] no outputs to compare
- `serv/servant_ram`: lowering Inconclusive, generated C++ unchecked. [FAIL] lowering      'servant_ram': solver gave no answer (2 outputs, 2 paired states, z3 120220191 us)
- `serv/servant_ram`: lowering Inconclusive, generated C++ unchecked. [FAIL] lowering      'servant_ram': solver gave no answer (2 outputs, 2 paired states, z3 120210456 us)
- `serv/serving_ram`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'serving_ram': solver gave no answer (1 outputs, 1 paired states, z3 120154176 us)
- `biriscv/biriscv_frontend`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'biriscv_frontend': solver gave no answer (31 outputs, 13 paired states, z3 122100312 us)
- `biriscv/biriscv_issue`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'biriscv_issue': solver gave no answer (69 outputs, 66 paired states, z3 120300143 us)
- `biriscv/biriscv_npc`: lowering BoundedNoDifference, generated C++ Inconclusive. [BND ] lowering      'biriscv_npc': no difference in 8 steps from reset state (bounded) (2 outputs, 1 paired states, z3 2015685 us)
- `biriscv/biriscv_trace_sim`: lowering ?, generated C++ ?. error: [svcc-equiv/pair] no outputs to compare
- `biriscv/riscv_core`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'riscv_core': solver gave no answer (13 outputs, 169 paired states, z3 12094825 us)
- `verilog-axi/axi_interconnect`: lowering Mismatch, generated C++ unchecked. [FAIL] lowering      'axi_interconnect': DIFFERENCE FOUND (44 outputs, 54 paired states, z3 0 us)
- `verilog-axi/axil_crossbar_addr`: lowering Mismatch, generated C++ unchecked. [FAIL] lowering      'axil_crossbar_addr': DIFFERENCE FOUND (9 outputs, 9 paired states, z3 0 us)
- `verilog-axi/axil_interconnect`: lowering Mismatch, generated C++ unchecked. [FAIL] lowering      'axil_interconnect': DIFFERENCE FOUND (19 outputs, 24 paired states, z3 0 us)
- `scr1/scr1_clk_ctrl`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'scr1_cg': combinational process 'comb_latch_en' uses a nonblocking assignment
- `scr1/scr1_pipe_ialu`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'scr1_pipe_ialu': proved for all runs from equal start states (4 outputs, 6 paired states, z3 28919 us)
- `scr1/scr1_pipe_ifu`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'scr1_pipe_ifu': solver gave no answer (9 outputs, 16 paired states, z3 120155872 us)
- `scr1/scr1_cg`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'scr1_cg': combinational process 'comb_latch_en' uses a nonblocking assignment
- `tech_cells_generic/tc_clk_gating`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'tc_clk_gating': combinational process 'comb_clk_en' uses a nonblocking assignment
- `common_cells/cc_unread`: lowering ?, generated C++ ?. error: [svcc-equiv/pair] no outputs to compare
- `common_cells/unread`: lowering ?, generated C++ ?. error: [svcc-equiv/pair] no outputs to compare
- `axi/axi_dumper`: lowering ?, generated C++ ?. error: [svcc-equiv/pair] no outputs to compare
- `cvfpu/fpnew_fma`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'fpnew_fma': solver gave no answer (10 outputs, 0 paired states, z3 240270458 us)
- `cvfpu/fpnew_opgroup_fmt_slice`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'fpnew_opgroup_fmt_slice': proved for all runs from equal start states (8 outputs, 0 paired states, z3 118920 us)
- `cv32e40p/cv32e40p_ex_stage`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'cv32e40p_ex_stage': proved for all runs from equal start states (26 outputs, 12 paired states, z3 87615 us)
- `cv32e40p/cv32e40p_mult`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'cv32e40p_mult': proved for all runs from equal start states (4 outputs, 2 paired states, z3 33223 us)
- `cv32e40p/cv32e40p_sleep_unit`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'cv32e40p_clock_gate': combinational process 'comb_clk_en' uses a nonblocking assignment
- `cv32e40p/unread`: lowering ?, generated C++ ?. error: [svcc-equiv/pair] no outputs to compare
- `cv32e40p/fpnew_fma`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'fpnew_fma': proved for all runs from equal start states (9 outputs, 0 paired states, z3 291809 us)
- `cv32e40p/fpnew_opgroup_fmt_slice`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'fpnew_opgroup_fmt_slice': solver gave no answer (7 outputs, 0 paired states, z3 240560393 us)
- `cv32e40x/cv32e40x_sleep_unit`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'cv32e40x_clock_gate': combinational process 'comb_clk_en' uses a nonblocking assignment
- `caliptra-rtl/aes_masked_inverse_gf2p8_noreuse`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'aes_masked_inverse_gf2p8_noreuse': proved for all runs from equal start states (1 outputs, 0 paired states, z3 37721 us)
- `caliptra-rtl/aes_dom_inverse_gf2p8`: lowering BoundedNoDifference, generated C++ Inconclusive. [BND ] lowering      'aes_dom_inverse_gf2p8': no difference in 8 steps from reset state (bounded) (3 outputs, 0 paired states, z3 115188 us)
- `caliptra-rtl/caliptra_tlul_socket_1n`: lowering Inconclusive, generated C++ unchecked. [FAIL] lowering      'caliptra_tlul_socket_1n': solver gave no answer (5 outputs, 2 paired states, z3 120579786 us)
- `caliptra-rtl/csrng_ctr_drbg_cmd`: lowering BoundedNoDifference, generated C++ Inconclusive. [BND ] lowering      'csrng_ctr_drbg_cmd': no difference in 8 steps from reset state (bounded) (21 outputs, 1 paired states, z3 65169 us)
- `caliptra-rtl/ecc_arith_unit`: lowering Inconclusive, generated C++ unchecked. [FAIL] lowering      'ecc_arith_unit': solver gave no answer (2 outputs, 149 paired states, z3 121617145 us)
- `caliptra-rtl/ecc_scalar_blinding`: lowering Inconclusive, generated C++ Inconclusive. [FAIL] lowering      'ecc_scalar_blinding': solver gave no answer (2 outputs, 28 paired states, z3 240223286 us)
- `caliptra-rtl/caliptra_sram`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'caliptra_sram': proved for all runs from equal start states (1 outputs, 257 paired states, z3 82007 us)
- `caliptra-rtl/mbox_csr`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'mbox_csr': process 'comb_field_combo_15': local 'next_c' is declared with different types in nested blocks
- `opentitan/aes_masked_inverse_gf2p8`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'aes_masked_inverse_gf2p8': proved for all runs from equal start states (1 outputs, 0 paired states, z3 34818 us)
- `opentitan/aes_masked_inverse_gf2p8_noreuse`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'aes_masked_inverse_gf2p8_noreuse': proved for all runs from equal start states (1 outputs, 0 paired states, z3 39141 us)
- `opentitan/aes_dom_inverse_gf2p8`: lowering BoundedNoDifference, generated C++ Inconclusive. [BND ] lowering      'aes_dom_inverse_gf2p8': no difference in 8 steps from reset state (bounded) (3 outputs, 14 paired states, z3 113864 us)
- `opentitan/alert_handler_class`: lowering Inconclusive, generated C++ ProvedInductive. [FAIL] lowering      'alert_handler_class': solver gave no answer (3 outputs, 0 paired states, z3 123752272 us)
- `basejump_stl/bsg_clkgate_optional`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'bsg_dlatch': combinational process 'comb_data_r' uses a nonblocking assignment
- `basejump_stl/bsg_icg_neg`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'bsg_icg_neg': combinational process 'comb_clk_en_r' uses a nonblocking assignment
- `basejump_stl/bsg_icg_pos`: lowering ?, generated C++ ?. error: [svcc-emit-sv/hdl] module 'bsg_icg_pos': combinational process 'comb_clk_en_r' uses a nonblocking assignment
- `basejump_stl/bsg_mul_32_32`: lowering ProvedInductive, generated C++ Inconclusive. [PASS] lowering      'bsg_mul_32_32': proved for all runs from equal start states (1 outputs, 0 paired states, z3 306861 us)

### Bounded only (129)

- `serv/serv_immdec`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `serv/serv_state`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `biriscv/biriscv_decode`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `biriscv/biriscv_mmu`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `biriscv/biriscv_regfile`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `verilog-axi/axi_register`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `verilog-axi/axi_register_rd`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `verilog-axi/axi_register_wr`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `verilog-axi/axil_register`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `verilog-axi/axil_register_rd`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `verilog-axi/axil_register_wr`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `scr1/scr1_pipe_hdu`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `scr1/scr1_pipe_lsu`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_4phase_dst`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_src`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_dst`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_src_ptr`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_dst_ptr`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_dst_data`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_src_clearable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_dst_clearable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_src_ptr_clearable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_dst_ptr_clearable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_cdc_fifo_gray_dst_data_clearable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_max_counter`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_mem_to_banks`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_mem_to_banks_detailed`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_ring_buffer`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_shift_register`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_spill_register`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_spill_register_flushable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_stream_delay`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_stream_fifo_optimal_wrap`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_stream_fork`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_stream_fork_dynamic`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_stream_omega_net`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_stream_to_mem`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cc_stream_xbar`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cdc_4phase_dst`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cdc_fifo_gray_src`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cdc_fifo_gray_dst`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cdc_fifo_gray_src_clearable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/cdc_fifo_gray_dst_clearable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/max_counter`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/mem_to_banks`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/mem_to_banks_detailed`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/ring_buffer`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/shift_reg`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/spill_register`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/spill_register_flushable`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/stream_delay`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/stream_fifo_optimal_wrap`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/stream_fork`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/stream_fork_dynamic`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `common_cells/stream_to_mem`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `ibex/ibex_fetch_fifo`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `ibex/ibex_prefetch_buffer`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `ibex/ibex_register_file_ff`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `cv32e40p/cdc_fifo_gray_dst`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
- `cv32e40p/spill_register`: lowering BoundedNoDifference, generated C++ BoundedNoDifference. 
