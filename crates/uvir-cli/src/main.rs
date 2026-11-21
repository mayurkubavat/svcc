//! `uvir` command-line compiler, round-trip verifier, and benchmark driver.

use serde::Serialize;
use std::env;
use std::fs;
use std::process::ExitCode;
use std::time::Instant;
use uvir_emit_sc::ScEmitter;
use uvir_emit_sv::SvEmitter;
use uvir_equiv::{prove_class_equivalence, SmtVerdict};
use uvir_front_sc::ScFrontend;
use uvir_front_sv::SvFrontend;
use uvir_ir::{LanguageRegistry, TargetCapabilities, UvirError};
use uvir_passes::PassPipeline;

fn build_registry() -> LanguageRegistry {
    let mut reg = LanguageRegistry::new();
    reg.register_frontend("sv-uvm", Box::new(SvFrontend::new()));
    reg.register_frontend("sc-uvm", Box::new(ScFrontend::new()));
    reg.register_backend("sv-uvm", Box::new(SvEmitter::new()));
    reg.register_backend("sc-uvm", Box::new(ScEmitter::new()));
    reg
}

fn detect_lang(path: &str) -> &'static str {
    if path.ends_with(".sv") || path.ends_with(".svh") {
        "sv-uvm"
    } else {
        "sc-uvm"
    }
}

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<(), UvirError> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_usage();
        return Ok(());
    }

    let reg = build_registry();
    match args[0].as_str() {
        "targets" => cmd_targets(&reg),
        "sv2sc" => {
            let input = args
                .get(1)
                .ok_or_else(|| UvirError::new("cli", "usage: uvir sv2sc <in.sv> [-o <out.hpp>]"))?;
            let out_path = parse_opt_flag(&args[2..], "-o");
            cmd_transpile(&reg, "sv-uvm", "sc-uvm", input, out_path.as_deref())
        }
        "sc2sv" => {
            let input = args
                .get(1)
                .ok_or_else(|| UvirError::new("cli", "usage: uvir sc2sv <in.hpp> [-o <out.sv>]"))?;
            let out_path = parse_opt_flag(&args[2..], "-o");
            cmd_transpile(&reg, "sc-uvm", "sv-uvm", input, out_path.as_deref())
        }
        "transpile" => {
            let from = parse_opt_flag(&args[1..], "--from").unwrap_or_else(|| "sv-uvm".into());
            let to = parse_opt_flag(&args[1..], "--to").unwrap_or_else(|| "sc-uvm".into());
            let out_path = parse_opt_flag(&args[1..], "-o");
            let input = positional_args(&args[1..])
                .into_iter()
                .next()
                .ok_or_else(|| UvirError::new("cli", "missing input file for transpile"))?;
            cmd_transpile(&reg, &from, &to, &input, out_path.as_deref())
        }
        "roundtrip-check" => {
            let input = args
                .get(1)
                .ok_or_else(|| UvirError::new("cli", "usage: uvir roundtrip-check <in.sv>"))?;
            cmd_roundtrip_check(&reg, input)
        }
        "prove-equiv" => {
            if args.len() < 3 {
                return Err(UvirError::new(
                    "cli",
                    "usage: uvir prove-equiv <lhs.{sv,hpp}> <rhs.{sv,hpp}>",
                ));
            }
            cmd_prove_equiv(&reg, &args[1], &args[2])
        }
        "bench" => {
            let input = args
                .get(1)
                .ok_or_else(|| UvirError::new("cli", "usage: uvir bench <in.sv> [--iters N]"))?;
            let iters = parse_opt_flag(&args[2..], "--iters")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(50);
            cmd_bench(&reg, input, iters)
        }
        other => Err(UvirError::new(
            "cli",
            format!("unknown subcommand '{other}' (run `uvir --help`)"),
        )),
    }
}

fn print_usage() {
    println!(
        "uvir 0.1.0 — Multi-Language Verification IR & Bidirectional Compiler\n\
         \n\
         USAGE:\n\
         \x20 uvir targets                                      List active & planned language spokes\n\
         \x20 uvir sv2sc <in.sv> [-o <out.hpp>]                 Transpile SV/UVM -> SystemC/UVM+CRAVE+FC4SC\n\
         \x20 uvir sc2sv <in.hpp> [-o <out.sv>]                 Transpile SystemC/UVM -> IEEE 1800-2023 SV/UVM\n\
         \x20 uvir transpile --from <L1> --to <L2> <in> [-o f]  Generic hub-and-spoke transpilation\n\
         \x20 uvir roundtrip-check <in.sv>                      SV -> SC -> SV + svlang check + Z3 QF_BV proof\n\
         \x20 uvir prove-equiv <lhs> <rhs>                      Prove Z3 SMT-LIB2 & UCIS equivalence\n\
         \x20 uvir bench <in.sv> [--iters 50]                   Emit JSON latency & equivalence telemetry"
    );
}

