//! Word-Level RTL & C2RTL Sequential Equivalence Checking (`SEC`) via `svlang` `Netlist` +
//! `sv-btor2` + Z3 `QF_BV` Bounded Unrolling.
//!
//! Given a reference `spec` [`RtlModule`] (lowered either from SV RTL, SystemC `SC_MODULE`, or an
//! untimed `// svcc:c2rtl` C++17 golden function) and an implementation `imp` [`RtlModule`]
//! (potentially pipelined by `latency` clock cycles), [`prove_rtl_sec_equivalence`]:
//! 1. Synthesizes a self-contained IEEE 1800-2023 product-machine miter `svcc_sec_miter` driving
//!    identical symbolic primary inputs into both `spec` and `imp`, delaying `spec` outputs by
//!    `latency` cycles, holding reset active on cycle 0 (`rst_n == 0` -> `rst_n == 1`), and
//!    asserting output equality after pipeline warmup (`latency + 1` cycles out of reset).
//! 2. Elaborates & lowers the product machine via `svlang::netlist::lower_trees` and emits a
//!    standard word-level **Btor2** transition system (`svlang::btor2::write`) ready for `InFormal`,
//!    `rIC3`, or `Pono`.
//! 3. Unrolls the Btor2 transition system for `K = latency + extra_cycles` steps into SMT-LIB2
//!    `QF_BV` and queries `z3 -in` to prove sequential equivalence (`EquivalentUnsat`) or report a
//!    bug (`CounterexampleSat`).

use crate::{run_z3_stdin, SmtVerdict};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::Instant;
use svcc_ir::{RtlModule, RtlPortDir, UvirError};
use svlang::netlist::{lower_trees, LowerConfig, LowerOutcome};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RtlSecReport {
    pub spec_module: String,
    pub imp_module: String,
    pub latency_cycles: u32,
    pub bmc_bound_cycles: u32,
    pub verdict: SmtVerdict,
    pub btor2_lines: usize,
    pub smtlib2_bytes: usize,
    pub solver_time_us: u64,
}

/// Proves sequential equivalence (`latency = 0` or pipelined `latency = L`) between two SV RTL
/// strings containing `spec_mod` and `imp_mod`.
pub fn prove_rtl_sec_equivalence(
    spec_mod: &RtlModule,
    spec_sv_src: &str,
    imp_mod: &RtlModule,
    imp_sv_src: &str,
    latency: u32,
    extra_cycles: u32,
) -> Result<RtlSecReport, UvirError> {
    let renamed_spec_name = format!("{}__svcc_spec", spec_mod.name);
    let renamed_imp_name = format!("{}__svcc_imp", imp_mod.name);

    let spec_sv_renamed = rename_sv_module_decl(spec_sv_src, &spec_mod.name, &renamed_spec_name);
    let imp_sv_renamed = rename_sv_module_decl(imp_sv_src, &imp_mod.name, &renamed_imp_name);

    let miter_sv = build_sv_sec_miter(
        spec_mod,
        &renamed_spec_name,
        imp_mod,
        &renamed_imp_name,
        latency,
    )?;

    let combined_sv = format!("{spec_sv_renamed}\n\n{imp_sv_renamed}\n\n{miter_sv}\n");
    let tree = svlang::syntax::parse_source(&combined_sv, "svcc_sec_miter.sv", Default::default());
    let netlist = match lower_trees(
        &[tree],
        &["svcc_sec_miter".to_string()],
        &[],
        &LowerConfig::default(),
    ) {
        LowerOutcome::Lowered(nl) => nl,
        LowerOutcome::Unsupported(ds) | LowerOutcome::Error(ds) => {
            let msg: Vec<String> = ds.iter().map(ToString::to_string).collect();
            return Err(UvirError::new(
                "svcc-equiv/netlist",
                format!(
                    "failed lowering SEC miter to sv-netlist:\n{}",
                    msg.join("\n")
                ),
            ));
        }
    };

    let btor_model = svlang::btor2::write(&netlist, &svlang::btor2::Options::default())
        .map_err(|e| UvirError::new("svcc-equiv/btor2", format!("btor2 write failed: {e:?}")))?;
    let btor2_lines = btor_model.btor2.lines().count();

    let bound_k = latency + extra_cycles.max(4) + 2;
    let smt = unroll_btor2_to_smtlib2(&btor_model.btor2, bound_k)?;
    let smt_bytes = smt.len();

    let t0 = Instant::now();
    let (verdict, _) = run_z3_stdin(&smt);
    let solver_time_us = t0.elapsed().as_micros() as u64;

    Ok(RtlSecReport {
        spec_module: spec_mod.name.clone(),
        imp_module: imp_mod.name.clone(),
        latency_cycles: latency,
        bmc_bound_cycles: bound_k,
        verdict,
        btor2_lines,
        smtlib2_bytes: smt_bytes,
        solver_time_us,
    })
}

