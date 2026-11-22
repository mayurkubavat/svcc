//! Differential simulation of generated SystemC against Verilator.
//!
//! The same SystemVerilog source is (a) converted to SystemC by `svcc` and (b) compiled to a
//! cycle-accurate C++ model by Verilator. A generated harness instantiates both in one
//! executable, drives every input of both models with the same random stimulus, and compares
//! every output after each step. Both models are two-state (`--x-assign 0 --x-initial 0`).
//!
//! Stimulus keeps data and edge events apart: one step changes data inputs, the next changes
//! exactly one clock or asynchronous control, so the result never depends on how a simulator
//! orders a clock edge against a simultaneous data change.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use svcc_emit_sc::hdl_emit::{class_cname, cname};
use svcc_ir::hdl::{Design, Dir, Module, Port, ProcKind};
use svcc_ir::SvccError;

const STAGE: &str = "svcc/cosim";

pub struct CosimOptions {
    /// Extra design arguments after the first source file: more sources, `-f`, `+incdir+`,
    /// `+define+`, `-G name=value`.
    pub design_args: Vec<String>,
    pub top: Option<String>,
    pub steps: u64,
    pub seed: u64,
    /// Directory to build in; kept afterwards. A temporary directory is used when `None`.
    pub work_dir: Option<PathBuf>,
    /// Simulate this header instead of the one generated now (a hand-edited or mutated
    /// copy, to see what the simulation says about it).
    pub header: Option<String>,
}

pub struct CosimReport {
    pub top: String,
    pub steps: u64,
    pub comparisons: u64,
    pub outputs: usize,
    pub work_dir: PathBuf,
}

/// How a top-level input takes part in edge events.
#[derive(Clone, Copy, PartialEq, Eq)]
struct EdgeUse {
    /// Read by the process body as well (asynchronous set or reset).
    async_ctrl: bool,
    /// Sensitive edge is the rising one.
    posedge: bool,
}

/// What the harness needs to know about a module's clocking.
#[derive(Clone, Default)]
struct ClockInfo {
    /// Input ports that reach an edge sensitivity, directly or through combinational logic
    /// (clock gates, muxes, buffers) and instance boundaries.
    edges: BTreeMap<String, EdgeUse>,
    /// For each output port, the input ports it depends on combinationally.
    out_deps: BTreeMap<String, BTreeSet<String>>,
}

