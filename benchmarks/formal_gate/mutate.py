#!/usr/bin/env python3
"""Mutation check of the formal gate itself.

For each design, the generated SystemC header is mutated one place at a time with edits
that change the width or signedness at which C++ evaluates an expression:

  unwrap  remove a truncating construction: sc_uint<8>(e) -> (e)
  widen   construct one bit wider:          sc_uint<8>(e) -> sc_uint<9>(e)
  resign  flip the signedness:              sc_uint<8>(e) <-> sc_int<8>(e)
  uncast  remove a built-in cast:           uint64_t(e) -> (e)

Every mutant is given to two independent judges:

  prove  `svcc rtl-prove --header`   formal: generated C++ against the source
  cosim  `svcc rtl-cosim --header`   simulation: real SystemC against Verilator

The row that matters is "cosim finds a difference, prove does not": such a mutant would be
a wrong proof. Mutants that do not compile are dropped.

Usage: mutate.py [--per-design N] [--steps N] [--out results.json] design.sv ...
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SVCC = os.environ.get("SVCC", str(ROOT / "target/debug/svcc"))
WRAP = re.compile(r"sc_dt::sc_u?int<\d+>\(")


def run(cmd, timeout=600):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=ROOT)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return 124, "timeout"


def in_body(code, pos):
    """True when `pos` is inside a process or function body, not a member declaration."""
    line_start = code.rfind("\n", 0, pos) + 1
    line = code[line_start:code.find("\n", pos)]
    return not ('{"' in line or line.lstrip().startswith(("sc_core::", "SC_")))


def close_paren(code, i):
    depth = 1
    while depth and i < len(code):
        depth += {"(": 1, ")": -1}.get(code[i], 0)
        i += 1
    return i if depth == 0 else None


def mutants(code, ops):
    """Yields (offset, operator, mutated text)."""
    if "unwrap" in ops:
        # Remove one truncating construction, keeping the expression inside.
        for m in WRAP.finditer(code):
            end = close_paren(code, m.end())
            if in_body(code, m.start()) and end:
                yield m.start(), "unwrap", code[:m.start()] + "(" + code[m.end():end - 1] + ")" + code[end:]
    if "widen" in ops:
        # Construct one bit wider: sc_uint<8>(e) -> sc_uint<9>(e).
        for m in re.finditer(r"(sc_dt::sc_u?int<)(\d+)(>\()", code):
            n = int(m.group(2))
            if in_body(code, m.start()) and n < 64:
                yield m.start(), "widen", code[:m.start()] + m.group(1) + str(n + 1) + m.group(3) + code[m.end():]
    if "resign" in ops:
        # Flip the signedness of one construction: sc_uint<8>(e) <-> sc_int<8>(e).
        for m in re.finditer(r"sc_dt::sc_(u?)int<(\d+)>\(", code):
            if in_body(code, m.start()):
                flipped = "sc_dt::sc_%sint<%s>(" % ("" if m.group(1) else "u", m.group(2))
                yield m.start(), "resign", code[:m.start()] + flipped + code[m.end():]
    if "uncast" in ops:
        # Remove one built-in cast: uint64_t(e) -> (e).
        for m in re.finditer(r"\b(u?int64_t|int|bool)\(", code):
            end = close_paren(code, m.end())
            if in_body(code, m.start()) and end:
                yield m.start(), "uncast", code[:m.start()] + "(" + code[m.end():end - 1] + ")" + code[end:]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("designs", nargs="+")
    ap.add_argument("--per-design", type=int, default=6)
    ap.add_argument("--steps", type=int, default=20000)
    ap.add_argument("--ops", default="unwrap,widen,resign,uncast")
    ap.add_argument("--out", default=str(ROOT / "benchmarks/results/formal_gate_mutation.json"))
    args = ap.parse_args()

    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        for d in args.designs:
            rc, out = run([SVCC, "rtl2sc", d, "-o", f"{tmp}/gen.hpp"])
            if rc != 0:
                print(f"skip {d}: rtl2sc failed", file=sys.stderr)
                continue
            code = Path(f"{tmp}/gen.hpp").read_text()
            all_m = list(mutants(code, args.ops.split(",")))
            # Evenly spaced sample, so the choice does not depend on anything random.
            step = max(1, len(all_m) // args.per_design)
            for off, op, text in all_m[::step][: args.per_design]:
                hp = f"{tmp}/mut.hpp"
                Path(hp).write_text(text)
                line = code.count("\n", 0, off) + 1
                crc, cout = run([SVCC, "rtl-cosim", d, "--steps", str(args.steps), "--header", hp])
                if crc != 0 and "mismatch" not in cout.lower():
                    verdict_cosim = "no-build"
                else:
                    verdict_cosim = "same" if crc == 0 else "differs"
                prc, pout = run([SVCC, "rtl-prove", d, "--header", hp])
                if "not checked" in pout:
                    verdict_prove = "rejected"
                elif prc == 0:
                    verdict_prove = "proved"
                elif "undefined behaviour" in pout:
                    verdict_prove = "undefined-behaviour"
                elif "DIFFERENCE FOUND" in pout:
                    verdict_prove = "differs"
                else:
                    verdict_prove = "error"
                rows.append({"design": Path(d).name, "line": line, "op": op, "cosim": verdict_cosim,
                             "prove": verdict_prove,
                             "detail": "" if verdict_prove != "error" else pout[-300:],
                             "cosim_detail": "" if verdict_cosim != "no-build" else cout[-300:]})
                print(f"{Path(d).name}:{line} {op} cosim={verdict_cosim} prove={verdict_prove}", flush=True)

    built = [r for r in rows if r["cosim"] != "no-build"]
    table = {}
    for r in built:
        table[f"cosim={r['cosim']} prove={r['prove']}"] = table.get(f"cosim={r['cosim']} prove={r['prove']}", 0) + 1
    wrong = [r for r in built if r["cosim"] == "differs" and r["prove"] == "proved"]
    summary = {"mutants": len(rows), "compiled": len(built), "table": table,
               "wrong_proofs": len(wrong), "rows": rows}
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(json.dumps(summary, indent=1) + "\n")
    print(json.dumps({k: v for k, v in summary.items() if k != "rows"}, indent=1))
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
