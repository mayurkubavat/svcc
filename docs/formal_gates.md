# Formal gates of the RTL path

This page says what each check of synthesizable design code proves, what it assumes, and
what is still only simulation evidence. It is the reference for any claim that `svcc`
"proves" an RTL conversion.

## Why the gates were rebuilt

Until November 2026 the RTL part of `roundtrip-check`, and the C2RTL proof `prove-sec`, ran
on a text-based RTL path. That path carried expressions as strings: the C++ it emitted was
read back *as if it were SystemVerilog* and compared with the source by a solver. A check
built that way cannot see a C++ width error, because it never asks what the C++ computes.
It reported `EquivalentUnsat` for two designs whose generated C++ was wrong:

| Source | What the C++ did | First difference |
| :--- | :--- | :--- |
| `assign y = (a + b) >> 1;` (8-bit) | shifted the 64-bit sum | `a = b = 0xff`: SystemVerilog `0x7f`, C++ `0xff` |
| `assign lt = (a - b) < c;` (8-bit) | compared the 64-bit difference | `a = 0x3e, b = 0xc1, c = 0x7e` |

The text-based path is gone (see "What was removed"). Every gate below runs on the typed
path, and the first two rows are regression tests that must fail the gate
(`crates/svcc/tests/formal_gates.rs`).

## The gates

| Gate | Command | Compares | Method |
| :--- | :--- | :--- | :--- |
| Generated SystemC against its source | `svcc rtl-prove <in.sv>` (also the RTL part of `roundtrip-check`) | the header `rtl2sc` writes, read back with C++ rules, against the source text | induction over paired state, else bounded |
| Lowering | same command, first line of output | the typed IR, written as explicit-width SystemVerilog, against the source text | same |
| C2RTL | `svcc prove-sec <golden.hpp> <rtl.sv> --latency L` | a `// svcc:c2rtl` C++ function, read with C++ rules, against RTL, `L` cycles later | from an arbitrary state of the RTL, else bounded |
| SV against SV | `svcc prove-sec <a.sv> <b.sv>` | two SystemVerilog designs, outputs matched by name | induction over paired state, else bounded |
| Differential simulation | `svcc rtl-cosim <in.sv>` | generated SystemC, compiled against real SystemC, against Verilator | random stimulus. **Not a proof.** |

`scripts/rtl_prove.sh` runs the first gate over `testdata/rtl` (24 designs),
`benchmarks/scav_compare/probes` (18) and corpus designs 13..20 (8); it is step 7 of
`scripts/check.sh` and needs only `z3`.

### How the generated C++ is examined

```text
            source.sv ──────────────────────────────► svlang netlist ─► Btor2 ─┐
                │                                                              ├─► product, Z3
                └─► typed IR ─► generated.hpp ─► C++ reader ─► explicit-width  │
                    (rtl2sc)                     (hdl_lift)    SV ─► svlang ───┘
```

The reference side is always the source text, lowered by the `svlang` netlist on its own.
Nothing on that side passes through `svcc`'s IR, so a wrong lowering cannot hide in both
sides at once.

The other side starts from the bytes of the generated header.
`svcc_front_sc::hdl_lift` parses them and `svcc_front_sc::cxx_sem` assigns each expression
the value C++ gives it:

- `bool` and the 8- and 16-bit integer types promote to `int`; an `sc_uint<W>` operand
  converts to `unsigned long long`, an `sc_int<W>` operand to `long long` (LP64);
- two operands are brought to a common type by the usual arithmetic conversions and the
  operation is carried out in that type, so `a + b` on two `sc_uint<8>` values is a 64-bit
  addition;
- a value is truncated only where the C++ constructs or assigns a narrower object;
- `sc_biguint` / `sc_bigint` arithmetic is exact, as in SystemC;
- `.write` in an edge-triggered method is a nonblocking assignment; in a level-sensitive
  method it stands for a blocking assignment only if the method never reads that signal,
  and a method that does is rejected;
