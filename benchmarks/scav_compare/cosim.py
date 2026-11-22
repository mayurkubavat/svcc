#!/usr/bin/env python3
"""cosim.py <verilog_ref> <top> <sc_header> <workdir> [extra g++ flags...]
Drives identical pseudo-random stimulus into the reference Verilog (Icarus) and the
generated SystemC module (real Accellera SystemC), prints per-cycle outputs, diffs them."""
import re, sys, os, random, subprocess


def bail(msg):
    """Harness could not drive this design (exit 4, distinct from a simulation mismatch)."""
    print('HARNESS: ' + msg)
    sys.exit(4)


ref, top, hdr, wd = sys.argv[1:5]; extra = sys.argv[5:]; 
wd = os.path.abspath(wd)
ref = os.path.abspath(ref)
os.makedirs(wd, exist_ok=True)
src = open(ref).read()
m = re.search(r'module\s+%s\b[^;]*?\((.*?)\)\s*;' % re.escape(top), src, re.S)
ports = []
for p in m.group(1).split(','):
    p = re.sub(r'//.*', '', p).strip()
    mm = re.match(r'(input|output)\s+(?:wire|reg|logic)?\s*(signed)?\s*(?:\[(\d+):(\d+)\])?\s*(\w+)$', p)
    if not mm:
        if not re.fullmatch(r'\w+', p): bail('cannot parse port: ' + p)
        d = re.search(r'(input|output)\s+(?:wire|reg)?\s*(signed)?\s*(?:\[(\d+):(\d+)\])?\s*(?:\w+\s*,\s*)*%s\b' % p, src)
        if not d: bail('cannot parse port: ' + p)
        ports.append((d.group(1), p, (int(d.group(3)) - int(d.group(4)) + 1) if d.group(3) else 1)); continue
    w = (int(mm.group(3)) - int(mm.group(4)) + 1) if mm.group(3) else 1
    ports.append((mm.group(1), mm.group(5), w))
ins = [(n, w) for d, n, w in ports if d == 'input']; outs = [(n, w) for d, n, w in ports if d == 'output']
clk = next((n for n, w in ins if n in ('clk', 'CLK')), None)
rst = next((n for n, w in ins if n in ('rst_n', 'rst', 'rstn', 'reset')), None)
rst_active = 0 if rst in ('rst_n', 'rstn') else 1
if top == 'even_divide3': rst_active = 0
data_ins = [(n, w) for n, w in ins if n not in (clk, rst)]
random.seed(1234); N = 400
vec = []
for c in range(N):
    row = {n: random.getrandbits(w) for n, w in data_ins}
    if rst: row[rst] = rst_active if c < 3 else 1 - rst_active
    vec.append(row)
drv = data_ins + ([(rst, 1)] if rst else [])
# ---- Verilog TB
tb = ['`timescale 1ns/1ps', 'module tb;']
for n, w in ins: tb.append('  reg [%d:0] %s;' % (w - 1, n))
for n, w in outs: tb.append('  wire [%d:0] %s;' % (w - 1, n))
tb.append('  %s dut(%s);' % (top, ', '.join('.%s(%s)' % (n, n) for _, n, _ in ports)))
tb.append('  initial begin')
if clk: tb.append('    %s = 0;' % clk)
for c, row in enumerate(vec):
    tb.append('    ' + ' '.join("%s = %d'h%x;" % (n, w, row[n]) for n, w in drv))
    tb.append('    #4;')
    if c >= 3: tb.append('    $display("%s", %s);' % (' '.join(['%h'] * len(outs)), ', '.join(n for n, _ in outs)))
    tb.append('    #1;' + (' %s = 1; #5; %s = 0;' % (clk, clk) if clk else ' #5;'))
tb += ['    $finish;', '  end', 'endmodule']
open(wd + '/tb.v', 'w').write('\n'.join(tb))
# ---- SystemC TB: signal types taken from the generated header's own port declarations
h = open(hdr).read()
sig = {}
for n, w in ins + outs:
    mm = re.search(r'sc_(?:in|out|inout)\s*<\s*(.*?)\s*>\s*%s\b' % n, h)
    if not mm: bail('port %s not found in generated module' % n)
    sig[n] = mm.group(1).strip()
