#!/usr/bin/env python3
"""Third-simulator adjudication of an svcc rtl-cosim mismatch.

Replays the exact stimulus of a failing rtl-cosim run (same tb.cpp, same seed) with a trace
of every input and of both models' outputs at each settle point, then drives the same input
sequence into Icarus Verilog and compares Icarus's outputs with SystemC and Verilator.

With --via-yosys the design is elaborated by yosys-slang into a netlist first, for sources
Icarus cannot parse. Icarus is 4-state: an x in its output marks an output the source leaves
unspecified for that input (out-of-range select, undefined case), where both 2-state models
are free to pick any value.

usage: adjudicate.py --census-tool T --corpus C --design D --module M --work OLD_WORKDIR --out DIR
                     [--define VERILATOR] [--via-yosys] [--tools DIR]
"""
import argparse, os, re, shutil, subprocess, sys

ap = argparse.ArgumentParser()
for a in ("census-tool", "corpus", "design", "module", "work", "out"):
    ap.add_argument("--" + a, required=True)
ap.add_argument("--tools", default=os.environ.get("ADJ_TOOLS", ""),
                help="directory with iverilog, vvp and yosys (with the slang plugin); default: PATH")
ap.add_argument("--define", nargs="*", default=[], help="extra defines for Icarus (e.g. VERILATOR)")
ap.add_argument("--via-yosys", action="store_true", help="elaborate with yosys-slang to a netlist, then simulate that netlist in Icarus")
o = ap.parse_args()
IV = o.tools or os.path.dirname(shutil.which("iverilog") or "/usr/bin/iverilog")

sys.path.insert(0, o.census_tool)
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import census, run_stress  # noqa: E402

entry = next(e for e in census.load_corpus() if e["name"] == o.design)
design = census.Design(entry, o.corpus)
unit = next((n, p) for n, p in design.units() if n == o.module)
files, dargs = run_stress.design_args(design, o.module, design.closure(unit[1]))

# 1. traced replay of the original harness
out = os.path.abspath(o.out)
shutil.rmtree(out, ignore_errors=True)
shutil.copytree(o.work, out)
old = os.path.abspath(o.work)
for root, _, fs in os.walk(os.path.join(out, "obj")):
    for f in fs:
        if f.endswith((".mk", ".d")):
            p = os.path.join(root, f)
            s = open(p, errors="replace").read()
            open(p, "w").write(s.replace(old, out))
for f in ("tb.o", "cosim", "Vref"):
    for d in (out, os.path.join(out, "obj")):
        try:
            os.remove(os.path.join(d, f))
        except OSError:
            pass
tb = open(os.path.join(out, "tb.cpp")).read()
sigs = re.findall(r"sc_core::sc_signal<(.+?)> p_(\w+)\{", tb)
width = {}
for t, n in sigs:
    m = re.search(r"<(\d+)>", t)
    width[n] = int(m.group(1)) if m else 1
inputs = re.findall(r"auto set_(\w+) = \[", tb)
outputs = list(dict.fromkeys(re.findall(r"output (\w+)(?: word %u)?: systemc", tb)))

def sc_hex(n):
    if width[n] == 1 and "bool" in dict((b, a) for a, b in sigs)[n]:
        return f'std::printf(" %x", (unsigned)p_{n}.read());'
    return f'std::printf(" %s", p_{n}.read().to_string(sc_dt::SC_HEX_US, false).c_str());'

def vl_hex(n):
    w = width[n]
    if w <= 64:
        return f'std::printf(" %llx", (unsigned long long)ref->{n});'
    words = (w + 31) // 32
    return (f'std::printf(" "); for (int i = {words - 1}; i >= 0; --i) std::printf(i == {words - 1} ? "%x" : "%08x", (unsigned)ref->{n}[i]);')

trace = ["auto trace_step = [&]() {", " std::printf(\"T\");"]
trace += ["  " + sc_hex(n) for n in inputs]
trace += ["  std::printf(\" |\");"] + ["  " + sc_hex(n) for n in outputs]
trace += ["  std::printf(\" |\");"] + ["  " + vl_hex(n) for n in outputs]
trace += ["  std::printf(\"\\n\");", "};"]
tb = tb.replace("auto settle = [&]() { sc_core::sc_start(1, sc_core::SC_NS); ref->eval(); };",
                "\n".join(trace) + "\n  auto settle = [&]() { sc_core::sc_start(1, sc_core::SC_NS); ref->eval(); trace_step(); };")
tb = tb.replace("std::printf(\"MISMATCH", "std::fprintf(stderr, \"MISMATCH")
open(os.path.join(out, "tb.cpp"), "w").write(tb)
env = dict(os.environ)
r = subprocess.run(["make", "-C", "obj", "-f", "Vref.mk", "-j", "4"], cwd=out, capture_output=True, text=True, env=env)
if r.returncode:
    sys.exit("trace build failed:\n" + r.stderr[-3000:])