- a level-sensitive method must be sensitive to every signal it reads, directly or through
  the functions it calls.

The result is written as SystemVerilog in which every operator result is self-determined
and every width change is explicit (`svcc_emit_sv::hdl_emit`), so that SystemVerilog's own
context sizing cannot change what was read from the C++. That text is lowered by `svlang`
like the source, and `svcc_equiv::pair` compares the two Btor2 systems.

### Undefined behaviour in the C++

An operation whose result C++ or SystemC does not define is not given a value. Each one
raises a 1-bit flag (`svcc__ub`, an extra output of every lifted module), and a proof must
show the flag stays 0: signed overflow, division or modulo by zero, a shift by a negative
amount or by the width or more, a left shift of a negative value, a bit or range select
outside the vector, an array index outside the array. Conditions an interval analysis
proves false are dropped; the rest go to the solver under the `?:`, `&&`, `||` and `if`
that guard them. So a guard the generator forgot is found as a counterexample, not assumed.

Behaviour the standard leaves to the implementation is fixed as every supported compiler
has it and as C++20 defines it: conversion to a narrower signed type wraps, `>>` of a
negative value is an arithmetic shift.

This is how the C2RTL gate found a real defect in a benchmark golden model:
`uint32_t prod = a * b;` with `uint16_t` operands multiplies in `int` and overflows for
large operands. The golden model now reads `uint32_t(a) * b`.

## What a verdict means

| Verdict | Meaning |
| :--- | :--- |
| `ProvedInductive` | Both sides have the same registers (paired by name, same width). From any state in which the paired registers are equal, the outputs agree and the paired registers are equal again after a step. So the two designs agree in every step of every run that starts from equal states. No bound. For C2RTL the same verdict means: from an arbitrary state of the RTL, its output equals the golden function of the inputs `L` cycles earlier. |
| `BoundedNoDifference` | No difference within `K` steps (default 8) from the initial state, where a register without a start value starts at 0. Not a full proof: the output says "(bounded)" and `rtl-prove` and `prove-sec` exit with code 2 (`--accept-bounded` makes it 0). |
| `Mismatch` | A concrete input sequence is printed for which the outputs differ or the undefined-behaviour flag is 1. |
| `Inconclusive` | The solver gave no answer. Counts as a failure. |
| NOT COVERED | The design, or its generated header, is outside what the proof models: more than one clock or a computed clock the Btor2 model rejects, a multi-dimensional array port, a C++ form the reader does not model. Neither proved nor refuted; `rtl-prove` exits with code 3 (0 proved, 1 failed, 2 bounded only) and simulation is the only evidence. `scripts/rtl_prove.sh` lists the designs of the repository expected to be in this state and fails if another one joins them. |

## What the proof assumes

A pass is a statement inside this model, and each line is a place where the model, not the
design, could be wrong:

1. **Two-state reference.** An X or Z bit in the source reads as 0, and division or modulo
   by zero yields 0. This is the model `docs/rtl_subset.md` defines for the conversion and
   the one the co-simulation uses. A design that relies on X is outside it.
2. **`svlang` is the meaning of SystemVerilog.** Both sides go through the pinned `svlang`
   netlist and Btor2 writer. A bug there that affects the source and the explicit-width
   text alike is not visible. The explicit-width text uses only concatenation, size casts,
   `$signed` and plain operators, to keep that surface small.
3. **One clock, synchronous view of asynchronous reset.** The Btor2 model has one clock
   (both edges are accepted) and treats an asynchronous reset as taking effect at the clock
   step. Reset is an ordinary free input, so reset behaviour is covered, but the timing of
   an asynchronous edge between clock edges is not. Only simulation covers that.
4. **`svcc`'s reading of C++.** The C++ reader is a model written for this purpose, not a
   compiler front end. Its rules for SystemC types were checked against SystemC 3.0.3 with
   `benchmarks/formal_gate/systemc_semantics_probe.cpp`, and the mutation check below
   compares its verdicts with real compiled SystemC. It reads the subset `rtl2sc` writes;
   hand-written C++ outside that subset is rejected.
