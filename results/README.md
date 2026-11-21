# `uvir` Experiment & Verification Results Directory

All development milestones, Z3 equivalence checks, and performance benchmarks are persisted here for engineering traceability and paper tables:

- `experiment_log.jsonl`: Append-only JSON Lines stream (one verified run per line).
- `journal.md`: Human-readable Markdown lab notebook automatically updated alongside `experiment_log.jsonl`.

## Recording a New Result

```bash
python3 scripts/record_result.py \
  --milestone "M0-baseline" \
  --title "Initial 8-crate UVIR hub-and-spoke workspace, Z3 QF_BV miter, & APB4 round-trip" \
  --command "./scripts/check.sh && cargo run --release -p uvir-cli -- bench testdata/sv/apb_timer_item.sv --iters 100" \
  --metrics-json '{"tests_passed": 5, "avg_sv2sc_us": 120, "avg_sc2sv_us": 95, "z3_verdict": "EquivalentUnsat"}' \
  --notes "Pinned svlang @ 478a0d10; zero clippy warnings."
```
