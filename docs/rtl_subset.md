# Synthesizable RTL conversion: accepted subset and mapping

`svcc rtl2sc` (and `svcc sv2sc` for files that hold RTL) converts SystemVerilog design code to
SystemC through a typed, elaborated representation (`svcc_ir::hdl`). This page states what is
accepted, how it is mapped, and how the result is checked.

There is no standard synthesizable subset of SystemVerilog. The subset below follows the
constructs Sutherland and Mills (DVCon 2014) report as supported by synthesis tools, and the
output stays inside the IEEE 1666.1 synthesizable subset of SystemC.

## How it works

1. `svlang` parses and elaborates the design. Parameters, `localparam`s and generate
   constructs are resolved; every instance has its own symbols.
2. Every expression is bound with `svlang`'s IEEE 1800-2023 11.6 / 11.8 binder. Each node
   carries its width and signedness, and every implicit conversion is an explicit node.
3. The lowering (`svcc-front-sv/src/hdl_lower.rs`) builds one module per specialization
   (definition plus parameter values).
4. The emitter (`svcc-emit-sc/src/hdl_emit.rs`) keeps one invariant: each emitted C++
   subexpression evaluates to exactly the value of its typed node. Wherever C++ would compute
   in a wider type, the result is truncated explicitly.

The lowering is fail-closed: a construct outside the subset is an error with a source
location. Constructs skipped on purpose are reported as notes.

## Mapping

| SystemVerilog | SystemC |
| :--- | :--- |
| `module` (one per parameter set) | `SC_MODULE` |
| `input` / `output` port | `sc_in<T>` / `sc_out<T>` |
| unpacked array port | `sc_vector<sc_in<T>>` / `sc_vector<sc_out<T>>` |
| variable, `wire` | `sc_signal<T>` |
| unpacked array of packed elements (memory) | `sc_vector<sc_signal<T>>` |
| 1-bit scalar | `bool` |
| packed vector, enum, packed struct, packed array, up to 64 bits | `sc_uint<W>` / `sc_int<W>` |
| the same, wider than 64 bits | `sc_biguint<W>` / `sc_bigint<W>` |
| `assign`, `always_comb`, `always @*`, `always_latch` | `SC_METHOD` sensitive to every signal read |
| `always_ff`, `always @(posedge ...)` | `SC_METHOD` sensitive to the same edges, `dont_initialize()` |
| nonblocking assignment | `signal.write(...)` |
| blocking assignment | local copy of the target, written back once at the end of the process |
| `function` (module, package) | `const` member function |
| module instance | member object, ports bound in the constructor |
| expression on a port, or a select on an output | glue signal plus a small `SC_METHOD` |

Supported statements: `if`, `case`, `casez`, `casex` (wildcards from constant items),
`case inside` (values, ranges, wildcard items), `for` and `while`, blocking and nonblocking assignment, compound assignment, `++` / `--`,
concatenation on the left-hand side, whole-array assignment (array copy or assignment
pattern), local variable declarations, `return`.

Supported expressions: all arithmetic, bitwise, reduction, logical, relational and shift
operators, `?:`, `inside` with constant items, concatenation and replication, bit, part and
indexed part selects, packed struct members, constant table lookups in unpacked parameters,
`$signed`, `$unsigned`, `$countones`, `$onehot`, `$onehot0`, and everything `svlang` folds to a
constant (`$clog2`, `$bits`, parameters, enum members).

### Points that need care

- **Edge-triggered blocks** are `SC_METHOD`s with the source block's edge list, and the body
  is translated statement by statement. Reset shape, synchronous or asynchronous, is whatever
  the source wrote; no reset pattern is assumed.
- **Blocking read-back.** A signal assigned with `=` and read later in the same block is read
  from the local copy, so the later statement sees the new value as in SystemVerilog.
- **Several blocks driving one signal.** A SystemC signal has one writer. Blocks (or instance
  outputs) that drive different bits of the same signal are merged into one process. Blocks
  with different triggers that share a signal are an error.
- **Edges on expressions and internal signals.** An edge on a struct member, a vector bit or
  any expression triggers on a 1-bit helper signal driven with that expression. This is
  accepted for asynchronous set and reset edges only (see the next point).
- **Clock sources.** A register may be clocked by an input port, by a plain copy of one
  (`assign gclk = clk;`, a buffer instance), or by a register output written with nonblocking
  assignments (a divider). Any other clock is an error: one produced by combinational logic
  (a clock-gating cell, `clk & en`), by an instance output that is not a register output, by a
  blocking assignment in an edge-triggered block, or by an edge on an expression. A signal
  write takes one delta cycle, so registers on such a clock would run after the registers on
  the source clock had been updated and would sample the new values, where SystemVerilog
  samples the old ones. The check follows clocks through instance ports.
