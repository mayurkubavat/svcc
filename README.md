# svcc

`svcc` converts synthesizable SystemVerilog RTL (IEEE 1800-2023) into readable SystemC
(IEEE 1666, C++17), and then checks the generated C++ against the source. The check reads the
generated header with C++17 and SystemC evaluation rules, treats undefined behavior as a
failure, and proves the result equal to the SystemVerilog source with Z3. A second command
simulates the generated SystemC against Verilator.

**Status: 0.1.2, early release.** The RTL path is the part that has been checked on real
designs. Read [What a result means](#what-a-result-means) and
[Known limits](#known-limits) before relying on it.

## Install

```bash
cargo install svcc        # command-line tool and Rust library (Rust 1.82 or later)
pip install svcc          # Python package (CPython 3.9 or later)
```

Python wheels are built for Linux x86_64 and macOS arm64; other platforms build from source
and need Rust. The proofs need the `z3` executable on `PATH` (tested with Z3 5.4.1).
Co-simulation needs `verilator` on `PATH` (tested with 5.052), a C++17 compiler and
`SYSTEMC_HOME` pointing at a SystemC 3.0 install (tested with 3.0.3).

## Quick start

```bash
# Convert synthesizable RTL to a SystemC header
svcc rtl2sc testdata/rtl/fsm_enum.sv --top fsm_enum -o fsm_enum.hpp

# Prove the generated header equal to its source (or a header you pass with --header)
svcc rtl-prove testdata/rtl/fsm_enum.sv
svcc rtl-prove testdata/rtl/fsm_enum.sv --header my_fsm_enum.hpp

# Simulate the generated SystemC against Verilator on random stimulus (not a proof)
svcc rtl-cosim testdata/rtl/fsm_enum.sv --top fsm_enum --steps 5000

# Prove an untimed C++ golden function equal to pipelined RTL, 2 cycles later
svcc prove-sec testdata/sc/alu_golden_c2rtl.hpp testdata/sv/alu_pipe_rtl.sv --latency 2
```

From Python:

```python
import json, svcc

src = open("counter.sv").read()
header = svcc.rtl_to_sc("counter.sv", src, ["counter"])   # SystemC header text
for item in json.loads(svcc.rtl_prove_json("counter.sv", src)):
    print(item)                                            # Proved / Skipped / NotCovered
```

Exit codes of `rtl-prove` and `prove-sec`: 0 when every module is proved, 1 on a mismatch or when
the solver gives no answer, 2 when a check is only bounded, 3 when a module is outside the
proof model (`--accept-bounded` turns 2 into 0, the behavior before 0.1.3). The other commands
exit with 0 or 1, so they can gate a regression run. `rtl-prove` takes a multi-file design the
same way as `rtl2sc`: more source files, `-f <list>`, `+incdir+`, `+define+` and `-G name=value`.

## What a result means

- **Proved**: equal for every run from equal start states, by induction over registers
  mapped by name.
- **Bounded**: induction did not close and 8 steps (`--bound K`) from the initial state found
  no difference. It is not a proof: it exits 2 and prints `[BND ] ... (bounded)`.
- **Mismatch**: with a concrete input sequence that shows the difference.
- **NOT COVERED** (exit 3): the design is outside the proof model, for example more than one
  clock. Use `rtl-cosim` for these designs.

What each check proves and assumes is in [`docs/formal_gates.md`](docs/formal_gates.md).
The accepted subset and the mapping to SystemC are in [`docs/rtl_subset.md`](docs/rtl_subset.md).

## Known limits

- Two-state model: X and Z read as 0, in the conversion and in the proof.
- The proof handles one clock, and sees an asynchronous reset at the clock step.
- The proof does not model SystemC delta-cycle scheduling. A hand-written header with a
  process on an internal copy of a clock can pass it; `rtl-cosim` is the check for that.
- `svlang` gives the meaning of SystemVerilog to the converter and to both sides of the
  proof, so a front-end defect can be invisible to the proof.
- `initial` blocks and system tasks such as `$display` are dropped with a note.
  Other constructs outside the subset are errors with a source location.
- The UVM testbench path (`sv2sc`, `sc2sv`, `roundtrip-check`) and the SystemC-to-SystemVerilog
  direction ship in the same tool but have not had the same scrutiny as the RTL path.

## Repository layout

| Path | Contents |
| :--- | :--- |
| `crates/svcc` | Command-line tool and Rust library |
| `crates/svcc-front-sv`, `crates/svcc-svlang` | SystemVerilog lowering; the only crate that depends on `svlang` |
| `crates/svcc-front-sc` | Reader that gives generated C++ its C++17 meaning |
| `crates/svcc-ir`, `crates/svcc-passes` | Typed intermediate representation and passes |
| `crates/svcc-emit-sc`, `crates/svcc-emit-sv` | SystemC emitter and SystemVerilog printer |
| `crates/svcc-equiv` | Btor2 and SMT-LIB2 generation, Z3 calls |
| `bindings/python` | Python package (PyO3) |
| `testdata/rtl` | RTL designs used by the proof and co-simulation scripts |
| `benchmarks/` | Benchmark suites, stress-run driver, results |
| `results/` | Experiment log (`experiment_log.jsonl`) and journal of every recorded run |
| `paper/` | Papers on svcc, see [`paper/README.md`](paper/README.md) |

## Reproducing the checks

```bash
./scripts/check.sh        # format, clippy, tests, and the proof over every repository design
./scripts/rtl_prove.sh    # proof only (needs z3)
./scripts/rtl_cosim.sh    # co-simulation only (needs verilator and SYSTEMC_HOME)
```

Every measured run is recorded in `results/experiment_log.jsonl` and summarized in
`results/journal.md`, with the commit, machine and tool versions.

## Citing

If you use svcc in published work, please cite the software archive
(DOI [10.5281/zenodo.24455889](https://doi.org/10.5281/zenodo.24455889), all versions) and
the paper in `paper/softwarex/`. `CITATION.cff` has the details.

The check method (reading the generated C++ and proving it against the source by
induction) is described in the preprint:

M. Kubavat, "Translation Validation of SystemVerilog-to-SystemC RTL Conversion by
Reading the Generated C++," arXiv:2611.31674, 2026.
[arXiv:2611.31674](https://arxiv.org/abs/2611.31674),
DOI [10.48550/arXiv.2611.31674](https://doi.org/10.48550/arXiv.2611.31674).
Source in `paper/rtl-tv/`.

## Support

Questions and bug reports: open a GitHub issue, or write to mkubavat01@gmail.com.

## License

Apache License 2.0, see [`LICENSE`](LICENSE).