exe = next(os.path.join(out, "obj", f) for f in ("Vref", "cosim") if os.path.exists(os.path.join(out, "obj", f)))
r = subprocess.run([exe], cwd=out, capture_output=True, text=True)
rows = [l.split()[1:] for l in r.stdout.splitlines() if l.startswith("T ")]
mism = [l for l in r.stderr.splitlines() if l.startswith("MISMATCH")]
open(os.path.join(out, "trace.txt"), "w").write(r.stdout)

# 2. Icarus Verilog replay of the same input sequence
ni, no = len(inputs), len(outputs)
clocks = [n for n in inputs if width[n] == 1 and re.search(r"cl(oc)?k", n, re.I)]
with open(os.path.join(out, "stim.hex"), "w") as f:
    for row in rows:
        f.write(" ".join(row[:ni]) + "\n")
params = [p.split("=", 1) for i, p in enumerate(dargs) if i and dargs[i - 1] == "-G"]
plist = ", ".join(f".{k}({v})" for k, v in params)
v = ["`timescale 1ns/1ps", "module adj_tb;"]
v += [f"  reg [{width[n]-1}:0] {n};" for n in inputs]
v += [f"  wire [{width[n]-1}:0] {n};" for n in outputs]
v += [f"  {o.module} {'#(' + plist + ') ' if plist else ''}dut (" + ", ".join(f".{n}({n})" for n in inputs + outputs) + ");"]
v += ["  integer fd, c, k;", f"  reg [{max(width[n] for n in inputs)-1}:0] tmp [0:{ni-1}];",
      "  initial begin", "    fd = $fopen(\"stim.hex\", \"r\"); k = 0;",
      "    while (!$feof(fd)) begin"]
v += [f"      c = $fscanf(fd, \"%h\", tmp[{i}]);" for i in range(ni)]
v += ["      if (c == 1) begin"]
v += [f"        {n} = tmp[{i}];" for i, n in enumerate(inputs) if n not in clocks]
v += ["        #1;"] + [f"        {n} = tmp[{inputs.index(n)}];" for n in clocks] + ["        #1;"]
v += ["        $write(\"I %0d\", k);"] + [f"        $write(\" %h\", {n});" for n in outputs]
v += ["        $write(\"\\n\"); k = k + 1;", "      end", "    end", "    $finish;", "  end", "endmodule"]
open(os.path.join(out, "adj_tb.sv"), "w").write("\n".join(v) + "\n")
iargs = []
for a in dargs:
    if a.startswith("+incdir+"):
        iargs.append("-I" + a[8:])
    elif a.startswith("+define+"):
        iargs.append("-D" + a[8:])
iargs += ["-D" + d for d in o.define]
srcs = files
if o.via_yosys:
    ys = " ".join(iargs) + " " + " ".join(f"-G {k}={v}" for k, v in params) + " --top " + o.module + " " + " ".join(files)
    r = subprocess.run([f"{IV}/yosys", "-q", "-m", "slang", "-p",
                        f"read_slang {ys}; hierarchy -top {o.module}; proc; flatten; opt_clean; write_verilog -noattr netlist.v"],
                       cwd=out, capture_output=True, text=True)
    if r.returncode:
        print("YOSYS FAILED:\n" + (r.stdout + r.stderr)[-4000:]); sys.exit(3)
    srcs, iargs = ["netlist.v"], []
r = subprocess.run([f"{IV}/iverilog", "-g2012", "-o", "adj.vvp", "-s", "adj_tb", *iargs, *srcs, "adj_tb.sv"],
                   cwd=out, capture_output=True, text=True)
if r.returncode:
    print("ICARUS COMPILE FAILED:\n" + r.stderr[-4000:])
    print("\n".join(mism[:4]))
    sys.exit(3)
r = subprocess.run([f"{IV}/vvp", "-n", "adj.vvp"], cwd=out, capture_output=True, text=True, timeout=600)
iv = [l.split()[2:] for l in r.stdout.splitlines() if l.startswith("I ")]

def norm(x):
    x = x.lower().lstrip("0")
    return x or "0"

print(f"{o.design}/{o.module}: {len(rows)} settle points, outputs {outputs}")
print("first original mismatches:", *mism[:3], sep="\n  ")
agree = {"sc": 0, "vl": 0, "neither": 0}
shown = 0
for k, (row, ivr) in enumerate(zip(rows, iv)):
    sc, vl = row[ni + 1:ni + 1 + no], row[ni + 2 + no:]
    for j, n in enumerate(outputs):
        a, b, c = norm(sc[j]), norm(vl[j]), norm(ivr[j])
        if a != b or "x" in c or "z" in c:
            who = "sc" if c == a else "vl" if c == b else "neither"
            if a != b:
                agree[who] += 1
            if shown < 8:
                print(f"  settle {k} {n}: systemc={a} verilator={b} icarus={c} -> icarus agrees with {who}")
                shown += 1
print("verdict counts over differing outputs:", agree)
