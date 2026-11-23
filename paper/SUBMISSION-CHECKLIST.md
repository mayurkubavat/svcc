# Submission status of the papers

## IEEE Access: submitted 2026-11-23

"Translation Validation of SystemVerilog-to-SystemC RTL Conversion by Reading the Generated C++",
Research Article, submitted through the IEEE Author Portal (Research Exchange) on 2026-11-23,
submission ID d6d5d20d-a8eb-4ad7-8dae-0bf62c1ccc7e. The ScholarOne manuscript ID follows by e-mail;
add it here when it arrives.

- Submitted build: the Mac build of `paper/ieee-access` before 7feab51 (the review PDF opens
  "RTL is written in SystemVerilog"). 7feab51 adds acronym definitions at first use (RTL, HLS, AXI,
  UVM, UB) and the outputs of Table 2 row 2; carry these into the revision, or into a
  return-to-draft if the editorial office asks for the acronyms.
- Uploaded with it: LaTeX source, cover letter, biography and photo sheet, and the SoftwareX
  manuscript as the related manuscript under review.
- PC-style review of 2026-11-23 (kept outside the repository): points expected from reviewers,
  to prepare for the revision: wider mutation operators (operators, constants) and an ablation
  against the first check; an independent second front end (yosys-slang) for the reference; the
  scheduling gap; related work (SCCL, ICSC, EquivFusion, Lööw 2025 are in refs.bib, uncited;
  hw-cbmc/EBMC; commercial C-to-RTL tools); coverage for co-simulation; RQ4 under one harness.

## Done since the review

- Verilator: both behaviors filed upstream (verilator/verilator#9328, #9329) and cited; fix
  branches prepared for upstream PRs.
- CVFPU `fpnew_fma_multi`: root-caused (M6.4, benchmarks/results/cosim_adjudication.md).
- Proof on the matching corpus modules: done (M6.3, benchmarks/results/rtl_prove_corpus.md).
- Bounded verdict exits 2 (M6.2).

## Still open

- Zenodo: publish 0.1.1 to crates.io and PyPI (version strings in the tag read 0.1.0), and add
  ORCID to the Zenodo record.
- svlang: release af574c9 (the `2**N` fix) as 0.5.2 so the papers can cite a released version.
- SCAV: read the JETC article (doi 10.1145/3798042) and confirm SCAV is its implementation.
- Stress run: regenerate `benchmarks/results/rtl_stress.md` from the latest run, with the
  per-stage split of the non-matching modules (a full run with proofs, rtl_stress_m6.jsonl, was
  started 2026-11-23).
- SoftwareX: check the official template and the generative-AI declaration wording.
- arXiv (`paper/rtl-tv`): set `\draftfalse` and bring it in line with the IEEE Access text before
  building the arXiv source.
