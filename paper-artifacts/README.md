# Paper artifacts

Public record of the evidence behind each svcc paper, for reviewers and
readers. Every number a paper quotes is defined once in that paper's
`paper/<paper>/numbers.tex`, with a comment naming where it was measured.
Each folder here maps those numbers to the command that regenerates them, the
committed result file, and the entry in `results/journal.md` that recorded
the run (commit, machine, tool versions).

| Folder | Paper | Venue | Source |
| --- | --- | --- | --- |
| `ieee-access/` | svcc translation validation, full paper | IEEE Access (Access-2026-61569, under review) | `paper/ieee-access/` |
| `rtl-tv/` | Translation validation of SV-to-SystemC RTL conversion | arXiv preprint | `paper/rtl-tv/` |
| `softwarex/` | svcc v0.1.2 software paper | Elsevier SoftwareX (SOFTX-S-26-02382) | `paper/softwarex/` |

Each submission is tagged `artifacts/<paper>-<round>` (for example
`artifacts/ieee-access-r0`), so a reviewer can check out exactly the state the
paper describes. Archived releases carry a Zenodo DOI (v0.1.2:
10.5281/zenodo.24455889).

## Setup common to all papers

```
cargo build --release -p svcc        # Rust 1.82 or later
pip install z3-solver                # proofs; the runs used Z3 4.15.5 from pip
export SYSTEMC_HOME=<SystemC 3.0.x, C++17>   # co-simulation only
verilator --version                  # co-simulation only; runs used 5.052
scripts/check.sh                     # the repository gate, 8 steps
```

The real-design corpus (16 open-source code bases, 2,184 modules) is not
redistributed here. It is the corpus of the svlang front-end census: pinned
open-source repositories fetched outside this repository, with the census tool
(`<svlang>/tools/quality`) working out each module's files and include
directories. The header of `benchmarks/rtl_stress/run_stress.py` gives the
stages and options.
