//! `uvir` command-line compiler, clang++/svlang verifier, Z3 prover, and benchmark driver
//! focused on IEEE 1800-2023 SystemVerilog/UVM <-> C++17 SystemC/UVM+CRAVE+FC4SC.

use serde::Serialize;
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;
use svcc_emit_sc::ScEmitter;
use svcc_emit_sv::SvEmitter;
use svcc_equiv::{prove_class_equivalence_with_enums, prove_sva_bmc_equivalence, SmtVerdict};
use svcc_front_sc::ScFrontend;
use svcc_front_sv::SvFrontend;
use svcc_ir::{BackendEmitter, LanguageRegistry, TargetCapabilities, UvirError};
use svcc_passes::PassPipeline;

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
            let input = args.get(1).ok_or_else(|| {
                UvirError::new("cli", "usage: uvir roundtrip-check <in.sv> [--shims <dir>]")
            })?;
            let shim_dir = parse_opt_flag(&args[2..], "--shims");
            cmd_roundtrip_check(&reg, input, shim_dir.as_deref())
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
        "svcc 0.2.0 — Bidirectional IEEE 1800-2023 SV/UVM <-> C++17 SystemC/UVM Compiler\n\
         \n\
         USAGE:\n\
         \x20 svcc targets                                      List SV/C++ capability profiles\n\
         \x20 svcc sv2sc <in.sv> [-o <out.hpp>]                 Transpile SV/UVM -> C++17 SystemC/UVM+CRAVE+FC4SC\n\
         \x20 svcc sc2sv <in.hpp> [-o <out.sv>]                 Transpile C++17 SystemC/UVM -> IEEE 1800-2023 SV/UVM\n\
         \x20 svcc roundtrip-check <in.sv> [--shims <dir>]      SV -> C++17 (clang++ -Werror) -> SV (svlang) + Z3 proofs\n\
         \x20 svcc prove-equiv <lhs> <rhs>                      Prove Z3 QF_BV CRV/Enum/Dist & UCIS equivalence\n\
         \x20 svcc bench <in.sv> [--iters 50]                   Emit JSON latency & formal equivalence telemetry"
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
        TargetCapabilities::sv_uvm_1800_2023(),
        TargetCapabilities::sc_uvm_crave_fc4sc(),
    ];
    println!("Registered Frontends : {:?}", reg.registered_frontends());
    println!("Registered Backends  : {:?}\n", reg.registered_backends());
    println!(
        "{:<10} {:<18} {:<22} {:<20} {:<18}",
        "LANG_ID", "MEMORY_MODEL", "CONCURRENCY", "SOLVER", "TEMPORAL"
    );
    for cap in profiles {
        println!(
            "{:<10} {:<18?} {:<22?} {:<20?} {:<18?}",
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
    PassPipeline::for_target(be.capabilities()).run_all(&mut module)?;

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

fn cmd_roundtrip_check(
    reg: &LanguageRegistry,
    input_path: &str,
    shim_dir: Option<&str>,
) -> Result<(), UvirError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| UvirError::new("io", format!("cannot read {input_path}: {e}")))?;
    let sv_fe = reg.frontend("sv-uvm").unwrap();
    let sc_fe = reg.frontend("sc-uvm").unwrap();
    let sc_em = ScEmitter::new();
    let sc_caps = TargetCapabilities::sc_uvm_crave_fc4sc();

    // 1. SV -> UVIR -> C++17 (verified via clang++ -Werror when shim directory exists)
    let (mut m_sv, _) = sv_fe.lower_str(input_path, &src)?;
    PassPipeline::for_target(sc_caps.clone()).run_all(&mut m_sv)?;
    let default_shims = Path::new("testdata/cpp_shims");
    let sc_code = if let Some(dir) = shim_dir {
        sc_em.emit_and_verify_clang(&m_sv, Path::new(dir))?
    } else if default_shims.is_dir() {
        sc_em.emit_and_verify_clang(&m_sv, default_shims)?
    } else {
        sc_em.emit(&m_sv)?
    };

    // 2. C++17 -> UVIR -> SV (verified via svlang)
    let (mut m_sc, _) = sc_fe.lower_str("roundtrip.hpp", &sc_code)?;
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023()).run_all(&mut m_sc)?;
    let sv_roundtrip = SvEmitter::new().emit_and_verify(&m_sc)?;

    // 3. Re-parse SV and prove Z3 QF_BV CRV/Enum/Dist + UCIS isomorphism
    let (mut m_sv2, _) = sv_fe.lower_str("roundtrip.sv", &sv_roundtrip)?;
    PassPipeline::for_target(sc_caps).run_all(&mut m_sv2)?;

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
        let rep = prove_class_equivalence_with_enums(
            orig,
            &m_sv.crv_arena,
            &m_sv.enums,
            rt,
            &m_sv2.crv_arena,
            &m_sv2.enums,
        )?;
        println!(
            "[PASS] class '{}': CRV={:?} (z3 {} us, {} B), soft={}, dist={}, cov_iso={}",
            rep.class_name,
            rep.hard_crv_verdict,
            rep.solver_time_us,
            rep.smtlib2_bytes,
            rep.soft_clauses_matched,
            rep.dist_clauses_matched,
            rep.coverage_isomorphic
        );
        if rep.hard_crv_verdict == SmtVerdict::CounterexampleSat || !rep.coverage_isomorphic {
            return Err(UvirError::new(
                "roundtrip",
                format!("equivalence mismatch in '{}'", rep.class_name),
            ));
        }
    }

    // 4. Prove SVA <-> C++17 pipeline observer BMC trace equivalence for every property
    for prop in &m_sv.properties {
        let bmc = prove_sva_bmc_equivalence(prop, 6)?;
        println!(
            "[PASS] sva   '{}': BMC({} cycles)={:?} (z3 {} us, {} B)",
            bmc.property_name, bmc.bound_cycles, bmc.verdict, bmc.solver_time_us, bmc.smtlib2_bytes
        );
        if bmc.verdict == SmtVerdict::CounterexampleSat {
            return Err(UvirError::new(
                "roundtrip/sva",
                format!("SVA observer BMC mismatch in '{}'", prop.name),
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
        let rep = prove_class_equivalence_with_enums(
            cls_l,
            &m_l.crv_arena,
            &m_l.enums,
            cls_r,
            &m_r.crv_arena,
            &m_r.enums,
        )?;
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
    enums: usize,
    clockings: usize,
    sva_properties: usize,
    crv_nodes: usize,
    z3_crv_verdict: SmtVerdict,
    z3_sva_bmc_verdict: SmtVerdict,
    z3_total_solver_us: u64,
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

    let mut z3_total_us = 0u64;
    let mut crv_verdict = SmtVerdict::EquivalentUnsat;
    let mut cov_iso = true;
    for cls_sv in &m_sv.classes {
        if let Some(cls_sc) = m_sc.find_class(&cls_sv.name) {
            let eq = prove_class_equivalence_with_enums(
                cls_sv,
                &m_sv.crv_arena,
                &m_sv.enums,
                cls_sc,
                &m_sc.crv_arena,
                &m_sc.enums,
            )?;
            z3_total_us += eq.solver_time_us;
            if eq.hard_crv_verdict != SmtVerdict::EquivalentUnsat {
                crv_verdict = eq.hard_crv_verdict;
            }
            cov_iso &= eq.coverage_isomorphic;
        }
    }

    let mut sva_verdict = SmtVerdict::EquivalentUnsat;
    for prop in &m_sv.properties {
        let bmc = prove_sva_bmc_equivalence(prop, 6)?;
        z3_total_us += bmc.solver_time_us;
        if bmc.verdict != SmtVerdict::EquivalentUnsat {
            sva_verdict = bmc.verdict;
        }
    }

    let report = BenchReport {
        input_file: input_path.to_string(),
        iterations: iters,
        avg_sv2sc_us,
        avg_sc2sv_us,
        emitted_sc_bytes: sc_code.len(),
        emitted_sv_bytes: sv_code.len(),
        classes: m_sv.classes.len(),
        enums: m_sv.enums.len(),
        clockings: m_sv.clockings.len(),
        sva_properties: m_sv.properties.len(),
        crv_nodes: m_sv.crv_arena.len(),
        z3_crv_verdict: crv_verdict,
        z3_sva_bmc_verdict: sva_verdict,
        z3_total_solver_us: z3_total_us,
        coverage_isomorphic: cov_iso,
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}
