# Papers

| Folder | What it is | State |
| :--- | :--- | :--- |
| `rtl-tv/` | "Translation Validation of SystemVerilog-to-SystemC RTL Conversion by Reading the Generated C++": the check method, the earlier version that failed, and its evaluation. Numbers and where each was measured are in `rtl-tv/numbers.tex`. | Preprint, [arXiv:2611.31674](https://arxiv.org/abs/2611.31674) |
| `softwarex/` | "svcc: SystemVerilog-to-SystemC Conversion with a Formal Check of the Generated C++": software paper for SoftwareX, describing v0.1.2. | Submitted |
| `early-draft/` | The first technical report on svcc. | Superseded, see below |
| `refs.bib` | Bibliography of the early draft; `references/` tracks which entries it holds. | |

`early-draft/` is kept as the starting point for a later paper on the testbench path (UVM,
constraints, coverage). Its RTL results come from a text-based equivalence gate that was
later found to be unsound and was removed; do not quote its RTL numbers. `rtl-tv/` explains
what went wrong and replaces those results. The early draft was not submitted to a journal.

Build any of them with `make` in its folder (pdflatex and bibtex).
