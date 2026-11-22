//! Synthesizable IEEE 1666.1 SystemC `SC_MODULE` code emitter for `svcc_ir::RtlModule`.

use std::collections::BTreeSet;
use std::fmt::Write;
use svcc_ir::{RtlModule, RtlPortDir, RtlStmt};

pub fn emit_sc_rtl_module(out: &mut String, m: &RtlModule) {
    let _ = writeln!(out, "// svcc:rtl_module {}", m.name);
    let _ = writeln!(out, "SC_MODULE({}) {{", m.name);

    for p in &m.ports {
        let sc_ty = format_sc_rtl_sig_type(p.width, p.signed);
        let dir_cls = match p.dir {
            RtlPortDir::Input => "sc_core::sc_in",
            RtlPortDir::Output => "sc_core::sc_out",
        };
        let _ = writeln!(
            out,
            "  {}<{}> {}{{\"{}\"}};",
            dir_cls, sc_ty, p.name, p.name
        );
    }
    for sig in &m.signals {
        let sc_ty = format_sc_rtl_sig_type(sig.width, sig.signed);
        let _ = writeln!(
            out,
            "  sc_core::sc_signal<{}> {}{{\"{}\"}};",
            sc_ty, sig.name, sig.name
        );
    }
    out.push('\n');

    let sig_names: BTreeSet<&str> = m
        .ports
        .iter()
        .map(|p| p.name.as_str())
        .chain(m.signals.iter().map(|s| s.name.as_str()))
        .collect();

    for cb in &m.comb_blocks {
        let _ = writeln!(out, "  void {}() {{", cb.name);
        emit_sc_rtl_stmts(out, &cb.stmts, &sig_names, "    ");
        out.push_str("  }\n\n");
    }

    for sb in &m.sync_blocks {
        let _ = writeln!(out, "  void {}() {{", sb.name);
        if !sb.reset_stmts.is_empty() {
            out.push_str("    // svcc:reset_begin\n");
            emit_sc_rtl_stmts(out, &sb.reset_stmts, &sig_names, "    ");
            out.push_str("    // svcc:reset_end\n");
        }
        out.push_str("    sc_core::wait();\n");
        out.push_str("    while (true) {\n");
        emit_sc_rtl_stmts(out, &sb.body_stmts, &sig_names, "      ");
        out.push_str("      sc_core::wait();\n");
        out.push_str("    }\n  }\n\n");
    }

    let _ = writeln!(out, "  SC_CTOR({}) {{", m.name);
    for cb in &m.comb_blocks {
        let _ = writeln!(out, "    SC_METHOD({});", cb.name);
        if !cb.sensitivity.is_empty() {
            let chain = cb.sensitivity.join(" << ");
            let _ = writeln!(out, "    sensitive << {};", chain);
        }
    }
    for sb in &m.sync_blocks {
        let _ = writeln!(out, "    SC_CTHREAD({}, {}.pos());", sb.name, sb.clock);
        if let Some(rst) = &sb.reset {
            let level = if sb.reset_active_low { "false" } else { "true" };
            let _ = writeln!(out, "    async_reset_signal_is({}, {});", rst, level);
        }
    }
    out.push_str("  }\n};\n\n");
}

fn format_sc_rtl_sig_type(width: u32, signed: bool) -> String {
    if width <= 1 && !signed {
        "bool".into()
    } else if signed {
        format!("sc_dt::sc_int<{}>", width)
    } else {
        format!("sc_dt::sc_uint<{}>", width)
    }
}

fn emit_sc_rtl_stmts(
    out: &mut String,
    stmts: &[RtlStmt],
    sig_names: &BTreeSet<&str>,
    indent: &str,
) {
    for st in stmts {
        match st {
            RtlStmt::Assign { lhs, rhs, .. } => {
                let clean_lhs = lhs
                    .lines()
                    .map(|l| l.split("//").next().unwrap_or("").trim())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let clean_rhs = rhs
                    .lines()
                    .map(|l| l.split("//").next().unwrap_or("").trim())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let _ = writeln!(out, "{indent}// svcc:rtl_assign {clean_lhs} = {clean_rhs}");
                let cpp_rhs = rewrite_reads_for_sc(&clean_rhs, sig_names);
                if let (Some(lb), Some(rb)) = (clean_lhs.find('['), clean_lhs.rfind(']')) {
                    let base = clean_lhs[..lb].trim();
                    let idx = &clean_lhs[lb + 1..rb];
                    let _ = writeln!(
                        out,
                        "{indent}{{ auto _svcc_tmp = {base}.read(); _svcc_tmp[{idx}] = static_cast<bool>({cpp_rhs}); {base}.write(_svcc_tmp); }}"
                    );
                } else {
                    let _ = writeln!(out, "{indent}{clean_lhs}.write({cpp_rhs});");
                }
            }
            RtlStmt::IfElse {
                cond,
                then_body,
                else_body,
            } => {
                let _ = writeln!(out, "{indent}// svcc:rtl_if {cond}");
                let cpp_cond = rewrite_reads_for_sc(cond, sig_names);
                let _ = writeln!(out, "{indent}if ({cpp_cond}) {{");
                emit_sc_rtl_stmts(out, then_body, sig_names, &format!("{indent}  "));
                if else_body.is_empty() {
                    let _ = writeln!(out, "{indent}}}");
                } else {
                    let _ = writeln!(out, "{indent}}} else {{");
                    emit_sc_rtl_stmts(out, else_body, sig_names, &format!("{indent}  "));
                    let _ = writeln!(out, "{indent}}}");
                }
            }
        }
    }
}

/// Replaces bare port/signal reads `sig` with `sig.read()` inside SystemC RTL expressions.
fn rewrite_reads_for_sc(expr: &str, sig_names: &BTreeSet<&str>) -> String {
    let mut pre = expr.replace("(|", "(0U != ");
    if let Some(rest) = pre.trim().strip_prefix('^') {
        pre = format!(
            "(__builtin_parityll(static_cast<uint64_t>({})) != 0)",
            rest.trim()
        );
    } else if let Some(rest) = pre.trim().strip_prefix('|') {
        pre = format!("(({}) != 0U)", rest.trim());
    }
    let expr = pre.as_str();
    let mut out = String::with_capacity(expr.len() + 16);
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'~'
            && i + 1 < bytes.len()
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'[' {
                out.push('!');
                i += 1;
                continue;
            }
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &expr[start..i];
            if sig_names.contains(ident) {
                out.push_str(ident);
                out.push_str(".read()");
            } else {
                out.push_str(ident);
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
