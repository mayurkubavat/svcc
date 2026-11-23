# IEEE Access paper: reproducing the results

Numbers: `paper/ieee-access/numbers.tex`. Journal entries: `results/journal.md`.

| Claim (macros) | Command | Result file | Journal entry |
| --- | --- | --- | --- |
| Repository designs under the proof: 50 designs, 45 proved, 5 not covered, 0 failed, 0 bounded (`\nDesigns`..`\nMaxPaired`) | `scripts/rtl_prove.sh` (step 7 of `scripts/check.sh`) | printed by the script | `M6.1-rtl-prove-main`, `M6.2-bounded-exit-multifile-prove` |
| Mutation check of the gate: 281 compiled mutants, 17 differ in simulation, 14 found by the proof, 3 rejected by the reader, 0 accepted (`\nMut*`) | `python3 benchmarks/formal_gate/mutate.py` | `benchmarks/results/formal_gate_mutation_unwrap.json`, `benchmarks/results/formal_gate_mutation_width_sign.json` | `M6-typed-formal-gates` (see `note-history-rewrite` for the commit) |
| C2RTL golden models: 8 proved (`\nGolden`) | `svcc prove-sec <golden.hpp> <rtl.sv> --latency L`, as in `scripts/check.sh` | printed | `M6-typed-formal-gates` |
| Co-simulation against Verilator on 2,184 real modules: 1,185 match, 5 differ, clock rejections 65 to 13 (`\nModules`..`\nSteps`) | `python3 benchmarks/rtl_stress/run_stress.py --stage cosim ...` | `benchmarks/results/rtl_stress_m52_cosim.jsonl` | `M5.2-clock-gating` |
| Adjudication of the 5 differences with Icarus and IEEE 1800 | `python3 benchmarks/rtl_stress/adjudicate.py ...` | `benchmarks/results/cosim_adjudication.md` | `M6.4-cosim-adjudication` |
| Proof of the 1,188 matching module records: 600 proved, 129 bounded, 45 fail (0 functional differences), 414 not covered (`\nProofRecords`..`\nCorpusMaxRegs`) | `python3 benchmarks/rtl_stress/run_stress.py --prove-from benchmarks/results/rtl_stress_m52_cosim.jsonl --out benchmarks/results/rtl_prove_corpus.jsonl`, then `--report` | `benchmarks/results/rtl_prove_corpus.jsonl`, `benchmarks/results/rtl_prove_corpus.md` | `M6.3-prove-corpus` |
| Comparison with SCAV: 18 probes, 8 corpus RTL designs (`\nProbes`, `\nCorpusRtl`) | `python3 benchmarks/scav_compare/run_compare.py` | `benchmarks/results/scav_compare_summary.json` | `M5-scav-head-to-head` |
| Code size: 25,700 lines of Rust (`\nRustLines`) | `find crates bindings -name '*.rs' \| xargs cat \| wc -l` | - | - |

Known gaps, stated in `numbers.tex`:
- `\nSvlangRej`, `\nSvccRej` and `\nCrash` (the split of the 994 rejected
  modules) and the per-design table in `benchmarks/results/rtl_stress.md`
  are from the earlier M5.1 run and are marked to be regenerated from M5.2.
- The full M6.3 run took about an hour on 8 cores with a 600 s per-module
  proof limit; `--jobs` and `--mem` set the parallelism.