5. **Equal starting states.** `ProvedInductive` is about runs that start from equal
   register values. Declared start values are compared; a register with none is X in
   SystemVerilog and 0 in SystemC, which rule 1 makes equal.
6. **Process scheduling.** A level-sensitive `SC_METHOD` is read as combinational logic
   (or a latch where it does not assign on every path), given the sensitivity and
   read/write checks above. Delta-cycle ordering between processes is not modelled beyond
   that.
7. **C2RTL holds resets inactive.** With `--latency L` every asynchronous reset of the RTL
   is held at its inactive level and other inputs are free; `--hold name=value` holds a
   synchronous one. The RTL is not checked under reset by this gate.

## Evidence for the gate itself

- At main 00c80f0 (v0.1.1), `scripts/rtl_prove.sh` covers 50 designs: 45 are proved by
  induction for both the lowering and the generated C++, none is only bounded, and 5 are
  NOT COVERED (more than one clock, or a form the reader does not model). Measured on Linux
  x86_64 with Z3 5.4.1; journal entry M6.1-rtl-prove-main.
- At an earlier commit, the 42 designs then in the set passed `rtl-cosim` against SystemC
  3.0.3 and Verilator (macOS arm64). `rtl-cosim` has not been re-run on all 50 at 00c80f0.
- 8 of 8 golden models pass `prove-sec` with `ProvedInductive`.
- Negative tests in `crates/svcc/tests/formal_gates.rs`: the two historical width bugs, a
  dropped sign conversion, an unguarded division, a missing sensitivity entry, a wrong
  latency, a datapath bug in the RTL, a golden model with promoted-multiplication overflow,
  and C++ outside the subset. Each must fail.
- Mutation check, `benchmarks/formal_gate/mutate.py`: see the next section.

## Mutation check

`benchmarks/formal_gate/mutate.py` edits the generated header one place at a time, with
edits that change the width or signedness at which C++ evaluates an expression, and gives
each mutant to two judges that share no code: the proof (`rtl-prove --header`) and real
compiled SystemC against Verilator (`rtl-cosim --header`, 5000 steps). The outcome to look
for is a mutant the simulation shows to be wrong and the proof accepts.

Run of 2026-11-22 on the 42 designs (macOS arm64, SystemC 3.0.3, Verilator 5.052), results
in `benchmarks/results/formal_gate_mutation_*.json`:

| Edit | Mutants that compile | Simulation differs | of those: proof finds a difference | of those: reader rejects | of those: proof accepts |
| :--- | ---: | ---: | ---: | ---: | ---: |
| remove a truncating construction, `sc_uint<8>(e)` to `(e)` | 133 | 7 | 4 | 3 | 0 |
| construct one bit wider, `sc_uint<8>(e)` to `sc_uint<9>(e)` | 106 | 5 | 5 | 0 | 0 |
| flip signedness, `sc_uint<8>(e)` to `sc_int<8>(e)` | 28 | 5 | 5 | 0 | 0 |
| remove a built-in cast, `uint64_t(e)` to `(e)` | 14 | 0 | 0 | 0 | 0 |
| **Total** | **281** | **17** | **14** | **3** | **0** |

How to read it, and what it does not show:

- No mutant that the simulation shows to be wrong was accepted by the proof. All 17 were
  caught, 14 with a counterexample and 3 because the reader refused the text.
- The other 264 mutants simulate equal, and the proof accepted 255 of them and the reader
  refused 9. The proof never reported a difference the simulation did not see.
- Only 17 of 281 mutants change behaviour: most edits are harmless because an outer
  construction truncates again. This is a small sample, on small designs, with four kinds
  of edit. It is evidence that the C++ model agrees with real SystemC where it was
  exercised, not a validation of the model.
