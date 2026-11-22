//! `svcc` command-line compiler, clang++/svlang verifier, Z3 CRV/SVA/SEC prover, and benchmarker.

use serde::Serialize;
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;
use svcc::prove_rtl_or_c2rtl_sec;
use svcc_emit_sc::ScEmitter;
use svcc_emit_sv::SvEmitter;
use svcc_equiv::{
    prove_class_equivalence_with_enums, prove_rtl_sec_equivalence, prove_sva_bmc_equivalence,
    prove_sva_cover_multiplicity_bmc, prove_sva_pair_equivalence, SmtVerdict,
};
use svcc_front_sv::SvFrontend;
use svcc_ir::{BackendEmitter, Frontend, LanguageRegistry, SvccError, TargetCapabilities};
use svcc_passes::PassPipeline;

fn build_registry() -> LanguageRegistry {
    svcc::default_registry()
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

fn run_cli() -> Result<(), SvccError> {
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
                .ok_or_else(|| SvccError::new("cli", "usage: svcc sv2sc <in.sv> [-o <out.hpp>] [--partition-clauses B] [--no-cst-fallback] [--no-dist-lcm] [--force-bitmask-sva]"))?;
            let out_path = parse_opt_flag(&args[2..], "-o");
            let partition = parse_opt_flag(&args[2..], "--partition-clauses")
                .and_then(|s| s.parse::<usize>().ok());
            let no_cst = args[2..].iter().any(|a| a == "--no-cst-fallback");
            let no_dist_lcm = args[2..].iter().any(|a| a == "--no-dist-lcm");
            let force_bitmask = args[2..].iter().any(|a| a == "--force-bitmask-sva");
            cmd_sv2sc_ext(
                input,
                out_path.as_deref(),
                partition,
                no_cst,
                no_dist_lcm,
                force_bitmask,
            )
        }
        "rtl2sc" => {
            let pos = positional_args(&args[1..]);
            let input = pos.first().ok_or_else(|| {
                SvccError::new(
                    "cli",
                    "usage: svcc rtl2sc <in.sv> [--top <module>] [-o <out.hpp>]",
                )
            })?;
            let tops: Vec<String> = parse_opt_flag(&args[1..], "--top").into_iter().collect();
            let src = fs::read_to_string(input)
                .map_err(|e| SvccError::new("io", format!("cannot read {input}: {e}")))?;
            let (code, design) = svcc::sv_rtl_to_sc(input, &src, &tops)?;
            for n in &design.notes {
                eprintln!("note: {n}");
            }
            match parse_opt_flag(&args[1..], "-o") {
                Some(p) => fs::write(&p, code)
                    .map_err(|e| SvccError::new("io", format!("cannot write {p}: {e}")))?,
                None => print!("{code}"),
            }
            Ok(())
        }
        "rtl-cosim" => {
            let pos = positional_args(&args[1..]);
            let input = pos.first().ok_or_else(|| {
                SvccError::new(
                    "cli",
                    "usage: svcc rtl-cosim <in.sv> [--top <module>] [--steps N] [--seed S] [--keep <dir>]",
                )
            })?;
            let opts = svcc::cosim::CosimOptions {
                top: parse_opt_flag(&args[1..], "--top"),
                steps: parse_opt_flag(&args[1..], "--steps")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20000),
                seed: parse_opt_flag(&args[1..], "--seed")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0x5eed_1234_abcd),
                work_dir: parse_opt_flag(&args[1..], "--keep").map(Into::into),
            };
            let rep = svcc::cosim::run_cosim(input, &opts)?;
            println!(
                "[PASS] cosim '{}': SystemC == Verilator on {} outputs, {} steps, {} comparisons",
                rep.top, rep.outputs, rep.steps, rep.comparisons
            );
            Ok(())
        }
        "sc2sv" | "c2rtl" => {
            let input = args.get(1).ok_or_else(|| {
                SvccError::new("cli", "usage: svcc sc2sv|c2rtl <in.hpp> [-o <out.sv>]")
            })?;
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
                .ok_or_else(|| SvccError::new("cli", "missing input file for transpile"))?;
            cmd_transpile(&reg, &from, &to, &input, out_path.as_deref())
        }
        "roundtrip-check" => {
            let input = args.get(1).ok_or_else(|| {
                SvccError::new("cli", "usage: svcc roundtrip-check <in.sv> [--shims <dir>]")
            })?;
            let shim_dir = parse_opt_flag(&args[2..], "--shims");
            cmd_roundtrip_check(&reg, input, shim_dir.as_deref())
        }
        "prove-equiv" => {
            if args.len() < 3 {
                return Err(SvccError::new(
                    "cli",
                    "usage: svcc prove-equiv <lhs.{sv,hpp}> <rhs.{sv,hpp}>",
                ));
            }
            cmd_prove_equiv(&reg, &args[1], &args[2])
        }
        "prove-sec" => {
            let pos = positional_args(&args[1..]);
            if pos.len() < 2 {
                return Err(SvccError::new(
                    "cli",
                    "usage: svcc prove-sec <spec.{sv,hpp}> <imp.{sv,hpp}> [--latency L]",
                ));
            }
            let latency = parse_opt_flag(&args[1..], "--latency")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            cmd_prove_sec(&pos[0], &pos[1], latency)
        }
        "bench" => {
            let input = args
                .get(1)
                .ok_or_else(|| SvccError::new("cli", "usage: svcc bench <in.sv> [--iters N]"))?;
            let iters = parse_opt_flag(&args[2..], "--iters")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(50);
            cmd_bench(&reg, input, iters)
        }
        other => Err(SvccError::new(
            "cli",
            format!("unknown subcommand '{other}' (run `svcc --help`)"),
        )),
    }
}