fn rename_sv_module_decl(sv: &str, old_name: &str, new_name: &str) -> String {
    sv.replacen(
        &format!("module {old_name}"),
        &format!("module {new_name}"),
        1,
    )
}

fn build_sv_sec_miter(
    spec: &RtlModule,
    spec_sv_name: &str,
    imp: &RtlModule,
    imp_sv_name: &str,
    latency: u32,
) -> Result<String, UvirError> {
    // Identify clock and reset ports on `imp` (or synthesize a miter clock/reset when both are combinational)
    let clk_port = imp
        .ports
        .iter()
        .chain(spec.ports.iter())
        .find(|p| {
            p.dir == RtlPortDir::Input && (p.name.contains("clk") || p.name.contains("clock"))
        })
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "clk".into());

    let rst_port_opt = imp
        .ports
        .iter()
        .chain(spec.ports.iter())
        .find(|p| {
            p.dir == RtlPortDir::Input && (p.name.contains("rst") || p.name.contains("reset"))
        })
        .map(|p| p.name.clone());
    let rst_name = rst_port_opt.clone().unwrap_or_else(|| "rst_n".into());
    let rst_active_low = rst_name.ends_with("_n") || rst_name.ends_with("n");

    // Collect shared data input ports (all inputs on `spec` that aren't clk/rst, or matching inputs on `imp`)
    let data_inputs: Vec<_> = imp
        .ports
        .iter()
        .filter(|p| {
            p.dir == RtlPortDir::Input
                && p.name != clk_port
                && Some(&p.name) != rst_port_opt.as_ref()
        })
        .collect();

    let spec_outputs: Vec<_> = spec
        .ports
        .iter()
        .filter(|p| p.dir == RtlPortDir::Output)
        .collect();
    let imp_outputs: Vec<_> = imp
        .ports
        .iter()
        .filter(|p| p.dir == RtlPortDir::Output)
        .collect();

    if spec_outputs.is_empty() || imp_outputs.is_empty() {
        return Err(UvirError::new(
            "svcc-equiv/sec",
            "both spec and imp RTL modules must declare at least one output port",
        ));
    }

    let mut s = String::with_capacity(2048);
    let _ = writeln!(s, "module svcc_sec_miter (");
    let _ = writeln!(s, "  input logic {clk_port},");
    for inp in &data_inputs {
        let decl = if inp.width <= 1 {
            format!("input logic {}", inp.name)
        } else {
            format!("input logic [{}:0] {}", inp.width - 1, inp.name)
        };
        let _ = writeln!(s, "  {decl},");
    }
    s.push_str("  input logic __svcc_dummy_in\n);\n");

    // Deterministic warmup + self-reset sequence (`init_q == 0` on step 0 asserts reset, then releases!)
    s.push_str("  logic [7:0] __warmup_cnt = 8'd0;\n");
    let rst_expr = if rst_active_low {
        "(__warmup_cnt != 8'd0)"
    } else {
        "(__warmup_cnt == 8'd0)"
    };
    let _ = writeln!(s, "  logic {rst_name};");
    let _ = writeln!(s, "  assign {rst_name} = {rst_expr};");
    let _ = writeln!(
        s,
        "  always_ff @(posedge {clk_port}) begin\n    if (__warmup_cnt < 8'd200) __warmup_cnt <= __warmup_cnt + 8'd1;\n  end\n"
    );

    // Output wires
    for (idx, (so, io)) in spec_outputs.iter().zip(imp_outputs.iter()).enumerate() {
        let w = so.width.max(io.width);
        if w <= 1 {
            let _ = writeln!(s, "  logic __spec_out_{idx};");
            let _ = writeln!(s, "  logic __imp_out_{idx};");
        } else {
            let _ = writeln!(s, "  logic [{}:0] __spec_out_{idx};", w - 1);
            let _ = writeln!(s, "  logic [{}:0] __imp_out_{idx};", w - 1);
        }
    }

    // Instantiate `spec`
    let mut spec_conns = Vec::new();
    for p in &spec.ports {
        match p.dir {
            RtlPortDir::Input => {
                spec_conns.push(format!(".{}({})", p.name, p.name));
            }
            RtlPortDir::Output => {
                if let Some(pos) = spec_outputs.iter().position(|o| o.name == p.name) {
                    spec_conns.push(format!(".{}(__spec_out_{pos})", p.name));
                }
            }
        }
    }
    let _ = writeln!(s, "  {spec_sv_name} u_spec ({});", spec_conns.join(", "));

    // Instantiate `imp`
    let mut imp_conns = Vec::new();
    for p in &imp.ports {
        match p.dir {
            RtlPortDir::Input => {
                imp_conns.push(format!(".{}({})", p.name, p.name));
            }
            RtlPortDir::Output => {
                if let Some(pos) = imp_outputs.iter().position(|o| o.name == p.name) {
                    imp_conns.push(format!(".{}(__imp_out_{pos})", p.name));
                }
            }
        }
    }
    let _ = writeln!(s, "  {imp_sv_name} u_imp ({});\n", imp_conns.join(", "));

    // Delay `spec` outputs by `latency` clock cycles when `latency > 0`
    let min_warm = latency + 1;
    for (idx, (so, io)) in spec_outputs.iter().zip(imp_outputs.iter()).enumerate() {
        let w = so.width.max(io.width);
        let delayed_sig = if latency == 0 {
            format!("__spec_out_{idx}")
        } else {
            for stg in 0..latency {
                if w <= 1 {
                    let _ = writeln!(s, "  logic __pipe_{idx}_{stg} = 1'b0;");
                } else {
                    let _ = writeln!(s, "  logic [{}:0] __pipe_{idx}_{stg} = '0;", w - 1);
                }
            }
            let _ = writeln!(s, "  always_ff @(posedge {clk_port}) begin");
            let _ = writeln!(s, "    __pipe_{idx}_0 <= __spec_out_{idx};");
            for stg in 1..latency {
                let _ = writeln!(s, "    __pipe_{idx}_{stg} <= __pipe_{idx}_{};", stg - 1);
            }
            s.push_str("  end\n");
            format!("__pipe_{idx}_{}", latency - 1)
        };

        // Immediate clocked assertion becomes a Btor2 `bad` line!
        let _ = writeln!(
            s,
            "  always @(posedge {clk_port}) sec_out_{idx}: assert ((__warmup_cnt <= 8'd{min_warm}) || (__imp_out_{idx} == {delayed_sig}));"
        );
    }

    s.push_str("endmodule\n");
    Ok(s)
}