- 70 further mutants did not compile and were dropped. The reader accepted 56 of those:
  it is not a C++ type checker and can read text a compiler rejects (for example an
  assignment between `sc_int` and `sc_uint` that C++ finds ambiguous). A header has to
  compile for a proof about it to mean anything; `rtl-cosim` checks that.

## Coverage after the stress-run changes

The stress run on real designs added emitter forms and four corpus designs. Measured on the
branch that integrated them, before the clock-gating work (46 designs: 20 in `testdata/rtl`,
18 probes, 8 benchmark designs). The current totals are in the previous section.

| Outcome | Designs |
| :--- | :--- |
| Proved by induction, lowering and generated C++ | 43 |
| NOT COVERED: more than one clock or a computed clock | `array_elems`, `edges_misc` |
| NOT COVERED: multi-dimensional array port | `array_ports` |

How the reader treats each new form:

| Emitted form | Result |
| :--- | :--- |
| `sig.posedge_event()` / `negedge_event()` on an internal signal | read as an edge; proved when the Btor2 model takes the clocking, NOT COVERED when it reports several clocks. Since v0.1.1, `rtl2sc` converts a clock-gating cell, clock buffer or inverter into a clock enable on the root clock; a clock made by other logic is still rejected, because the SystemC would run it one delta cycle late. See `docs/rtl_subset.md`, Clock sources. The proof does not model delta-cycle scheduling, so a hand-written header that keeps a derived clock can pass it; use `rtl-cosim` for scheduling |
| `sc_vector<sc_in<T>>` / `sc_vector<sc_out<T>>`, one dimension | proved, one output per element |
| array ports with two or more dimensions | NOT COVERED |
| lambda for replication of more than 16 copies into a vector over 64 bits | NOT COVERED (the lowering is still proved) |
| per-bit loop for a dynamic range write, shift-based out-of-range read | proved |
| module class name with a trailing `_` | proved |

The designs of the stress corpus itself live outside this repository and have not yet been
put to the proof. Since 0.1.3 `rtl-prove` takes the same multi-file arguments as `rtl2sc`
(more files, `-f`, `+incdir+`, `+define+`, `-G`); the reference netlist is built from those
arguments by `svlang` alone.

## What is still only simulation evidence

- Anything about real compiled SystemC: that the header compiles, that SystemC's kernel
  schedules it as assumed in rule 6, that the data types behave as the reader models them.
  `rtl-cosim` and `scripts/rtl_cosim.sh` (step 8 of `scripts/check.sh`) cover this over
  random stimulus.
- Every design reported NOT COVERED, including all multi-clock designs.
- Asynchronous reset timing (rule 3) and X behaviour (rule 1).
- Designs beyond those listed above: the formal gate has not been run on them.

## What these gates do not cover

The UVM-side checks (`prove-equiv`, and the class, coverage and SVA parts of
`roundtrip-check`) were not changed and were not re-examined for the same weakness. They
compare constraints, coverage models and assertion observers through `svcc`'s own IR.
Whether a C++ width error in a CRAVE constraint expression could pass them is an open
question, listed in `STATUS.md`.

## What was removed

- `RtlModule` and the rest of the `rtl` dialect in `svcc-ir`; `IrModule` keeps only the
  names of the design modules it saw.
- `svcc-front-sv/src/rtl_lower.rs`, `svcc-front-sc/src/rtl_lift.rs`,
  `svcc-emit-sc/src/rtl_emit.rs`, `svcc-emit-sv/src/rtl_emit.rs`.
- The textual miter builder and the lenient Btor2 unroller in `svcc-equiv/src/sec.rs`
  (the unroller skipped lines it did not know and misread constants wider than 128 bits).
  `svcc-equiv/src/pair.rs` replaces them and rejects any Btor2 line it does not model.
- The fallback that sent a module to the text-based path when the typed path could not
  convert it. Such a module is now an error.

`svcc sc2sv` and `svcc c2rtl` still accept `SC_MODULE`s and golden functions; what they
print for them is the explicit-width text of the proof, with the `svcc__ub` output, not
hand-style RTL.