fn clock_info(design: &Design, m: &Module, memo: &mut BTreeMap<String, ClockInfo>) {
    if memo.contains_key(&m.name) {
        return;
    }
    // Combinational dependencies between the signals of this module.
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut direct: BTreeMap<String, EdgeUse> = BTreeMap::new();
    let add = |sig: &str, u: EdgeUse, direct: &mut BTreeMap<String, EdgeUse>| {
        direct
            .entry(sig.to_string())
            .and_modify(|e| e.async_ctrl |= u.async_ctrl)
            .or_insert(u);
    };
    for p in &m.processes {
        match &p.kind {
            ProcKind::Seq { edges } => {
                for e in edges {
                    add(
                        &e.sig,
                        EdgeUse {
                            async_ctrl: e.is_async_ctrl,
                            posedge: e.posedge,
                        },
                        &mut direct,
                    );
                }
            }
            ProcKind::Comb => {
                let (mut r, mut w) = (BTreeSet::new(), BTreeSet::new());
                m.reads(&p.body, &mut r);
                Module::writes(&p.body, &mut w);
                for t in w {
                    deps.entry(t).or_default().extend(r.iter().cloned());
                }
            }
        }
    }
    for inst in &m.instances {
        let Some(child) = design.module(&inst.module) else {
            continue;
        };
        clock_info(design, child, memo);
        let info = memo.get(&child.name).cloned().unwrap_or_default();
        let actual = |port: &str| {
            inst.conns
                .iter()
                .find(|c| c.port == port)
                .map(|c| c.sig.clone())
        };
        for c in &inst.conns {
            if let Some(u) = info.edges.get(&c.port) {
                add(&c.sig, *u, &mut direct);
            }
            if let Some(ins) = info.out_deps.get(&c.port) {
                let srcs: Vec<String> = ins.iter().filter_map(|i| actual(i)).collect();
                deps.entry(c.sig.clone()).or_default().extend(srcs);
            }
        }
    }
    let closure = |start: &str| -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![start.to_string()];
        while let Some(s) = todo.pop() {
            for d in deps.get(&s).into_iter().flatten() {
                if seen.insert(d.clone()) {
                    todo.push(d.clone());
                }
            }
        }
        seen
    };
    let is_input = |n: &str| m.ports.iter().any(|p| p.name == n && p.dir == Dir::In);
    let mut info = ClockInfo::default();
    for (sig, u) in &direct {
        if is_input(sig) {
            add(sig, *u, &mut info.edges);
        }
        // Whatever feeds an edge through logic is toggled like a clock, alone in its step.
        for src in closure(sig) {
            if is_input(&src) {
                add(
                    &src,
                    EdgeUse {
                        async_ctrl: false,
                        posedge: true,
                    },
                    &mut info.edges,
                );
            }
        }
    }
    for p in m.ports.iter().filter(|p| p.dir == Dir::Out) {
        let ins: BTreeSet<String> = closure(&p.name)
            .into_iter()
            .filter(|s| is_input(s))
            .collect();
        info.out_deps.insert(p.name.clone(), ins);
    }
    memo.insert(m.name.clone(), info);
}

fn sc_type(p: &Port) -> String {
    if p.scalar && p.ty.w == 1 && !p.ty.s {
        return "bool".into();
    }
    match (p.ty.w > 64, p.ty.s) {
        (false, false) => format!("sc_dt::sc_uint<{}>", p.ty.w),
        (false, true) => format!("sc_dt::sc_int<{}>", p.ty.w),
        (true, false) => format!("sc_dt::sc_biguint<{}>", p.ty.w),
        (true, true) => format!("sc_dt::sc_bigint<{}>", p.ty.w),
    }
}