fn parse_opt_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn positional_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with('-') {
            i += 2;
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    out
}

fn cmd_targets(reg: &LanguageRegistry) -> Result<(), UvirError> {
    let profiles = [
        ("active", TargetCapabilities::sv_uvm_1800_2023()),
        ("active", TargetCapabilities::sc_uvm_crave_fc4sc()),
        ("roadmap-spoke", TargetCapabilities::python_cocotb_pyuvm()),
        ("roadmap-spoke", TargetCapabilities::pss_1800_1()),
        ("roadmap-spoke", TargetCapabilities::rust_verilator()),
    ];
    println!("Registered Frontends : {:?}", reg.registered_frontends());
    println!("Registered Backends  : {:?}\n", reg.registered_backends());
    println!(
        "{:<14} {:<10} {:<18} {:<22} {:<20} {:<16}",
        "STATUS", "LANG_ID", "MEMORY_MODEL", "CONCURRENCY", "SOLVER", "TEMPORAL"
    );
    for (status, cap) in profiles {
        println!(
            "{:<14} {:<10} {:<18?} {:<22?} {:<20?} {:<16?}",
            status,
            cap.language_id,
            cap.memory_model,
            cap.concurrency_model,
            cap.solver_model,
            cap.temporal_model
        );
    }
    Ok(())
}

fn cmd_transpile(
    reg: &LanguageRegistry,
    from: &str,
    to: &str,
    input_path: &str,
    out_path: Option<&str>,
) -> Result<(), UvirError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| UvirError::new("io", format!("cannot read {input_path}: {e}")))?;
    let fe = reg
        .frontend(from)
        .ok_or_else(|| UvirError::new("cli", format!("unknown frontend '{from}'")))?;
    let be = reg
        .backend(to)
        .ok_or_else(|| UvirError::new("cli", format!("unknown backend '{to}'")))?;

    let (mut module, _) = fe.lower_str(input_path, &src)?;
    let pipeline = PassPipeline::for_target(be.capabilities());
    let _ = pipeline.run_all(&mut module)?;

    let emitted = if to == "sv-uvm" {
        SvEmitter::new().emit_and_verify(&module)?
    } else {
        be.emit(&module)?
    };

    if let Some(p) = out_path {
        fs::write(p, &emitted)
            .map_err(|e| UvirError::new("io", format!("cannot write {p}: {e}")))?;
    } else {
        print!("{emitted}");
    }
    Ok(())
}

fn cmd_roundtrip_check(reg: &LanguageRegistry, input_path: &str) -> Result<(), UvirError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| UvirError::new("io", format!("cannot read {input_path}: {e}")))?;
    let sv_fe = reg.frontend("sv-uvm").unwrap();
    let sc_fe = reg.frontend("sc-uvm").unwrap();
    let sc_be = reg.backend("sc-uvm").unwrap();

    // 1. SV -> UVIR -> SC
    let (mut m_sv, _) = sv_fe.lower_str(input_path, &src)?;
    PassPipeline::for_target(sc_be.capabilities()).run_all(&mut m_sv)?;
    let sc_code = sc_be.emit(&m_sv)?;

    // 2. SC -> UVIR -> SV (verified with svlang)
    let (mut m_sc, _) = sc_fe.lower_str("roundtrip.hpp", &sc_code)?;
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023()).run_all(&mut m_sc)?;
    let sv_roundtrip = SvEmitter::new().emit_and_verify(&m_sc)?;

    // 3. Re-parse emitted SV and prove formal Z3 QF_BV + UCIS equivalence against original!
    let (mut m_sv2, _) = sv_fe.lower_str("roundtrip.sv", &sv_roundtrip)?;
    PassPipeline::for_target(sc_be.capabilities()).run_all(&mut m_sv2)?;

    if m_sv.classes.len() != m_sv2.classes.len() {
        return Err(UvirError::new(
            "roundtrip",
            format!(
                "class count mismatch after round-trip: {} vs {}",
                m_sv.classes.len(),
                m_sv2.classes.len()
            ),
        ));
    }

    for (orig, rt) in m_sv.classes.iter().zip(m_sv2.classes.iter()) {
        let rep = prove_class_equivalence(orig, &m_sv.crv_arena, rt, &m_sv2.crv_arena)?;
        println!(
            "[PASS] class '{}': CRV={:?} (z3 {} us, {} B SMT-LIB2), soft_matched={}, cov_isomorphic={}",
            rep.class_name,
            rep.hard_crv_verdict,
            rep.solver_time_us,
            rep.smtlib2_bytes,
            rep.soft_clauses_matched,
            rep.coverage_isomorphic
        );
        if rep.hard_crv_verdict == SmtVerdict::CounterexampleSat || !rep.coverage_isomorphic {
            return Err(UvirError::new(
                "roundtrip",
                format!(
                    "equivalence mismatch in '{}': {}",
                    rep.class_name, rep.raw_solver_output
                ),
            ));
        }
    }
    Ok(())
}

