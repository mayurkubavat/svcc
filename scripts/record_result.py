#!/usr/bin/env python3
"""Append a structured development/verification/benchmark record to:
  1. results/experiment_log.jsonl  (machine-readable paper & regression log)
  2. results/journal.md            (human-readable engineering lab notebook)
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import platform
import subprocess
from pathlib import Path


def git_head(repo: Path) -> str:
    try:
        out = subprocess.check_output(
            ["git", "-C", str(repo), "rev-parse", "--short", "HEAD"],
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
        return out or "uncommitted"
    except Exception:
        return "uncommitted"


def rustc_ver() -> str:
    try:
        return subprocess.check_output(["rustc", "--version"], text=True).strip()
    except Exception:
        return "unknown"


def main() -> None:
    parser = argparse.ArgumentParser(description="Record a verified uvir experiment/milestone result.")
    parser.add_argument("--milestone", required=True, help="Milestone ID, e.g. M0-scaffold")
    parser.add_argument("--title", required=True, help="Short result title")
    parser.add_argument("--command", required=True, help="Exact verification/benchmark command executed")
    parser.add_argument("--metrics-json", default="{}", help="JSON object string of measured metrics")
    parser.add_argument("--notes", default="", help="Engineering observations / open items")
    args = parser.parse_args()

    repo = Path(__file__).resolve().parent.parent
    results_dir = repo / "results"
    results_dir.mkdir(parents=True, exist_ok=True)

    ts = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()
    metrics = json.loads(args.metrics_json)

    record = {
        "timestamp_utc": ts,
        "milestone": args.milestone,
        "title": args.title,
        "git_commit": git_head(repo),
        "host": f"{platform.system()}-{platform.machine()}",
        "rustc": rustc_ver(),
        "command": args.command,
        "metrics": metrics,
        "notes": args.notes,
    }

    jsonl_path = results_dir / "experiment_log.jsonl"
    with jsonl_path.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(record, sort_keys=True) + "\n")

    journal_path = results_dir / "journal.md"
    if not journal_path.exists():
        journal_path.write_text(
            "# `uvir` Engineering & Paper Result Journal\n\n"
            "Chronological log of verified compiler builds, Z3 equivalence proofs, and latency benchmarks.\n\n",
            encoding="utf-8",
        )

    metrics_md = "\n".join(f"  - `{k}`: `{v}`" for k, v in sorted(metrics.items())) or "  - *(none)*"
    entry = (
        f"## [{ts}] `{args.milestone}` — {args.title}\n"
        f"- **Commit**: `{record['git_commit']}` (`{record['host']}`, `{record['rustc']}`)\n"
        f"- **Command**: `{args.command}`\n"
        f"- **Measured Metrics**:\n{metrics_md}\n"
        f"- **Notes**: {args.notes or 'N/A'}\n\n"
    )
    with journal_path.open("a", encoding="utf-8") as fh:
        fh.write(entry)

    print(f"Recorded '{args.milestone}' -> {jsonl_path.relative_to(repo)} & {journal_path.relative_to(repo)}")


if __name__ == "__main__":
    main()