/// Generates the harness source for `top`.
pub fn harness_source(
    design: &Design,
    top: &Module,
    dut_header: &str,
    opts: &CosimOptions,
) -> String {
    let mut memo = BTreeMap::new();
    clock_info(design, top, &mut memo);
    // Only 1-bit scalar inputs can be driven as clocks; anything wider stays a data input.
    let edges: BTreeMap<String, EdgeUse> = memo
        .get(&top.name)
        .map(|i| i.edges.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|(n, _)| {
            top.ports
                .iter()
                .any(|p| p.name == *n && p.ty.w == 1 && p.dims.is_empty() && sc_type(p) == "bool")
        })
        .collect();

    let mut s = String::new();
    let _ = writeln!(s, "// Generated by svcc rtl-cosim. Do not edit.");
    let _ = writeln!(
        s,
        "#include <cstdint>\n#include <cstdio>\n#include <systemc>"
    );
    let _ = writeln!(s, "#include \"{dut_header}\"");
    let _ = writeln!(s, "#include \"Vref.h\"\n#include \"verilated.h\"\n");
    let _ = writeln!(s, "static uint64_t rng_state = {}ULL;", opts.seed | 1);
    s.push_str(
        "static uint64_t rnd() {\n  rng_state ^= rng_state >> 12; rng_state ^= rng_state << 25; rng_state ^= rng_state >> 27;\n  return rng_state * 0x2545F4914F6CDD1DULL;\n}\n",
    );
    s.push_str(
        "// Random value with corner cases (0, 1, all ones, top bit) over-represented.\nstatic uint64_t rnd_val(unsigned w) {\n  const uint64_t m = w >= 64 ? ~0ULL : ((1ULL << w) - 1);\n  switch (rnd() % 16) {\n    case 0: return 0;\n    case 1: return m;\n    case 2: return 1 & m;\n    case 3: return (1ULL << (w - 1)) & m;\n    case 4: return rnd() & 0xf & m;\n    default: return rnd() & m;\n  }\n}\n\n",
    );
    s.push_str("int sc_main(int argc, char** argv) {\n  Verilated::commandArgs(argc, argv);\n");
    s.push_str(
        "  sc_core::sc_report_handler::set_actions(sc_core::SC_INFO, sc_core::SC_DO_NOTHING);\n",
    );
    s.push_str(
        "  sc_core::sc_report_handler::set_actions(sc_core::SC_WARNING, sc_core::SC_DO_NOTHING);\n",
    );
    for p in &top.ports {
        if p.dims.is_empty() {
            let _ = writeln!(
                s,
                "  sc_core::sc_signal<{}> p_{}{{\"p_{}\"}};",
                sc_type(p),
                p.name,
                p.name
            );
        } else {
            let n: u64 = p.dims.iter().map(|d| *d as u64).product();
            let _ = writeln!(
                s,
                "  sc_core::sc_vector<sc_core::sc_signal<{}>> p_{}{{\"p_{}\", {}}};",
                sc_type(p),
                p.name,
                p.name,
                n
            );
        }
    }
    let _ = writeln!(s, "  {} dut{{\"dut\"}};", class_cname(&top.name));
    for p in &top.ports {
        let _ = writeln!(s, "  dut.{}(p_{});", cname(&p.name), p.name);
    }
    s.push_str("  Vref* ref = new Vref;\n");
    s.push_str("  unsigned long long comparisons = 0;\n  bool ok = true;\n\n");

    // SystemC-side and Verilator-side expression of a port (element `k` of an array port).
    let leaf = |p: &Port| -> (String, String) {
        if p.dims.is_empty() {
            return (format!("p_{}", p.name), format!("ref->{}", p.name));
        }
        let mut idx = String::new();
        for (i, _) in p.dims.iter().enumerate() {
            let below: u64 = p.dims[i + 1..].iter().map(|d| *d as u64).product();
            idx.push_str(&format!("[(k / {below}) % {}]", p.dims[i]));
        }
        (format!("p_{}[k]", p.name), format!("ref->{}{idx}", p.name))
    };

    // Setters.
    for p in top.ports.iter().filter(|p| p.dir == Dir::In) {
        let n = &p.name;
        let w = p.ty.w;
        let (ps, rs) = leaf(p);
        if p.dims.is_empty() {
            let _ = writeln!(s, "  auto set_{n} = [&](uint64_t v) {{");
        } else {
            let _ = writeln!(s, "  auto set_{n} = [&](unsigned k, uint64_t v) {{");
        }
        if w <= 64 {
            if sc_type(p) == "bool" {
                let _ = writeln!(s, "    {ps}.write(v & 1); {rs} = v & 1;");
            } else {
                let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                let _ = writeln!(s, "    v &= 0x{mask:x}ULL;");
                let _ = writeln!(s, "    {ps}.write({}(v)); {rs} = v;", sc_type(p));
            }
        } else {
            let words = w.div_ceil(32);
            let _ = writeln!(s, "    sc_dt::sc_biguint<{w}> b;");
            let _ = writeln!(s, "    for (unsigned i = 0; i < {words}; ++i) {{");
            let _ = writeln!(
                s,
                "      const unsigned hi = (32 * i + 31 < {w}) ? 32 * i + 31 : {};",
                w - 1
            );
            let _ = writeln!(
                s,
                "      uint64_t word = (v == 0 || v == 1) ? (i == 0 ? v : 0) : (v == ~0ULL ? 0xffffffffULL : (rnd() & 0xffffffffULL));"
            );
            let _ = writeln!(
                s,
                "      word &= (hi - 32 * i == 31) ? 0xffffffffULL : ((1ULL << (hi - 32 * i + 1)) - 1);"
            );
            let _ = writeln!(
                s,
                "      b.range(hi, 32 * i) = word; {rs}[i] = (uint32_t)word;"
            );
            let _ = writeln!(s, "    }}");
            if p.ty.s {
                let _ = writeln!(s, "    {ps}.write(sc_dt::sc_bigint<{w}>(b));");
            } else {
                let _ = writeln!(s, "    {ps}.write(b);");
            }
        }
        let _ = writeln!(s, "  }};");
    }

    // Output comparison.
    s.push_str("\n  auto check = [&](unsigned long long step, const char* phase) {\n");
    for p in top.ports.iter().filter(|p| p.dir == Dir::Out) {
        let n = &p.name;
        let (ps, rs) = leaf(p);
        let elems: u64 = p.dims.iter().map(|d| *d as u64).product();
        if !p.dims.is_empty() {
            let _ = writeln!(s, "    for (unsigned k = 0; k < {elems}; ++k) {{");
        }
        let w = p.ty.w;
        if w <= 64 {
            let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            let read = if sc_type(p) == "bool" {
                format!("(uint64_t){ps}.read()")
            } else {
                format!("{ps}.read().to_uint64()")
            };
            let _ = writeln!(s, "    {{");
            let _ = writeln!(
                s,
                "      const uint64_t a = {read} & 0x{mask:x}ULL, b = (uint64_t){rs} & 0x{mask:x}ULL;"
            );
            let _ = writeln!(s, "      ++comparisons;");
            let _ = writeln!(
                s,
                "      if (a != b) {{ std::printf(\"MISMATCH step %llu (%s) output {n}: systemc=0x%llx verilator=0x%llx\\n\", step, phase, (unsigned long long)a, (unsigned long long)b); ok = false; }}"
            );
            let _ = writeln!(s, "    }}");
        } else {
            let words = w.div_ceil(32);
            let _ = writeln!(s, "    for (unsigned i = 0; i < {words}; ++i) {{");
            let _ = writeln!(
                s,
                "      const unsigned hi = (32 * i + 31 < {w}) ? 32 * i + 31 : {};",
                w - 1
            );
            let _ = writeln!(
                s,
                "      const uint64_t a = sc_dt::sc_biguint<{w}>({ps}.read()).range(hi, 32 * i).to_uint64(), b = (uint32_t){rs}[i];"
            );
            let _ = writeln!(s, "      ++comparisons;");
            let _ = writeln!(
                s,
                "      if (a != b) {{ std::printf(\"MISMATCH step %llu (%s) output {n} word %u: systemc=0x%llx verilator=0x%llx\\n\", step, phase, i, (unsigned long long)a, (unsigned long long)b); ok = false; }}"
            );
            let _ = writeln!(s, "    }}");
        }
        if !p.dims.is_empty() {
            let _ = writeln!(s, "    }}");
        }
    }
    s.push_str("  };\n");
    s.push_str("  auto settle = [&]() { sc_core::sc_start(1, sc_core::SC_NS); ref->eval(); };\n\n");

    let data: Vec<&Port> = top
        .ports
        .iter()
        .filter(|p| p.dir == Dir::In && !edges.contains_key(&p.name))
        .collect();
    let clocks: Vec<&Port> = top
        .ports
        .iter()
        .filter(|p| p.dir == Dir::In && edges.get(&p.name).is_some_and(|e| !e.async_ctrl))
        .collect();
    let ctrls: Vec<(&Port, EdgeUse)> = top
        .ports
        .iter()
        .filter_map(|p| {
            edges
                .get(&p.name)
                .filter(|e| e.async_ctrl && p.dir == Dir::In)
                .map(|e| (p, *e))
        })
        .collect();

    // Initial state: clocks low, asynchronous controls inactive, data zero. Controls are
    // asserted at step 1 and released at step 3, so the first reset is a real edge in both
    // simulators rather than a level present at time zero.
    for p in &data {
        if p.dims.is_empty() {
            let _ = writeln!(s, "  set_{}(0);", p.name);
        } else {
            let n: u64 = p.dims.iter().map(|d| *d as u64).product();
            let _ = writeln!(
                s,
                "  for (unsigned k = 0; k < {n}; ++k) set_{}(k, 0);",
                p.name
            );
        }
    }
    for p in &clocks {
        let _ = writeln!(s, "  bool cur_{0} = false; set_{0}(0);", p.name);
    }
    for (p, e) in &ctrls {
        let active = if e.posedge { 1 } else { 0 };
        let _ = writeln!(
            s,
            "  bool cur_{0} = {1}; const bool home_{0} = {1}; set_{0}({1});",
            p.name,
            1 - active
        );
    }
    // Per-input bias for 1-bit data inputs, redrawn now and then, so enables and
    // synchronous resets spend time both mostly on and mostly off.
    let _ = writeln!(s, "  unsigned bias[{}] = {{}};", data.len().max(1));
    s.push_str("  settle();\n  check(0, \"init\");\n\n");
    let _ = writeln!(s, "  const unsigned long long STEPS = {}ULL;", opts.steps);
    s.push_str("  for (unsigned long long step = 1; step <= STEPS && ok; ++step) {\n");
    let _ = writeln!(
        s,
        "    if (step % 64 == 1) for (unsigned i = 0; i < {}; ++i) bias[i] = rnd() % 3;",
        data.len().max(1)
    );
    for (i, p) in data.iter().enumerate() {
        if !p.dims.is_empty() {
            let n: u64 = p.dims.iter().map(|d| *d as u64).product();
            let v = if p.ty.w <= 64 {
                format!("rnd_val({})", p.ty.w)
            } else {
                "(rnd() % 16 == 0 ? 0 : 2)".to_string()
            };
            let _ = writeln!(
                s,
                "    for (unsigned k = 0; k < {n}; ++k) if (rnd() % 4) set_{}(k, {v});",
                p.name
            );
        } else if p.ty.w == 1 {
            let _ = writeln!(
                s,
                "    if (rnd() % 4) set_{}(bias[{i}] == 0 ? (rnd() % 8 == 0) : bias[{i}] == 1 ? (rnd() & 1) : (rnd() % 8 != 0));",
                p.name
            );
        } else if p.ty.w <= 64 {
            let _ = writeln!(s, "    if (rnd() % 4) set_{}(rnd_val({}));", p.name, p.ty.w);
        } else {
            let _ = writeln!(
                s,
                "    if (rnd() % 4) {{ const unsigned k = rnd() % 16; set_{}(k == 0 ? 0 : k == 1 ? 1 : k == 2 ? ~0ULL : 2); }}",
                p.name
            );
        }
    }
    s.push_str("    settle();\n    check(step, \"data\");\n");
    for (p, _) in &ctrls {
        let n = &p.name;
        let _ = writeln!(
            s,
            "    if (step == 1 ? true : step == 2 ? false : step == 3 ? cur_{n} != home_{n} : (cur_{n} == home_{n} ? rnd() % 48 == 0 : rnd() % 3 == 0)) {{ cur_{n} = !cur_{n}; set_{n}(cur_{n}); settle(); check(step, \"async {n}\"); }}"
        );
    }
    if !clocks.is_empty() {
        let _ = writeln!(s, "    switch (rnd() % {}) {{", clocks.len());
        for (i, p) in clocks.iter().enumerate() {
            let n = &p.name;
            let _ = writeln!(
                s,
                "      case {i}: cur_{n} = !cur_{n}; set_{n}(cur_{n}); settle(); check(step, \"clock {n}\"); break;"
            );
        }
        s.push_str("    }\n");
    }
    s.push_str("  }\n");
    s.push_str("  ref->final();\n  delete ref;\n");
    let _ = writeln!(
        s,
        "  if (ok) std::printf(\"COSIM PASS: {} outputs, %llu steps, %llu comparisons\\n\", STEPS, comparisons);",
        top.ports.iter().filter(|p| p.dir == Dir::Out).count()
    );
    s.push_str("  else std::printf(\"COSIM FAIL\\n\");\n  return ok ? 0 : 1;\n}\n");
    s
}

