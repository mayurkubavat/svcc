#!/usr/bin/env python3
"""Stress the typed RTL path (`svcc rtl2sc` / `svcc rtl-cosim`) on real open-source RTL.

Every module of every corpus design is tried as a top. Three stages, each recorded:

  convert  svcc rtl2sc            -> ok | front (svlang rejects) | subset (svcc rejects with a
                                     located error) | crash | timeout
  compile  g++ -fsyntax-only      -> ok | error      (generated C++ against real SystemC)
  cosim    svcc rtl-cosim         -> pass | mismatch | build   (SystemC vs Verilator)

The corpus is the one the svlang front-end census uses: pinned open-source repositories
fetched outside this repo, with the census tool's scan working out the files and include
directories each module needs. Nothing from those repositories is executed; their sources are
only read by svcc, g++ (generated code only) and Verilator.

Usage:
  run_stress.py --census-tool <svlang>/tools/quality --corpus <dir> [--only ibex ...]
                [--stage convert|compile|cosim] [--jobs N] [--out results.jsonl]
  run_stress.py --report results.jsonl [--md out.md]
"""
import argparse
import collections
import concurrent.futures
import json
import os
import re
import resource
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def limit(mem_gb):
    def apply():
        b = int(mem_gb * (1 << 30))
        resource.setrlimit(resource.RLIMIT_AS, (b, b))
    return apply


def run(cmd, timeout, mem_gb=6, env=None):
    t0 = time.time()
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           preexec_fn=limit(mem_gb), env=env, errors="replace")
        return p.returncode, p.stdout, p.stderr, time.time() - t0
    except subprocess.TimeoutExpired:
        return None, "", "", time.time() - t0


NORMALIZE = [
    (re.compile(r"^\S+?:\d+:\d+: "), ""),
    (re.compile(r"'[^']*'"), "'…'"),
    (re.compile(r'"[^"]*"'), '"…"'),
    (re.compile(r"\b\d+\b"), "N"),
    (re.compile(r"\s+"), " "),
]


def construct(msg):
    for rx, to in NORMALIZE:
        msg = rx.sub(to, msg)
    return msg.strip()[:160]


