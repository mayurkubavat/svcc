#!/usr/bin/env python3
"""Head-to-head RTL check of svcc against SCAV (github.com/zsf1998/SCAV, branch `all`).

For every input design this script runs both translators in the Verilog -> SystemC
direction and then judges the generated SystemC the same way for both tools:

  1. does the translator finish without an internal error,
  2. does the output compile against a real Accellera SystemC install (g++ -lsystemc),
  3. does it produce the same outputs as the original RTL (Icarus Verilog) over 397
     cycles of identical pseudo-random stimulus (see cosim.py).

For svcc it also records the verdict of `svcc roundtrip-check`, so that the verdict can be
compared with the independent simulation result.

Inputs: probes/*.v (18 small synthesizable Verilog-2005 designs written for this study),
the 8 RTL designs of benchmarks/corpus (13..20), and the 9 Verilog examples SCAV ships.
SCAV is cloned at a pinned commit into the output directory; nothing from it is copied
into this repository.

Requires: cargo build --release -p svcc, g++, libsystemc-dev, iverilog, z3, python3 `ply`.
Usage:    python3 benchmarks/scav_compare/run_compare.py [--out DIR] [--scav-dir DIR]
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
SVCC = os.path.join(ROOT, "target", "release", "svcc")
SHIMS = os.path.join(ROOT, "testdata", "cpp_shims")
SCAV_URL = "https://github.com/zsf1998/SCAV.git"
SCAV_SHA = "2ef2e6bb69b81945d6eabaae37e6973401599fc7"
SCAV_OWN = ["halfadder", "fulladder", "even_divide3", "moore_fsm", "s_to_p", "rom_ctrl",
            "code_83", "SM3_controller", "CompressExtend"]


def sh(cmd, cwd=None, timeout=300):
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        return 124, "", "timeout"


def top_module(path):
    mods = re.findall(r"^\s*module\s+(\w+)", open(path).read(), re.M)
    return mods[-1]


def cosim(ref, top, hdr, wd, extra):
    rc, out, err = sh([sys.executable, "-I", os.path.join(HERE, "cosim.py"), ref, top, hdr, wd] + extra)
    last = (out.strip().split("\n") or [""])[-1] or err.strip()[:120]
    return {0: "match", 1: "mismatch"}.get(rc, "not_run"), last[:160]


def to_verilog2005(sv_path, v_path):
    t = open(sv_path).read()
    t = re.sub(r"input +logic", "input ", t)
    t = re.sub(r"output +logic", "output reg", t)
    t = re.sub(r"^( *)logic ", r"\1reg ", t, flags=re.M)
    t = t.replace("always_ff", "always").replace("always_comb", "always @(*)")
    open(v_path, "w").write(t)


def run_svcc(src, out, dv_inc):
    name = os.path.splitext(os.path.basename(src))[0]
    hpp = os.path.join(out, name + ".hpp")
    r = {"sv2sc_exit": sh([SVCC, "sv2sc", src, "-o", hpp])[0]}
    rc, o, e = sh([SVCC, "roundtrip-check", src, "--shims", "testdata/cpp_shims"], cwd=ROOT)
    r["roundtrip"] = "pass" if rc == 0 else "fail"
    m = re.search(r"SEC\([^)]*\)=(\w+)", o + e)
    r["sec_verdict"] = m.group(1) if m else None
    if rc != 0:
        r["roundtrip_error"] = next((line for line in (o + e).split("\n") if "error" in line), "")[:160]
    cc = os.path.join(out, name + ".cc")
    open(cc, "w").write('#include "%s"\n' % hpp)
    r["compiles_real_systemc"] = sh(["g++", "-std=c++17", "-fsyntax-only", "-w", "-I" + dv_inc, cc])[0] == 0
    r["cosim"], r["cosim_detail"] = ("not_run", "does not compile")
    if r["compiles_real_systemc"]:
        r["cosim"], r["cosim_detail"] = cosim(src, top_module(src), hpp, os.path.join(out, "cos_" + name), ["-I" + dv_inc])
    return r


def run_scav(src_v, scav, out, ref=None):
    name = os.path.splitext(os.path.basename(src_v))[0]
    wd = os.path.join(out, name)
    os.makedirs(os.path.join(wd, "test"), exist_ok=True)
    v = os.path.join(wd, "test", name + ".v")
    shutil.copy(src_v, v)
    v2sc = os.path.join(scav, "v2sc")
    rc, o, _ = sh([sys.executable, "-I", "examples/example_parser.py", v], cwd=v2sc)
    open(os.path.join(wd, "test", name + "_ast.txt"), "w").write(o)
    r = {"parses": rc == 0 and bool(o.strip())}
    rc, _, e = sh([sys.executable, "-I", os.path.join(v2sc, "v2sc.py"), name], cwd=wd)
    r["translator_exit"] = rc
    if rc != 0:
        r["translator_error"] = (re.findall(r"^\w+Error.*$", e, re.M) or [e.strip()[-120:]])[-1][:120]
    cpp = os.path.join(wd, "test", name + ".cpp")
    hpp = os.path.join(wd, "test", name + ".hpp")
    r["compiles_real_systemc"] = os.path.exists(cpp) and sh(["g++", "-std=c++17", "-fsyntax-only", "-w", cpp], cwd=os.path.join(wd, "test"))[0] == 0
    r["cosim"], r["cosim_detail"] = ("not_run", "does not compile")
    if r["compiles_real_systemc"]:
        refv = ref or src_v
        r["cosim"], r["cosim_detail"] = cosim(refv, top_module(refv), hpp, os.path.join(wd, "cos"), [cpp])
    return r


def tally(rows, tool):
    n = len(rows)
    t = {"designs": n,
         "compiles_real_systemc": sum(1 for r in rows if r[tool]["compiles_real_systemc"]),
         "cosim_match": sum(1 for r in rows if r[tool]["cosim"] == "match")}
    if tool == "scav":
        t["translator_internal_error"] = sum(1 for r in rows if r[tool]["translator_exit"] != 0)
    else:
        t["roundtrip_pass"] = sum(1 for r in rows if r[tool]["roundtrip"] == "pass")
        t["roundtrip_pass_but_cosim_mismatch"] = sum(1 for r in rows if r[tool]["roundtrip"] == "pass" and r[tool]["cosim"] == "mismatch")
        t["roundtrip_fail_but_cosim_match"] = sum(1 for r in rows if r[tool]["roundtrip"] == "fail" and r[tool]["cosim"] == "match")
    return t


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(ROOT, "tmp", "scav_compare"))
    ap.add_argument("--scav-dir", default=None)
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    if not os.path.exists(SVCC):
        sys.exit("build svcc first: cargo build --release -p svcc")
    scav = a.scav_dir or os.path.join(out, "SCAV")
    if not os.path.isdir(scav):
        sh(["git", "clone", "--branch", "all", SCAV_URL, scav], timeout=600)
        if sh(["git", "checkout", "--quiet", SCAV_SHA], cwd=scav)[0] != 0:
            sys.exit("could not check out SCAV at " + SCAV_SHA)
    # svcc RTL output always includes <uvm>, CRAVE and FC4SC headers; give g++ stand-ins for
    # those three only, so that <systemc> still resolves to the real library.
    dv_inc = os.path.join(out, "dv_headers")
    os.makedirs(dv_inc, exist_ok=True)
    for f in ("uvm", "fc4sc.hpp"):
        shutil.copy(os.path.join(SHIMS, f), dv_inc)
    shutil.copytree(os.path.join(SHIMS, "crave2"), os.path.join(dv_inc, "crave2"), dirs_exist_ok=True)

    groups = {"probes": [], "corpus_rtl": [], "scav_own_examples": []}
    for d in ("svcc_probes", "svcc_corpus", "scav_probes", "scav_corpus", "scav_own", "corpus_v"):
        os.makedirs(os.path.join(out, d), exist_ok=True)
    for src in sorted(glob.glob(os.path.join(HERE, "probes", "*.v"))):
        groups["probes"].append({"design": os.path.basename(src),
                                 "svcc": run_svcc(src, os.path.join(out, "svcc_probes"), dv_inc),
                                 "scav": run_scav(src, scav, os.path.join(out, "scav_probes"))})
    for src in sorted(glob.glob(os.path.join(ROOT, "benchmarks", "corpus", "1[3-9]_*.sv")) + glob.glob(os.path.join(ROOT, "benchmarks", "corpus", "20_*.sv"))):
        v = os.path.join(out, "corpus_v", os.path.basename(src)[:-3] + ".v")
        to_verilog2005(src, v)
        groups["corpus_rtl"].append({"design": os.path.basename(src),
                                     "svcc": run_svcc(src, os.path.join(out, "svcc_corpus"), dv_inc),
                                     "scav": run_scav(v, scav, os.path.join(out, "scav_corpus"))})
    for n in SCAV_OWN:
        src = os.path.join(scav, "v2sc", "test", n + ".v")
        groups["scav_own_examples"].append({"design": n + ".v", "scav": run_scav(src, scav, os.path.join(out, "scav_own"))})

    summary = {"scav_commit": SCAV_SHA,
               "systemc": sh(["sh", "-c", "dpkg -s libsystemc-dev 2>/dev/null | grep '^Version' || echo unknown"])[1].strip(),
               "totals": {"probes": {"svcc": tally(groups["probes"], "svcc"), "scav": tally(groups["probes"], "scav")},
                          "corpus_rtl": {"svcc": tally(groups["corpus_rtl"], "svcc"), "scav": tally(groups["corpus_rtl"], "scav")},
                          "scav_own_examples": {"scav": tally(groups["scav_own_examples"], "scav")}},
               "designs": groups}
    res = os.path.join(ROOT, "benchmarks", "results", "scav_compare_summary.json")
    json.dump(summary, open(res, "w"), indent=2, sort_keys=True)
    open(res, "a").write("\n")
    for g, rows in groups.items():
        print("== " + g)
        for r in rows:
            s = r.get("svcc")
            sv = "svcc: roundtrip=%-4s cosim=%-8s | " % (s["roundtrip"], s["cosim"]) if s else ""
            c = r["scav"]
            print("  %-26s %sscav: exit=%s compiles=%s cosim=%s" % (r["design"], sv, c["translator_exit"], c["compiles_real_systemc"], c["cosim"]))
    print(json.dumps(summary["totals"], indent=2, sort_keys=True))
    print("wrote " + res)


if __name__ == "__main__":
    main()
