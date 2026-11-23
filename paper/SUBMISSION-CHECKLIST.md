# Before submitting the two papers

Open items from the PC-style review of 2026-11-23 (full review kept outside the repository).

- Zenodo: archive the v0.1.0 tag, then put the DOI in SoftwareX C2 and in the preprint's Artifacts line.
- Verilator: file the self-instantiating-top issue upstream (5.052 and 5.055) and cite the issue link.
- SCAV: read the JETC article (doi 10.1145/3798042) and confirm SCAV is its implementation; summarize the article's own evaluation in RQ4.
- Stress run: regenerate `benchmarks/results/rtl_stress.md` from the M5.2 run (it still shows the M5.1 run: 1,170 matches, "15 designs"), and add the per-stage split of the 994 non-matching modules.
- CVFPU: root-cause the two `fpnew_fma_multi` mismatches before Access submission.
- Tool (chosen to stay as is for the preprint): bounded verdict exits 0; clock-copy header with `gclk.posedge_event()` passes. Both are stated in the papers as limits.
- Reviewer suggestions not done: proof on the 1,185 matching corpus modules, Verilator `--sc` and LLM baselines, scheduling and structural mutation operators, clang AST cross-check of the C++ reader.
- SoftwareX: check the official template and the generative-AI declaration wording.
- Set `\draftfalse` in `paper/rtl-tv/main.tex` before building the arXiv source.