def measure(design, module, path, plan, opts):
    files, incdirs = plan
    dargs = [f"+incdir+{d}" for d in incdirs + [os.path.join(design.dir, x) for x in design.entry.get("incdirs", [])]]
    dargs += [f"+define+{x}" for x in design.entry.get("defines", [])]
    params = design.entry.get("params", {}).get(module, [])
    dargs += [x for p in params for x in ("-G", p)]
    rec = {"design": design.name, "module": module, "file": design.rel(path), "files": len(files)}
    work = os.path.join(opts.work, design.name, module)
    os.makedirs(work, exist_ok=True)
    hpp = os.path.join(work, "dut.hpp")

    code, out, err, secs = run([opts.svcc, "rtl2sc", *files, *dargs, "--top", module, "-o", hpp], opts.timeout, mem_gb=opts.mem)
    rec["convert_s"] = round(secs, 2)
    if code is None:
        rec["convert"] = "timeout"
    elif code == 0:
        rec["convert"] = "ok"
        rec["notes"] = sum(1 for l in err.splitlines() if l.startswith("note:"))
        try:
            rec["sc_lines"] = sum(1 for _ in open(hpp))
        except OSError:
            pass
    else:
        lines = [l.strip() for l in err.splitlines() if l.strip()]
        first = next((l for l in lines if l.startswith("error:")), lines[0] if lines else "")
        if "[svcc-front-sv/svlang]" in first:
            rec["convert"] = "front"
        elif "[svcc-front-sv/hdl]" in first or "[svcc-emit-sc/hdl]" in first:
            rec["convert"] = "subset"
        else:
            rec["convert"] = "crash"
        msgs = [re.sub(r"^error: \[[^\]]*\] ", "", l) for l in lines if "panicked" not in l or True]
        msgs = [m.replace(design.base + os.sep, "") for m in msgs][:40]
        rec["messages"] = msgs[:8]
        rec["constructs"] = sorted({construct(m) for m in msgs if not m.startswith("...")})[:12]
        if rec["convert"] == "crash":
            rec["messages"] = lines[-6:]
    if rec["convert"] != "ok" or opts.stage == "convert":
        shutil.rmtree(work, ignore_errors=True)
        return rec

    tu = os.path.join(work, "tu.cpp")
    with open(tu, "w") as f:
        f.write('#include "dut.hpp"\n')
    code, out, err, secs = run(["g++", "-std=c++17", "-fsyntax-only", f"-I{opts.systemc}/include", tu], 300, mem_gb=8)
    rec["compile_s"] = round(secs, 2)
    if code == 0:
        rec["compile"] = "ok"
    else:
        rec["compile"] = "timeout" if code is None else "error"
        errs = [l for l in err.splitlines() if " error: " in l]
        rec["messages"] = [e.split(" error: ", 1)[1][:200] for e in errs[:5]]
        rec["constructs"] = sorted({construct(m) for m in rec["messages"]})
    if rec["compile"] != "ok" or opts.stage == "compile":
        if rec["compile"] == "ok":
            shutil.rmtree(work, ignore_errors=True)
        return rec

    env = dict(os.environ, SYSTEMC_HOME=opts.systemc)
    code, out, err, secs = run(
        [opts.svcc, "rtl-cosim", *files, *dargs, "--top", module, "--steps", str(opts.steps), "--keep", work],
        900, mem_gb=12, env=env)
    rec["cosim_s"] = round(secs, 2)
    if code == 0 and "[PASS]" in out:
        rec["cosim"] = "pass"
        m = re.search(r"on (\d+) outputs, (\d+) steps, (\d+) comparisons", out)
        if m:
            rec["outputs"], rec["comparisons"] = int(m.group(1)), int(m.group(3))
        shutil.rmtree(work, ignore_errors=True)
    else:
        text = err + out
        if code is None:
            rec["cosim"] = "timeout"
        elif "differs from Verilator" in text:
            rec["cosim"] = "mismatch" if "MISMATCH" in text else "runtime"
        elif "%Error" in text and "exited with" not in text:
            rec["cosim"] = "verilator"
        else:
            rec["cosim"] = "build"
        rec["messages"] = [l.strip()[:240] for l in text.splitlines() if l.strip()][:8]
    return rec


def do_run(opts):
    sys.path.insert(0, opts.census_tool)
    import census  # noqa: E402  (the svlang front-end census tool)
    os.makedirs(opts.work, exist_ok=True)
    with open(opts.out, "w") as out:
        rev = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        out.write(json.dumps({"meta": {"svcc": rev, "stage": opts.stage, "steps": opts.steps, "timeout": opts.timeout}}) + "\n")
        for entry in census.load_corpus():
            if opts.only and entry["name"] not in opts.only:
                continue
            if not os.path.isdir(os.path.join(opts.corpus, entry["name"])):
                print(f"{entry['name']}: not fetched, skipped", file=sys.stderr)
                continue
            t0 = time.time()
            design = census.Design(entry, opts.corpus)
            units = [(n, p, design.closure(p)) for n, p in design.units()]
            if opts.module:
                units = [u for u in units if u[0] in opts.module]
            if opts.limit:
                units = units[: opts.limit]
            tally = collections.Counter()
            with concurrent.futures.ThreadPoolExecutor(opts.jobs) as pool:
                for rec in pool.map(lambda u: measure(design, u[0], u[1], u[2], opts), units):
                    rec["license"] = entry.get("license", "")
                    key = rec.get("cosim") and "cosim:" + rec["cosim"] or rec.get("compile") and "compile:" + rec["compile"] or "convert:" + rec["convert"]
                    tally[key] += 1
                    out.write(json.dumps(rec, sort_keys=True) + "\n")
            out.flush()
            print(f"{design.name:20} {len(units):5} modules in {time.time() - t0:5.0f}s: "
                  + ", ".join(f"{k} {v}" for k, v in sorted(tally.items())), file=sys.stderr)