sc = ['#include <systemc>', 'using namespace sc_core; using namespace sc_dt;', '#include "%s"' % os.path.abspath(hdr), '#include <iostream>', '#include <iomanip>', '#include <sstream>', '#include <type_traits>',
      'std::string fx(unsigned long long v,int w){ if(w<64) v&=((1ULL<<w)-1); std::ostringstream o; o<<std::hex<<std::setw((w+3)/4)<<std::setfill(\'0\')<<v; return o.str(); }',
      'template<int W> std::string hx(const sc_uint<W>& v,int w){ return fx(v.to_uint64(),w); }',
      'template<int W> std::string hx(const sc_int<W>& v,int w){ return fx((unsigned long long)v.to_int64(),w); }',
      'template<int W> std::string hx(const sc_biguint<W>& v,int w){ std::string s; for(int i=((w+3)/4)*4-1;i>=0;i-=4){ int d=0; for(int k=0;k<4;k++){ int bi=i-k; if(bi<w && v[bi].to_bool()) d|=1<<(3-k);} s+="0123456789abcdef"[d]; } return s; }',
      'template<class T, class=typename std::enable_if<std::is_integral<T>::value>::type> std::string hx(T v,int w){ return fx((unsigned long long)v,w); }',
      'std::string hx(bool v,int){ return v?"1":"0"; }',
      'int sc_main(int, char**) {']
for n, w in ins + outs: sc.append('  sc_signal<%s> %s("%s");' % (sig[n], n, n))
sc.append('  %s dut("dut");' % top)
for n, w in ins + outs: sc.append('  dut.%s(%s);' % (n, n))
if clk: sc.append('  %s = 0;' % clk)
def lit(n, w, v):
    return '"0x%x"' % v if w > 64 else '%dULL' % v
for c, row in enumerate(vec):
    sc.append('  ' + ' '.join('%s = %s;' % (n, ('(%s)' % lit(n, w, row[n])) if 'bool' != sig[n] else str(row[n] & 1)) for n, w in drv))
    sc.append('  sc_start(4, SC_NS);')
    if c >= 3: sc.append('  std::cout << %s << std::endl;' % ' << " " << '.join('hx(%s.read(), %d)' % (n, w) for n, w in outs))
    sc.append('  sc_start(1, SC_NS);' + (' %s = 1; sc_start(5, SC_NS); %s = 0;' % (clk, clk) if clk else ' sc_start(5, SC_NS);'))
sc += ['  return 0;', '}']
open(wd + '/main.cpp', 'w').write('\n'.join(sc))
r = subprocess.run(['iverilog', '-g2012', '-o', wd + '/ref.vvp', wd + '/tb.v', ref], capture_output=True, text=True)
if r.returncode: bail('reference build failed ' + r.stderr[:300])
vo = subprocess.run(['vvp', '-n', wd + '/ref.vvp'], capture_output=True, text=True).stdout.strip().split('\n'); vo = [l for l in vo if re.fullmatch(r'[0-9a-fxz ]+', l)]
r = subprocess.run(['g++', '-std=c++17', '-O0', '-w', '-o', wd + '/sim', wd + '/main.cpp'] + extra + ['-lsystemc'], capture_output=True, text=True, cwd=os.path.dirname(os.path.abspath(hdr)))
if r.returncode: print('SC_BUILD_FAIL', r.stderr.split('\n')[0][:200]); sys.exit(2)
try:
    so = subprocess.run([wd + '/sim'], capture_output=True, text=True, timeout=120)
except subprocess.TimeoutExpired:
    print('SC_SIM_TIMEOUT (no convergence)'); sys.exit(3)
sl = [l for l in so.stdout.split('\n') if re.fullmatch(r'[0-9a-fxz ]+', l)]
mis = sum(1 for a, b in zip(vo, sl) if a.strip() != b.strip()) + abs(len(vo) - len(sl))
print('cycles=%d mismatches=%d' % (len(vo), mis) + ('' if not mis else '  first: ref=%s sc=%s' % next(((a, b) for a, b in zip(vo, sl) if a.strip() != b.strip()), ('len', 'len'))))
sys.exit(0 if mis == 0 else 1)