// ============================================================================
// Built-In Word-Level Btor2 -> Z3 SMT-LIB2 `QF_BV` Unroller
// ============================================================================

#[derive(Clone, Debug)]
enum BtorLine {
    Input {
        sort: u32,
    },
    State {
        sort: u32,
    },
    ConstBin {
        bits: String,
    },
    ConstDec {
        sort: u32,
        dec: String,
    },
    ConstHex {
        sort: u32,
        hex: String,
    },
    Zero {
        sort: u32,
    },
    One {
        sort: u32,
    },
    Ones {
        sort: u32,
    },
    Unary {
        op: String,
        sort: u32,
        a: i64,
    },
    Binary {
        op: String,
        sort: u32,
        a: i64,
        b: i64,
    },
    Slice {
        a: i64,
        hi: u32,
        lo: u32,
    },
    Ext {
        signed: bool,
        a: i64,
        ext: u32,
    },
    Ite {
        c: i64,
        t: i64,
        e: i64,
    },
}

fn unroll_btor2_to_smtlib2(btor2: &str, k_steps: u32) -> Result<String, UvirError> {
    let mut sorts: BTreeMap<u32, u32> = BTreeMap::new();
    let mut nodes: BTreeMap<u32, BtorLine> = BTreeMap::new();
    let mut inits: BTreeMap<u32, i64> = BTreeMap::new();
    let mut nexts: BTreeMap<u32, i64> = BTreeMap::new();
    let mut bads: Vec<i64> = Vec::new();
    let mut constraints: Vec<i64> = Vec::new();
    let mut states: Vec<(u32, u32)> = Vec::new();
    let mut inputs: Vec<(u32, u32)> = Vec::new();

    for raw in btor2.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 2 {
            continue;
        }
        let Ok(id) = toks[0].parse::<u32>() else {
            continue;
        };
        let op = toks[1];
        match op {
            "sort" if toks.get(2) == Some(&"bitvec") => {
                let w = toks[3].parse::<u32>().unwrap_or(1);
                sorts.insert(id, w);
            }
            "input" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                inputs.push((id, s));
                nodes.insert(id, BtorLine::Input { sort: s });
            }
            "state" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                states.push((id, s));
                nodes.insert(id, BtorLine::State { sort: s });
            }
            "init" => {
                let st = toks[3].parse::<u32>().unwrap_or(0);
                let val = toks[4].parse::<i64>().unwrap_or(0);
                inits.insert(st, val);
            }
            "next" => {
                let st = toks[3].parse::<u32>().unwrap_or(0);
                let val = toks[4].parse::<i64>().unwrap_or(0);
                nexts.insert(st, val);
            }
            "const" => {
                nodes.insert(
                    id,
                    BtorLine::ConstBin {
                        bits: toks[3].to_string(),
                    },
                );
            }
            "constd" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                nodes.insert(
                    id,
                    BtorLine::ConstDec {
                        sort: s,
                        dec: toks[3].to_string(),
                    },
                );
            }
            "consth" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                nodes.insert(
                    id,
                    BtorLine::ConstHex {
                        sort: s,
                        hex: toks[3].to_string(),
                    },
                );
            }
            "zero" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                nodes.insert(id, BtorLine::Zero { sort: s });
            }
            "one" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                nodes.insert(id, BtorLine::One { sort: s });
            }
            "ones" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                nodes.insert(id, BtorLine::Ones { sort: s });
            }
            "slice" => {
                let a = toks[3].parse::<i64>().unwrap_or(0);
                let hi = toks[4].parse::<u32>().unwrap_or(0);
                let lo = toks[5].parse::<u32>().unwrap_or(0);
                nodes.insert(id, BtorLine::Slice { a, hi, lo });
            }
            "uext" | "sext" => {
                let a = toks[3].parse::<i64>().unwrap_or(0);
                let ext = toks[4].parse::<u32>().unwrap_or(0);
                nodes.insert(
                    id,
                    BtorLine::Ext {
                        signed: op == "sext",
                        a,
                        ext,
                    },
                );
            }
            "ite" => {
                let c = toks[3].parse::<i64>().unwrap_or(0);
                let t = toks[4].parse::<i64>().unwrap_or(0);
                let e = toks[5].parse::<i64>().unwrap_or(0);
                nodes.insert(id, BtorLine::Ite { c, t, e });
            }
            "bad" => {
                let c = toks[2].parse::<i64>().unwrap_or(0);
                bads.push(c);
            }
            "constraint" => {
                let c = toks[2].parse::<i64>().unwrap_or(0);
                constraints.push(c);
            }
            "not" | "neg" | "redand" | "redor" | "redxor" => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                let a = toks[3].parse::<i64>().unwrap_or(0);
                nodes.insert(
                    id,
                    BtorLine::Unary {
                        op: op.to_string(),
                        sort: s,
                        a,
                    },
                );
            }
            _ if toks.len() >= 5 => {
                let s = toks[2].parse::<u32>().unwrap_or(1);
                let a = toks[3].parse::<i64>().unwrap_or(0);
                let b = toks[4].parse::<i64>().unwrap_or(0);
                nodes.insert(
                    id,
                    BtorLine::Binary {
                        op: op.to_string(),
                        sort: s,
                        a,
                        b,
                    },
                );
            }
            _ => {}
        }
    }

    let mut smt = String::with_capacity(16384);
    smt.push_str("(set-logic QF_BV)\n");

    let ref_term = |r: i64, step: u32| -> String {
        let abs = r.unsigned_abs() as u32;
        if r < 0 {
            format!("(bvnot n_{step}_{abs})")
        } else {
            format!("n_{step}_{abs}")
        }
    };

    let mut bad_step_terms = Vec::new();

    for step in 0..k_steps {
        for &(inp_id, s_id) in &inputs {
            let w = sorts.get(&s_id).copied().unwrap_or(1);
            let _ = writeln!(smt, "(declare-const n_{step}_{inp_id} (_ BitVec {w}))");
        }
        if step == 0 {
            for &(st_id, s_id) in &states {
                let w = sorts.get(&s_id).copied().unwrap_or(1);
                let _ = writeln!(smt, "(declare-const s_init_{st_id} (_ BitVec {w}))");
            }
        }

        // Emit DAG definitions in ascending Btor2 ID order (guaranteed topological except state init/next)
        for (&id, node) in &nodes {
            match node {
                BtorLine::Input { .. } => {}
                BtorLine::State { sort } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    if step == 0 {
                        if let Some(&init_ref) = inits.get(&id) {
                            let iv = ref_term(init_ref, 0);
                            let _ = writeln!(smt, "(define-fun n_0_{id} () (_ BitVec {w}) {iv})");
                        } else {
                            let _ = writeln!(
                                smt,
                                "(define-fun n_0_{id} () (_ BitVec {w}) s_init_{id})"
                            );
                        }
                    } else if let Some(&nxt_ref) = nexts.get(&id) {
                        let pv = ref_term(nxt_ref, step - 1);
                        let _ = writeln!(smt, "(define-fun n_{step}_{id} () (_ BitVec {w}) {pv})");
                    } else {
                        let _ = writeln!(
                            smt,
                            "(define-fun n_{step}_{id} () (_ BitVec {w}) n_{}_{id})",
                            step - 1
                        );
                    }
                }
                BtorLine::ConstBin { bits } => {
                    let w = bits.len();
                    let clean: String = bits
                        .chars()
                        .map(|c| if c == '1' { '1' } else { '0' })
                        .collect();
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) #b{clean})"
                    );
                }
                BtorLine::ConstDec { sort, dec } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    let v = dec.parse::<i128>().unwrap_or(0) as u128;
                    let mask = if w >= 128 {
                        u128::MAX
                    } else {
                        (1u128 << w) - 1
                    };
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) (_ bv{} {w}))",
                        v & mask
                    );
                }
                BtorLine::ConstHex { sort, hex } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    let v = u128::from_str_radix(hex, 16).unwrap_or(0);
                    let mask = if w >= 128 {
                        u128::MAX
                    } else {
                        (1u128 << w) - 1
                    };
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) (_ bv{} {w}))",
                        v & mask
                    );
                }
                BtorLine::Zero { sort } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) (_ bv0 {w}))"
                    );
                }
                BtorLine::One { sort } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) (_ bv1 {w}))"
                    );
                }
                BtorLine::Ones { sort } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) (bvnot (_ bv0 {w})))"
                    );
                }
                BtorLine::Slice { a, hi, lo } => {
                    let w = hi - lo + 1;
                    let av = ref_term(*a, step);
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) ((_ extract {hi} {lo}) {av}))"
                    );
                }
                BtorLine::Ext { signed, a, ext } => {
                    let av = ref_term(*a, step);
                    let op_s = if *signed {
                        "sign_extend"
                    } else {
                        "zero_extend"
                    };
                    let base_id = a.unsigned_abs() as u32;
                    let aw = infer_btor_width(base_id, &nodes, &sorts);
                    let w = aw + ext;
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) ((_ {op_s} {ext}) {av}))"
                    );
                }
                BtorLine::Ite { c, t, e } => {
                    let cv = ref_term(*c, step);
                    let tv = ref_term(*t, step);
                    let ev = ref_term(*e, step);
                    let w = infer_btor_width(t.unsigned_abs() as u32, &nodes, &sorts);
                    let _ = writeln!(
                        smt,
                        "(define-fun n_{step}_{id} () (_ BitVec {w}) (ite (= {cv} #b1) {tv} {ev}))"
                    );
                }
                BtorLine::Unary { op, sort, a } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    let av = ref_term(*a, step);
                    let aw = infer_btor_width(a.unsigned_abs() as u32, &nodes, &sorts);
                    let expr = match op.as_str() {
                        "not" => format!("(bvnot {av})"),
                        "neg" => format!("(bvneg {av})"),
                        "redor" => format!("(ite (= {av} (_ bv0 {aw})) #b0 #b1)"),
                        "redand" => format!("(ite (= {av} (bvnot (_ bv0 {aw}))) #b1 #b0)"),
                        "redxor" => {
                            let mut bits = Vec::new();
                            for b in 0..aw {
                                bits.push(format!("((_ extract {b} {b}) {av})"));
                            }
                            bits.into_iter()
                                .reduce(|acc, x| format!("(bvxor {acc} {x})"))
                                .unwrap_or_else(|| "#b0".into())
                        }
                        _ => av,
                    };
                    let _ = writeln!(smt, "(define-fun n_{step}_{id} () (_ BitVec {w}) {expr})");
                }
                BtorLine::Binary { op, sort, a, b } => {
                    let w = sorts.get(sort).copied().unwrap_or(1);
                    let av = ref_term(*a, step);
                    let bv = ref_term(*b, step);
                    let expr = match op.as_str() {
                        "add" => format!("(bvadd {av} {bv})"),
                        "sub" => format!("(bvsub {av} {bv})"),
                        "mul" => format!("(bvmul {av} {bv})"),
                        "udiv" => format!("(bvudiv {av} {bv})"),
                        "sdiv" => format!("(bvsdiv {av} {bv})"),
                        "urem" => format!("(bvurem {av} {bv})"),
                        "srem" => format!("(bvsrem {av} {bv})"),
                        "and" => format!("(bvand {av} {bv})"),
                        "or" => format!("(bvor {av} {bv})"),
                        "xor" => format!("(bvxor {av} {bv})"),
                        "xnor" => format!("(bvxnor {av} {bv})"),
                        "nand" => format!("(bvnand {av} {bv})"),
                        "nor" => format!("(bvnor {av} {bv})"),
                        "sll" => format!("(bvshl {av} {bv})"),
                        "srl" => format!("(bvlshr {av} {bv})"),
                        "sra" => format!("(bvashr {av} {bv})"),
                        "concat" => format!("(concat {av} {bv})"),
                        "eq" => format!("(ite (= {av} {bv}) #b1 #b0)"),
                        "neq" => format!("(ite (= {av} {bv}) #b0 #b1)"),
                        "ult" => format!("(ite (bvult {av} {bv}) #b1 #b0)"),
                        "ulte" => format!("(ite (bvule {av} {bv}) #b1 #b0)"),
                        "ugt" => format!("(ite (bvugt {av} {bv}) #b1 #b0)"),
                        "ugte" => format!("(ite (bvuge {av} {bv}) #b1 #b0)"),
                        "slt" => format!("(ite (bvslt {av} {bv}) #b1 #b0)"),
                        "slte" => format!("(ite (bvsle {av} {bv}) #b1 #b0)"),
                        "sgt" => format!("(ite (bvsgt {av} {bv}) #b1 #b0)"),
                        "sgte" => format!("(ite (bvsge {av} {bv}) #b1 #b0)"),
                        "implies" => format!("(bvor (bvnot {av}) {bv})"),
                        "iff" => format!("(ite (= {av} {bv}) #b1 #b0)"),
                        other => {
                            return Err(UvirError::new(
                                "svcc-equiv/btor2-smt",
                                format!("unsupported btor2 binary op '{other}'"),
                            ));
                        }
                    };
                    let _ = writeln!(smt, "(define-fun n_{step}_{id} () (_ BitVec {w}) {expr})");
                }
            }
        }

        for &c_ref in &constraints {
            let cv = ref_term(c_ref, step);
            let _ = writeln!(smt, "(assert (= {cv} #b1))");
        }
        for &b_ref in &bads {
            let bv = ref_term(b_ref, step);
            bad_step_terms.push(format!("(= {bv} #b1)"));
        }
    }

    if bad_step_terms.is_empty() {
        smt.push_str("(assert false)\n");
    } else {
        let _ = writeln!(smt, "(assert (or {}))", bad_step_terms.join(" "));
    }
    smt.push_str("(check-sat)\n");
    Ok(smt)
}

fn infer_btor_width(id: u32, nodes: &BTreeMap<u32, BtorLine>, sorts: &BTreeMap<u32, u32>) -> u32 {
    match nodes.get(&id) {
        Some(
            BtorLine::Input { sort }
            | BtorLine::State { sort }
            | BtorLine::ConstDec { sort, .. }
            | BtorLine::ConstHex { sort, .. }
            | BtorLine::Zero { sort }
            | BtorLine::One { sort }
            | BtorLine::Ones { sort }
            | BtorLine::Unary { sort, .. }
            | BtorLine::Binary { sort, .. },
        ) => sorts.get(sort).copied().unwrap_or(1),
        Some(BtorLine::ConstBin { bits }) => bits.len() as u32,
        Some(BtorLine::Slice { hi, lo, .. }) => hi - lo + 1,
        Some(BtorLine::Ext { a, ext, .. }) => {
            infer_btor_width(a.unsigned_abs() as u32, nodes, sorts) + ext
        }
        Some(BtorLine::Ite { t, .. }) => infer_btor_width(t.unsigned_abs() as u32, nodes, sorts),
        _ => 1,
    }
}
