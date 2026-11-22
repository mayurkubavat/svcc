#!/usr/bin/env python3
"""Record or list falsifiable architectural & performance predictions in results/predictions.jsonl."""

from __future__ import annotations

import argparse
import datetime as dt
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="Record a testable svcc prediction for later evaluation.")
    parser.add_argument("--id", required=True, help="Prediction ID, e.g. P1")
    parser.add_argument("--area", required=True, help="Subsystem area (crv, sva, sched, types, frontend)")
    parser.add_argument("--hypothesis", required=True, help="Quantitative falsifiable prediction")
    parser.add_argument("--eval-plan", required=True, help="Experiment that will confirm or falsify it")
    parser.add_argument("--status", default="open", choices=["open", "confirmed", "falsified"])
    args = parser.parse_args()

    repo = Path(__file__).resolve().parent.parent
    out_path = repo / "results" / "predictions.jsonl"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    rec = {
        "recorded_utc": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
        "id": args.id,
        "area": args.area,
        "status": args.status,
        "hypothesis": args.hypothesis,
        "eval_plan": args.eval_plan,
    }
    with out_path.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(rec, sort_keys=True) + "\n")
    print(f"Recorded prediction {args.id} -> {out_path.relative_to(repo)}")


if __name__ == "__main__":
    main()