- **Arrays with several writers.** Each element is its own signal. A process writes back only
  the elements it assigns; elements assigned through a computed index make the whole array
  belong to one process.
- **No driver.** A signal that is read, or an output port, with no driver at all is an error.
  In a two-state model it would silently read 0.
- **Two-state.** `x` and `z` map to `0`. Reads outside a vector or array give `0`; writes
  outside are dropped.
- **Division and modulo by zero** give `0`.
- **Names** that collide with C++ keywords or `sc_module` members get a `_` suffix; the
  SystemC object name keeps the source spelling. Generate scopes become name prefixes.

## Not supported (reported as errors)

`inout` and `ref` ports, interfaces and modports, tri-state and net types other than `wire`,
unpacked structs and unions, arrays local to a process, array ports connected to anything but
a whole array, `foreach`, `repeat`, `forever`, `do while`, tasks, functions with `output` arguments,
recursive functions, hierarchical references, `**` with a non-constant operand, gate
primitives and UDPs, initializers that are not constant.

Skipped with a note: `initial` and `final` blocks, assertions, clocking blocks, system task
calls.

Known limits of the current mapping:

- Variables declared inside a process are created anew on each activation. A static local
  that is read before it is written is not preserved across activations.
- A blocking assignment becomes visible to *other* processes one delta cycle later, like any
  signal write. A clock generated by a blocking assignment in an edge-triggered block is
  rejected (see Clock sources). A reset generated that way and read by another block is
  converted and takes effect one delta later. Generators of this kind were seen in
  `cc_clk_int_div` and `scr1_scu`.
- An asynchronous set or reset produced by combinational logic takes effect one delta cycle
  later than in SystemVerilog. The register settles to the same value within the time step.
- A signal written by two edge-triggered blocks with different clocks (a dual-clock memory)
  is rejected.
- Parameters are resolved to constants. The output is not a C++ template.
- A file with several top modules is converted as several tops; multi-file `-f` builds are
  not wired into the command line yet.

## How the result is checked

`svcc rtl-cosim <file.sv> --top <module>` converts the design, builds the generated SystemC
together with a Verilator model of the same source in one executable, drives both with the
same random stimulus and compares every output after every step. Data inputs and edge events
change in separate steps, so the comparison does not depend on how a simulator orders a clock
edge against a simultaneous data change. Both models run two-state
(`--x-assign 0 --x-initial 0`).

`scripts/rtl_cosim.sh` runs this over `testdata/rtl` (one design per construct group) and is
step 7 of `scripts/check.sh`. It needs `verilator` on `PATH` and `SYSTEMC_HOME`.

Inputs that reach an edge sensitivity, directly or through gating logic and instance
boundaries, are driven as clocks: one of them changes per step, alone. Asynchronous controls
start inactive, are asserted at step 1 and released at step 3. Verilator runs with
`--no-assert`, because the design's own assertions about its environment do not hold under
random stimulus, and with `-UVERILATOR`: Verilator defines the `VERILATOR` macro and svcc
does not, so without it the two models are built from different text wherever the source
tests it.

Two known Verilator deviations affect the comparison:

- A module that instantiates itself (a recursive tree) is built without its children when it
  is the top level (Verilator 5.052 and 5.055). rtl-cosim then gives the reference a wrapper
  top, so the recursive module is an ordinary instance (`testdata/rtl/recursive_top.sv`).
- A write through a computed index outside the declared range is not ignored, as IEEE
  1800-2023 11.5.1 requires; Verilator wraps the index into the range. svcc ignores the
  write. A difference on such a design is in the reference, not in svcc.

`benchmarks/rtl_stress/run_stress.py` runs the same three stages (convert, compile against
SystemC, simulate against Verilator) over every module of a corpus of open-source designs
kept outside this repository; `benchmarks/results/rtl_stress.md` holds the last result.

This is simulation evidence over random stimulus, not a proof.

## Formal check

`svcc rtl-prove <file.sv>` reads the generated header back with C++ evaluation rules and
proves it equal to the source text; `scripts/rtl_prove.sh` runs it over `testdata/rtl`, the
probes of `benchmarks/scav_compare` and corpus designs 13..20, and is step 7 of
`scripts/check.sh`. `roundtrip-check` runs the same proof for the design modules of a file.
`docs/formal_gates.md` says what a pass proves, what it assumes (the two-state rules above,
one clock, a synchronous view of asynchronous reset) and what remains simulation evidence.
The text-based RTL path that these checks used before has been removed.