fn systemc_home() -> Result<PathBuf, SvccError> {
    if let Ok(h) = std::env::var("SYSTEMC_HOME") {
        return Ok(PathBuf::from(h));
    }
    if let Ok(home) = std::env::var("HOME") {
        for prefix in [
            ".local/opt/accellera-svcc/systemc",
            ".local/opt/accellera/systemc",
        ] {
            let p = Path::new(&home).join(prefix);
            if p.join("include/systemc").exists() {
                return Ok(p);
            }
        }
    }
    Err(SvccError::new(
        STAGE,
        "SystemC not found: set SYSTEMC_HOME to an Accellera SystemC install",
    ))
}

/// Translates design arguments to Verilator's spelling (`-G name=value` -> `-Gname=value`,
/// relative paths made absolute because Verilator runs from another directory).
fn verilator_design_args(args: &[String]) -> Vec<String> {
    let abs = |p: &str| {
        Path::new(p)
            .canonicalize()
            .map(|x| x.display().to_string())
            .unwrap_or_else(|_| p.to_string())
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-G" && i + 1 < args.len() {
            out.push(format!("-G{}", args[i + 1]));
            i += 2;
        } else if a == "-f" && i + 1 < args.len() {
            out.push("-f".into());
            out.push(abs(&args[i + 1]));
            i += 2;
        } else if let Some(d) = a.strip_prefix("+incdir+") {
            out.push(format!("+incdir+{}", abs(d)));
            i += 1;
        } else if a.starts_with('+') || a.starts_with('-') {
            out.push(a.clone());
            i += 1;
        } else {
            out.push(abs(a));
            i += 1;
        }
    }
    out
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Converts `sv_path`, builds the two models and runs the comparison.
pub fn run_cosim(sv_path: &str, opts: &CosimOptions) -> Result<CosimReport, SvccError> {
    let (code, design) = if opts.design_args.is_empty() {
        let src = std::fs::read_to_string(sv_path)
            .map_err(|e| SvccError::new("io", format!("cannot read {sv_path}: {e}")))?;
        let tops: Vec<String> = opts.top.iter().cloned().collect();
        crate::sv_rtl_to_sc(sv_path, &src, &tops)?
    } else {
        let mut args = vec![sv_path.to_string()];
        args.extend(opts.design_args.iter().cloned());
        if let Some(t) = &opts.top {
            args.push("-top".into());
            args.push(t.clone());
        }
        crate::sv_rtl_args_to_sc(&args)?
    };
    let code = opts.header.clone().unwrap_or(code);
    let top_name = match &opts.top {
        Some(t) => design
            .modules
            .iter()
            .rev()
            .find(|m| m.definition == *t)
            .map(|m| m.name.clone())
            .ok_or_else(|| SvccError::new(STAGE, format!("top module '{t}' not found")))?,
        None => design
            .tops
            .last()
            .cloned()
            .ok_or_else(|| SvccError::new(STAGE, "no top module"))?,
    };
    let top = design.module(&top_name).expect("top module");

    let work = match &opts.work_dir {
        Some(d) => d.clone(),
        None => std::env::temp_dir().join(format!(
            "svcc-cosim-{}-{}",
            top.definition,
            std::process::id()
        )),
    };
    std::fs::create_dir_all(&work)
        .map_err(|e| SvccError::new("io", format!("cannot create {}: {e}", work.display())))?;
    let work = work
        .canonicalize()
        .map_err(|e| SvccError::new("io", e.to_string()))?;
    let write = |name: &str, text: &str| {
        std::fs::write(work.join(name), text)
            .map_err(|e| SvccError::new("io", format!("cannot write {name}: {e}")))
    };
    write("dut.hpp", &code)?;
    write("tb.cpp", &harness_source(&design, top, "dut.hpp", opts))?;

    let sc = systemc_home()?;
    let lib = ["lib", "lib-linux64", "lib64"]
        .iter()
        .map(|l| sc.join(l))
        .find(|p| {
            ["libsystemc.so", "libsystemc.dylib", "libsystemc.a"]
                .iter()
                .any(|f| p.join(f).exists())
        })
        .ok_or_else(|| SvccError::new(STAGE, "libsystemc not found under SYSTEMC_HOME"))?;
    let sv_abs = Path::new(sv_path)
        .canonicalize()
        .map_err(|e| SvccError::new("io", e.to_string()))?;
    let obj = work.join("obj");
    let out = Command::new("verilator")
        .args(["--cc", "--exe", "--build", "-j", "4", "--prefix", "Vref"])
        .args(["--top-module", &top.definition])
        .args(["-Wno-fatal", "-Wno-lint", "-Wno-style", "-Wno-MULTITOP"])
        // The comparison drives random stimulus; the design's own assertions about its
        // environment are not part of it.
        .args(["--x-assign", "0", "--x-initial", "0", "-O1", "--no-assert"])
        .arg("--Mdir")
        .arg(&obj)
        .args(["-o", "cosim"])
        .arg("-CFLAGS")
        .arg(format!(
            "-std=c++17 -O1 -I{} -I{}",
            sc.join("include").display(),
            work.display()
        ))
        .arg("-LDFLAGS")
        .arg(format!("-L{0} -Wl,-rpath,{0} -lsystemc", lib.display()))
        .arg(&sv_abs)
        .args(verilator_design_args(&opts.design_args))
        .arg(work.join("tb.cpp"))
        .output()
        .map_err(|e| SvccError::new(STAGE, format!("cannot run verilator: {e}")))?;
    if !out.status.success() {
        let log = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let errors: Vec<&str> = log
            .lines()
            .filter(|l| l.contains("error") || l.contains("%Error"))
            .take(12)
            .collect();
        let shown = if errors.is_empty() {
            tail(&log, 15)
        } else {
            errors.join("\n")
        };
        return Err(SvccError::new(
            STAGE,
            format!("build failed in {}:\n{}", work.display(), shown),
        ));
    }
    let run = Command::new(obj.join("cosim"))
        .output()
        .map_err(|e| SvccError::new(STAGE, format!("cannot run the cosim binary: {e}")))?;
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    if !run.status.success() || !stdout.contains("COSIM PASS") {
        let mism: Vec<&str> = stdout
            .lines()
            .filter(|l| l.contains("MISMATCH"))
            .take(10)
            .collect();
        let detail = if mism.is_empty() {
            format!(
                "{}\n{}",
                tail(&stdout, 8),
                tail(&String::from_utf8_lossy(&run.stderr), 8)
            )
        } else {
            mism.join("\n")
        };
        return Err(SvccError::new(
            STAGE,
            format!(
                "'{}' differs from Verilator (work dir {}):\n{}",
                top.definition,
                work.display(),
                detail
            ),
        ));
    }
    let comparisons = stdout
        .lines()
        .find(|l| l.contains("COSIM PASS"))
        .and_then(|l| {
            l.split_whitespace()
                .rev()
                .nth(1)
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0);
    let report = CosimReport {
        top: top.definition.clone(),
        steps: opts.steps,
        comparisons,
        outputs: top.ports.iter().filter(|p| p.dir == Dir::Out).count(),
        work_dir: work.clone(),
    };
    if opts.work_dir.is_none() {
        let _ = std::fs::remove_dir_all(&work);
    }
    Ok(report)
}
