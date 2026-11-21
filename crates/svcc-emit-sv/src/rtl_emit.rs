//! Synthesizable IEEE 1800-2023 `module` emitter for `svcc_ir::RtlModule`.

use std::fmt::Write;
use svcc_ir::{RtlModule, RtlPortDir, RtlStmt};

pub fn emit_sv_rtl_module(out: &mut String, m: &RtlModule) {
    if !m.provenance.file.is_empty() {
        let _ = writeln!(
            out,
            "(* svcc_src = \"{}:{}\" *)",
            m.provenance.file, m.provenance.line
        );
    }
    let _ = writeln!(out, "module {} (", m.name);
    for (i, p) in m.ports.iter().enumerate() {
        let dir = match p.dir {
            RtlPortDir::Input => "input",
            RtlPortDir::Output => "output",
        };
        let comma = if i + 1 < m.ports.len() { "," } else { "" };
        let _ = writeln!(
            out,
            "  {} {}{}",
            dir,
            format_sv_rtl_decl(&p.name, p.width, p.signed),
            comma
        );
    }
    out.push_str(");\n");

    for sig in &m.signals {
        let _ = writeln!(
            out,
            "  {};",
            format_sv_rtl_decl(&sig.name, sig.width, sig.signed)
        );
    }
    if !m.signals.is_empty() {
        out.push('\n');
    }

    for cb in &m.comb_blocks {
        out.push_str("  always_comb begin\n");
        emit_sv_rtl_stmts(out, &cb.stmts, false, "    ");
        out.push_str("  end\n\n");
    }

    for sb in &m.sync_blocks {
        if let Some(rst) = &sb.reset {
            let edge = if sb.reset_active_low {
                "negedge"
            } else {
                "posedge"
            };
            let _ = writeln!(
                out,
                "  always_ff @(posedge {} or {} {}) begin",
                sb.clock, edge, rst
            );
            let rst_cond = if sb.reset_active_low {
                format!("!{rst}")
            } else {
                rst.clone()
            };
            let _ = writeln!(out, "    if ({rst_cond}) begin");
            emit_sv_rtl_stmts(out, &sb.reset_stmts, true, "      ");
            out.push_str("    end else begin\n");
            emit_sv_rtl_stmts(out, &sb.body_stmts, true, "      ");
            out.push_str("    end\n  end\n\n");
        } else {
            let _ = writeln!(out, "  always_ff @(posedge {}) begin", sb.clock);
            emit_sv_rtl_stmts(out, &sb.body_stmts, true, "    ");
            out.push_str("  end\n\n");
        }
    }

    out.push_str("endmodule\n\n");
}

fn format_sv_rtl_decl(name: &str, width: u32, signed: bool) -> String {
    let s = if signed { " signed" } else { "" };
    if width <= 1 {
        format!("logic{s} {name}")
    } else {
        format!("logic{s} [{}:0] {name}", width - 1)
    }
}

fn emit_sv_rtl_stmts(out: &mut String, stmts: &[RtlStmt], force_nba: bool, indent: &str) {
    for st in stmts {
        match st {
            RtlStmt::Assign {
                lhs,
                rhs,
                nonblocking,
            } => {
                let op = if force_nba || *nonblocking { "<=" } else { "=" };
                let sv_rhs = strip_cpp_unsigned_suffix(rhs);
                let _ = writeln!(out, "{indent}{lhs} {op} {sv_rhs};");
            }
            RtlStmt::IfElse {
                cond,
                then_body,
                else_body,
            } => {
                let sv_cond = strip_cpp_unsigned_suffix(cond);
                let _ = writeln!(out, "{indent}if ({sv_cond}) begin");
                emit_sv_rtl_stmts(out, then_body, force_nba, &format!("{indent}  "));
                if else_body.is_empty() {
                    let _ = writeln!(out, "{indent}end");
                } else {
                    let _ = writeln!(out, "{indent}end else begin");
                    emit_sv_rtl_stmts(out, else_body, force_nba, &format!("{indent}  "));
                    let _ = writeln!(out, "{indent}end");
                }
            }
        }
    }
}

/// Converts C++ unsigned integer literals (`0U`, `0xFFU`, `3ULL`) into valid IEEE 1800 literals (`0`, `'hFF`, `3`).
fn strip_cpp_unsigned_suffix(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let tok = &expr[start..i];
            let trimmed = tok.trim_end_matches(['u', 'U', 'l', 'L']);
            if let Some(hex) = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
            {
                out.push_str(&format!("'h{hex}"));
            } else {
                out.push_str(trimmed);
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