fn print_usage() {
    println!(
        "svcc 0.2.0 — Bidirectional IEEE 1800-2023 SV (RTL + UVM) <-> C++17 SystemC Compiler\n\
         \n\
         USAGE:\n\
         \x20 svcc targets                                          List SV/C++ capability profiles\n\
         \x20 svcc sv2sc <in.sv> [-o <out.hpp>]                     Transpile SV (RTL/UVM) -> C++17 SystemC\n\
         \x20 svcc rtl2sc <in.sv> [--top M] [-o <out.hpp>]            Convert synthesizable SV RTL -> SystemC (typed, elaborated)\n\
         \x20 svcc rtl-cosim <in.sv> [--top M] [--steps N]            Simulate generated SystemC against Verilator\n\
         \x20 svcc sc2sv <in.hpp> [-o <out.sv>]                     Transpile C++17 SystemC -> IEEE 1800-2023 SV\n\
         \x20 svcc c2rtl <golden.hpp> [-o <spec.sv>]                Compile untimed C++17 golden model -> SSA SV RTL\n\
         \x20 svcc roundtrip-check <in.sv> [--shims <dir>]          SV -> C++17 (clang++) -> SV (svlang) + Z3 CRV/SVA/SEC\n\
         \x20 svcc prove-equiv <lhs> <rhs>                          Prove Z3 QF_BV CRV/Enum/Dist & UCIS equivalence\n\
         \x20 svcc prove-sec <spec.sv|hpp> <imp.sv> [--latency L]   Prove Btor2 + Z3 Sequential Equivalence (SEC/C2RTL)\n\
         \x20 svcc bench <in.sv> [--iters 50]                       Emit JSON latency & formal equivalence telemetry"
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

fn cmd_targets(reg: &LanguageRegistry) -> Result<(), SvccError> {
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

fn cmd_sv2sc_ext(
    input_path: &str,
    out_path: Option<&str>,
    partition_clauses: Option<usize>,
    no_cst_fallback: bool,
    no_dist_lcm: bool,
    force_bitmask_sva: bool,
) -> Result<(), SvccError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {input_path}: {e}")))?;
    let fe = if no_cst_fallback {
        SvFrontend::strict_semantic_only()
    } else {
        SvFrontend::new()
    };
    let mut emitter = ScEmitter::new();
    emitter.max_clauses_per_constraint = partition_clauses;
    emitter.force_bitmask_sva = force_bitmask_sva;

    let mut caps = emitter.capabilities();
    if no_dist_lcm {
        caps.exact_dist_split_weights = true;
    }

    let (mut module, _) = fe.lower_str(input_path, &src)?;
    PassPipeline::for_target(caps).run_all(&mut module)?;
    let emitted = svcc::emit_sc_with_typed_rtl(input_path, &src, module, &emitter)?;

    if let Some(p) = out_path {
        fs::write(p, &emitted)
            .map_err(|e| SvccError::new("io", format!("cannot write {p}: {e}")))?;
    } else {
        print!("{emitted}");
    }
    Ok(())
}

fn cmd_transpile(
    reg: &LanguageRegistry,
    from: &str,
    to: &str,
    input_path: &str,
    out_path: Option<&str>,
) -> Result<(), SvccError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {input_path}: {e}")))?;
    let fe = reg
        .frontend(from)
        .ok_or_else(|| SvccError::new("cli", format!("unknown frontend '{from}'")))?;
    let be = reg
        .backend(to)
        .ok_or_else(|| SvccError::new("cli", format!("unknown backend '{to}'")))?;

    let (mut module, _) = fe.lower_str(input_path, &src)?;
    PassPipeline::for_target(be.capabilities()).run_all(&mut module)?;

    let emitted = if to == "sv-uvm" {
        SvEmitter::new().emit_and_verify(&module)?
    } else {
        be.emit(&module)?
    };

    if let Some(p) = out_path {
        fs::write(p, &emitted)
            .map_err(|e| SvccError::new("io", format!("cannot write {p}: {e}")))?;
    } else {
        print!("{emitted}");
    }
    Ok(())
}

fn cmd_roundtrip_check(
    reg: &LanguageRegistry,
    input_path: &str,
    shim_dir: Option<&str>,
) -> Result<(), SvccError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {input_path}: {e}")))?;
    let sv_fe = reg.frontend("sv-uvm").unwrap();
    let sc_fe = reg.frontend("sc-uvm").unwrap();
    let sc_em = ScEmitter::new();
    let sc_caps = TargetCapabilities::sc_uvm_crave_fc4sc();

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

    let (mut m_sc, _) = sc_fe.lower_str("roundtrip.hpp", &sc_code)?;
    PassPipeline::for_target(TargetCapabilities::sv_uvm_1800_2023()).run_all(&mut m_sc)?;
    let sv_roundtrip = SvEmitter::new().emit_and_verify(&m_sc)?;

    let (mut m_sv2, _) = sv_fe.lower_str("roundtrip.sv", &sv_roundtrip)?;
    PassPipeline::for_target(sc_caps).run_all(&mut m_sv2)?;

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
            return Err(SvccError::new(
                "roundtrip",
                format!("equivalence mismatch in '{}'", rep.class_name),
            ));
        }
    }

    for prop in &m_sv.properties {
        let bmc = if prop.is_cover {
            prove_sva_cover_multiplicity_bmc(prop, 6, true)?
        } else {
            prove_sva_bmc_equivalence(prop, 6)?
        };
        println!(
            "[PASS] sva   '{}': BMC({} cycles)={:?} (z3 {} us, {} B)",
            bmc.property_name, bmc.bound_cycles, bmc.verdict, bmc.solver_time_us, bmc.smtlib2_bytes
        );
        if bmc.verdict == SmtVerdict::CounterexampleSat {
            return Err(SvccError::new(
                "roundtrip/sva",
                format!("SVA observer BMC mismatch in '{}'", prop.name),
            ));
        }
    }

    for (orig_rtl, rt_rtl) in m_sv.rtl_modules.iter().zip(m_sv2.rtl_modules.iter()) {
        let sec = prove_rtl_sec_equivalence(orig_rtl, &src, rt_rtl, &sv_roundtrip, 0, 4)?;
        println!(
            "[PASS] rtl   '{}': SEC(lat={}, {} steps, {} btor2 lines)={:?} (z3 {} us)",
            sec.spec_module,
            sec.latency_cycles,
            sec.bmc_bound_cycles,
            sec.btor2_lines,
            sec.verdict,
            sec.solver_time_us
        );
        if sec.verdict == SmtVerdict::CounterexampleSat {
            return Err(SvccError::new(
                "roundtrip/rtl",
                format!("RTL SEC mismatch in '{}'", orig_rtl.name),
            ));
        }
    }

    Ok(())
}

