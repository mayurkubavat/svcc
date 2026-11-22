# svcc (Python)

SystemVerilog to SystemC conversion for RTL, with formal checks of the result.

**Status: 0.1, early release.** The RTL conversion path is the part of `svcc` that has been
checked on real designs. Read "What is checked" and "Known limits" before relying on it.

## What it does

- `rtl_to_sc` (CLI `svcc rtl2sc`): converts synthesizable SystemVerilog RTL to C++17 SystemC (`SC_MODULE`s).
  The design is elaborated by [`svlang`](https://crates.io/crates/svlang); every expression
  carries its width and signedness, and the emitter truncates explicitly wherever C++ would
  compute in a wider type.
- `rtl_prove_json` (CLI `svcc rtl-prove`): proves the generated SystemC equal to the source with Z3. The generated
  header is read back with C++ arithmetic rules, so a C++ width error is a counterexample.
- CLI only, `svcc rtl-cosim`: compiles the generated SystemC against a real SystemC library and runs it
  against Verilator on the same random stimulus. This is simulation evidence, not a proof.
- `prove_sec_json` (CLI `svcc prove-sec`): sequential equivalence between two RTL designs, or between an untimed
  C++ function and pipelined RTL some cycles later (C2RTL).

The accepted subset and the mapping are in
[`docs/rtl_subset.md`](https://github.com/mayurkubavat/svcc/blob/main/docs/rtl_subset.md);
what each check proves and assumes is in
[`docs/formal_gates.md`](https://github.com/mayurkubavat/svcc/blob/main/docs/formal_gates.md).

## Install

```bash
pip install svcc
```

Wheels are built for CPython 3.9 and later (abi3). The proof functions need `z3` on `PATH`.
The `svcc` command-line tool, including `rtl-cosim`, is in the Rust crate
(`cargo install svcc`).

```python
import json, svcc

src = open("counter.sv").read()
header = svcc.rtl_to_sc("counter.sv", src, ["counter"])   # C++17 SystemC text
for item in json.loads(svcc.rtl_prove_json("counter.sv", src)):
    print(item)                                            # Proved / Skipped / NotCovered
```

Errors (a construct outside the subset, a parse error) raise `ValueError` with the source
location. Also available: `sv_to_sc`, `sc_to_sv`, `prove_equivalent_json`, `prove_sec_json`,
and `svcc.__version__`.

## What is checked

- **Fail closed.** A construct outside the subset is an error with a source location, not
  a guess. Skipped constructs (`initial` blocks, assertions, system tasks) are reported as
  notes.
- **Proof.** For a design in the proof model, the generated SystemC is proved equal to the
  source for all runs from equal start states (by induction), or for a bounded number of
  cycles when induction does not close. Designs outside the model are reported as NOT
  COVERED (exit code 3), never as passing.
- **Real designs.** Every module of 15 open-source designs (2,184 modules, among them ibex,
  cv32e40p, cva6, OpenTitan, Caliptra, common_cells) was tried as a top. 1,191 converted;
  all of them compile against SystemC, and 1,170 match Verilator over 3,000 random steps.
  The remaining differences are listed in the repository
  (`benchmarks/results/rtl_stress.md`); the ones that were `svcc`'s mapping of derived
  clocks are now rejected with an error.

## Known limits

- **Two-state.** `x` and `z` read as 0, in the conversion and in the proof. A design that
  relies on X behaviour is outside the model.
- **Clocks.** A register may be clocked by an input port, a plain copy of it, or a register
  output (a divider written with nonblocking assignments). A gated clock, a clock made by
  other logic, or a clock written with a blocking assignment is rejected: SystemC would run
  it one delta cycle late. Signals written by blocks on different clocks are rejected.
- **Proof model.** One clock; an asynchronous reset is treated as taking effect at the clock
  step. Designs with several clocks are simulated, not proved.
- **Front end.** `svlang` is the meaning of SystemVerilog for both the conversion and the
  proof, so an `svlang` defect can affect both sides of a proof alike. This release uses
  svlang 0.5.1, which fixes the three elaboration defects the stress run found.
- **Not supported:** interfaces, `inout`, unpacked structs, hierarchical references, tasks,
  `foreach`, arrays local to a process, gate primitives, among others (see the subset page).
- **Known differences on real designs.** In the 2,184-module stress run, the modules that
  still differ from Verilator are `popcount` and `prim_pad_attr` (the Verilator reference
  is wrong or sees different source), `stream_mux` with a placeholder parameter of 0
  (undefined in two-state), and `fpnew_cast_multi` / `fpnew_opgroup_block` from cvfpu, which
  are not yet explained. (`lfsr` and `cc_lfsr` differed because of an svlang defect fixed
  in svlang 0.5.1, which this release uses; they now match.)
- **Other directions.** The UVM/verification-class translation (`sv2sc`, `sc2sv`) and the
  SystemC-to-SystemVerilog direction ship in this package but have not had the same scrutiny
  on real code as the RTL path. Treat their output as a starting point.

## License

Apache-2.0. Copyright 2026 Mayur Kubavat.