fn cmd_prove_equiv(
    reg: &LanguageRegistry,
    lhs_path: &str,
    rhs_path: &str,
) -> Result<(), UvirError> {
    let lhs_src = fs::read_to_string(lhs_path)
        .map_err(|e| UvirError::new("io", format!("cannot read {lhs_path}: {e}")))?;
    let rhs_src = fs::read_to_string(rhs_path)
        .map_err(|e| UvirError::new("io", format!("cannot read {rhs_path}: {e}")))?;

    let lhs_fe = reg.frontend(detect_lang(lhs_path)).unwrap();
    let rhs_fe = reg.frontend(detect_lang(rhs_path)).unwrap();

    let (mut m_l, _) = lhs_fe.lower_str(lhs_path, &lhs_src)?;
    let (mut m_r, _) = rhs_fe.lower_str(rhs_path, &rhs_src)?;

    // Normalize both sides through canonical solver-legalized form
    let norm = PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc());
    norm.run_all(&mut m_l)?;
    norm.run_all(&mut m_r)?;

    for cls_l in &m_l.classes {
        let cls_r = m_r.find_class(&cls_l.name).ok_or_else(|| {
            UvirError::new(
                "equiv",
                format!("class '{}' missing in {rhs_path}", cls_l.name),
            )
        })?;
        let rep = prove_class_equivalence(cls_l, &m_l.crv_arena, cls_r, &m_r.crv_arena)?;
        println!("{}", serde_json::to_string_pretty(&rep).unwrap());
        if rep.hard_crv_verdict == SmtVerdict::CounterexampleSat || !rep.coverage_isomorphic {
            return Err(UvirError::new("equiv", "equivalence check failed"));
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct BenchReport {
    input_file: String,
    iterations: usize,
    avg_sv2sc_us: u64,
    avg_sc2sv_us: u64,
    emitted_sc_bytes: usize,
    emitted_sv_bytes: usize,
    classes: usize,
    crv_nodes: usize,
    z3_verdict: SmtVerdict,
    z3_solver_us: u64,
    coverage_isomorphic: bool,
}

fn cmd_bench(reg: &LanguageRegistry, input_path: &str, iters: usize) -> Result<(), UvirError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| UvirError::new("io", format!("cannot read {input_path}: {e}")))?;
    let sv_fe = reg.frontend("sv-uvm").unwrap();
    let sc_fe = reg.frontend("sc-uvm").unwrap();
    let sc_be = reg.backend("sc-uvm").unwrap();
    let sv_em = SvEmitter::new();

    let iters = iters.max(1);
    let mut sc_code = String::new();
    let t_sv2sc = Instant::now();
    let mut last_sv_mod = None;
    for _ in 0..iters {
        let (mut m, _) = sv_fe.lower_str(input_path, &src)?;
        PassPipeline::for_target(sc_be.capabilities()).run_all(&mut m)?;
        sc_code = sc_be.emit(&m)?;
        last_sv_mod = Some(m);
    }
    let avg_sv2sc_us = (t_sv2sc.elapsed().as_micros() as u64) / (iters as u64);

    let mut sv_code = String::new();
    let t_sc2sv = Instant::now();
    let mut last_sc_mod = None;
    for _ in 0..iters {
        let (mut m, _) = sc_fe.lower_str("bench.hpp", &sc_code)?;
        PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023()).run_all(&mut m)?;
        sv_code = sv_em.emit_and_verify(&m)?;
        last_sc_mod = Some(m);
    }
    let avg_sc2sv_us = (t_sc2sv.elapsed().as_micros() as u64) / (iters as u64);

    let m_sv = last_sv_mod.unwrap();
    let m_sc = last_sc_mod.unwrap();
    let eq = prove_class_equivalence(
        &m_sv.classes[0],
        &m_sv.crv_arena,
        &m_sc.classes[0],
        &m_sc.crv_arena,
    )?;

    let report = BenchReport {
        input_file: input_path.to_string(),
        iterations: iters,
        avg_sv2sc_us,
        avg_sc2sv_us,
        emitted_sc_bytes: sc_code.len(),
        emitted_sv_bytes: sv_code.len(),
        classes: m_sv.classes.len(),
        crv_nodes: m_sv.crv_arena.len(),
        z3_verdict: eq.hard_crv_verdict,
        z3_solver_us: eq.solver_time_us,
        coverage_isomorphic: eq.coverage_isomorphic,
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}