def do_report(path, md_path):
    recs, meta = [], {}
    for line in open(path):
        r = json.loads(line)
        if "meta" in r:
            meta = r["meta"]
        else:
            recs.append(r)
    out = []
    w = out.append
    w("# Typed RTL path on real designs\n")
    w(f"svcc `{meta.get('svcc', '?')}`, stage `{meta.get('stage')}`, {meta.get('steps')} cosim steps. "
      "Every module of each design is tried as a top with its default parameters.\n")
    designs = sorted({r["design"] for r in recs})
    w("| Design | Modules | svlang rejects | svcc rejects (subset) | Crash / timeout | Converted | C++ compiles | Matches Verilator | Mismatch | Cosim not built |")
    w("| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    tot = collections.Counter()

    def row(name, rs):
        c = collections.Counter()
        for r in rs:
            c["n"] += 1
            c[r["convert"]] += 1
            if r.get("compile") == "ok":
                c["cc"] += 1
            if "cosim" in r:
                c["cosim_" + r["cosim"]] += 1
        other = c["cosim_build"] + c["cosim_verilator"] + c["cosim_timeout"] + c["cosim_runtime"]
        w(f"| {name} | {c['n']} | {c['front']} | {c['subset']} | {c['crash'] + c['timeout']} | {c['ok']} | {c['cc']} | {c['cosim_pass']} | {c['cosim_mismatch']} | {other} |")
        return c

    for d in designs:
        tot.update(row(d, [r for r in recs if r["design"] == d]))
    row("**Total**", recs)

    def ranked(title, want, n=25):
        cnt, ex = collections.Counter(), {}
        for r in recs:
            if not want(r):
                continue
            for c in set(r.get("constructs", [])[:6]) or {"(no message)"}:
                cnt[c] += 1
                ex.setdefault(c, f"{r['design']}/{r['module']}")
        if not cnt:
            return
        w(f"\n## {title}\n")
        w("| Modules | Message | Example |")
        w("| ---: | :--- | :--- |")
        for c, k in cnt.most_common(n):
            w(f"| {k} | {c.replace('|', chr(92) + '|')} | {ex[c]} |")

    ranked("Rejected by svcc with a located error (subset gaps)", lambda r: r["convert"] == "subset")
    ranked("Rejected by svlang before lowering", lambda r: r["convert"] == "front", 15)
    ranked("Crashes and timeouts", lambda r: r["convert"] in ("crash", "timeout"))
    ranked("Generated C++ does not compile", lambda r: r.get("compile") == "error")
    for kind, title in (("mismatch", "Mismatch against Verilator"), ("runtime", "Simulation aborted"),
                        ("build", "Cosim executable not built"), ("verilator", "Verilator rejects the source"),
                        ("timeout", "Cosim timeout")):
        rs = [r for r in recs if r.get("cosim") == kind]
        if rs:
            w(f"\n## {title} ({len(rs)})\n")
            for r in rs[:40]:
                msg = next((m for m in r.get("messages", []) if "MISMATCH" in m or "rror" in m), (r.get("messages") or [""])[0])
                w(f"- `{r['design']}/{r['module']}`: {msg[:200]}")
    text = "\n".join(out) + "\n"
    if md_path:
        open(md_path, "w").write(text)
    else:
        sys.stdout.write(text)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--census-tool")
    ap.add_argument("--corpus")
    ap.add_argument("--only", nargs="*", default=[])
    ap.add_argument("--module", nargs="*", default=[], help="only these modules")
    ap.add_argument("--stage", choices=["convert", "compile", "cosim"], default="cosim")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--steps", type=int, default=5000)
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--mem", type=float, default=3.0, help="memory cap in GB for one svcc conversion")
    ap.add_argument("--out", default="rtl_stress.jsonl")
    ap.add_argument("--work", default=os.path.join(ROOT, "target", "rtl-stress"))
    ap.add_argument("--svcc", default=os.path.join(ROOT, "target", "release", "svcc"))
    ap.add_argument("--systemc", default=os.environ.get("SYSTEMC_HOME", os.path.expanduser("~/.local/opt/accellera-svcc/systemc")))
    ap.add_argument("--report")
    ap.add_argument("--md")
    opts = ap.parse_args()
    if opts.report:
        do_report(opts.report, opts.md)
    else:
        if not (opts.census_tool and opts.corpus):
            ap.error("--census-tool and --corpus are required to run")
        do_run(opts)


if __name__ == "__main__":
    main()