fn cmd_prove_equiv(
    reg: &LanguageRegistry,
    lhs_path: &str,
    rhs_path: &str,
) -> Result<(), SvccError> {
    let lhs_src = fs::read_to_string(lhs_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {lhs_path}: {e}")))?;
    let rhs_src = fs::read_to_string(rhs_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {rhs_path}: {e}")))?;

    let lhs_fe = reg.frontend(detect_lang(lhs_path)).unwrap();
    let rhs_fe = reg.frontend(detect_lang(rhs_path)).unwrap();

    let (mut m_l, _) = lhs_fe.lower_str(lhs_path, &lhs_src)?;
    let (mut m_r, _) = rhs_fe.lower_str(rhs_path, &rhs_src)?;

    let norm = PassPipeline::for_target(TargetCapabilities::sc_uvm_crave_fc4sc());
    norm.run_all(&mut m_l)?;
    norm.run_all(&mut m_r)?;

    for cls_l in &m_l.classes {
        let cls_r = m_r.find_class(&cls_l.name).ok_or_else(|| {
            SvccError::new(
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
            return Err(SvccError::new("equiv", "equivalence check failed"));
        }
    }

    for prop_l in &m_l.properties {
        let prop_r = m_r
            .properties
            .iter()
            .find(|p| p.name == prop_l.name)
            .ok_or_else(|| {
                SvccError::new(
                    "equiv/sva",
                    format!("SVA property '{}' missing in {rhs_path}", prop_l.name),
                )
            })?;
        let rep = prove_sva_pair_equivalence(prop_l, prop_r, 6)?;
        println!("{}", serde_json::to_string_pretty(&rep).unwrap());
        if rep.verdict == SmtVerdict::CounterexampleSat {
            return Err(SvccError::new("equiv/sva", "SVA equivalence check failed"));
        }
    }
    Ok(())
}

fn cmd_prove_sec(spec_path: &str, imp_path: &str, latency: u32) -> Result<(), SvccError> {
    let spec_src = fs::read_to_string(spec_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {spec_path}: {e}")))?;
    let imp_src = fs::read_to_string(imp_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {imp_path}: {e}")))?;

    let rep = prove_rtl_or_c2rtl_sec(spec_path, &spec_src, imp_path, &imp_src, latency)?;
    println!("{}", serde_json::to_string_pretty(&rep).unwrap());
    if rep.verdict == SmtVerdict::CounterexampleSat {
        return Err(SvccError::new(
            "sec",
            format!(
                "sequential equivalence check FAILED (SAT counterexample) between '{}' and '{}'",
                rep.spec_module, rep.imp_module
            ),
        ));
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
    rtl_modules: usize,
    enums: usize,
    clockings: usize,
    sva_properties: usize,
    crv_nodes: usize,
    z3_crv_verdict: SmtVerdict,
    z3_sva_bmc_verdict: SmtVerdict,
    z3_rtl_sec_verdict: SmtVerdict,
    z3_total_solver_us: u64,
    coverage_isomorphic: bool,
}

fn cmd_bench(reg: &LanguageRegistry, input_path: &str, iters: usize) -> Result<(), SvccError> {
    let src = fs::read_to_string(input_path)
        .map_err(|e| SvccError::new("io", format!("cannot read {input_path}: {e}")))?;
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

    let mut rtl_verdict = SmtVerdict::EquivalentUnsat;
    for (orig_rtl, rt_rtl) in m_sv.rtl_modules.iter().zip(m_sc.rtl_modules.iter()) {
        let sec = prove_rtl_sec_equivalence(orig_rtl, &src, rt_rtl, &sv_code, 0, 4)?;
        z3_total_us += sec.solver_time_us;
        if sec.verdict != SmtVerdict::EquivalentUnsat {
            rtl_verdict = sec.verdict;
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
        rtl_modules: m_sv.rtl_modules.len(),
        enums: m_sv.enums.len(),
        clockings: m_sv.clockings.len(),
        sva_properties: m_sv.properties.len(),
        crv_nodes: m_sv.crv_arena.len(),
        z3_crv_verdict: crv_verdict,
        z3_sva_bmc_verdict: sva_verdict,
        z3_rtl_sec_verdict: rtl_verdict,
        z3_total_solver_us: z3_total_us,
        coverage_isomorphic: cov_iso,
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}
