//! Synthesizable IEEE 1800-2023 SystemVerilog `module` -> `svcc_ir::RtlModule` lowering.

use std::collections::BTreeSet;
use svcc_ir::{
    Provenance, RtlCombBlock, RtlModule, RtlPort, RtlPortDir, RtlSignal, RtlStmt, RtlSyncBlock,
    SourceLang,
};
use svcc_svlang::syntax::{NodeId, SyntaxKind, Tree};

pub fn try_lower_sv_module(filename: &str, tree: &Tree, mod_node: NodeId) -> Option<RtlModule> {
    let name = tree
        .child(mod_node, SyntaxKind::Name)
        .map(|n| tree.bare_text(n).trim().to_string())?;
    if name == "svcc_sva_assertions" {
        return None;
    }

    let mut ports = Vec::new();
    let plist_opt = tree
        .child(mod_node, SyntaxKind::Header)
        .and_then(|h| tree.child(h, SyntaxKind::PortList))
        .or_else(|| tree.child(mod_node, SyntaxKind::PortList));
    if let Some(plist) = plist_opt {
        let mut cur_dir = RtlPortDir::Input;
        for ch in tree.children(plist) {
            if tree.kind(ch) == SyntaxKind::Port {
                if let Some(p) = parse_ansi_port(&tree.text(ch), &mut cur_dir) {
                    ports.push(p);
                }
            }
        }
    }

    let mut signals = Vec::new();
    let mut comb_blocks = Vec::new();
    let mut sync_blocks = Vec::new();

    for ch in tree.children(mod_node) {
        match tree.kind(ch) {
            SyntaxKind::DataDecl => {
                parse_internal_signals(&tree.text(ch), &ports, &mut signals, &mut comb_blocks);
            }
            SyntaxKind::ContinuousAssign => {
                let raw = tree.text(ch);
                if let Some(rest) = raw.trim().strip_prefix("assign ") {
                    let clean = rest.trim().trim_end_matches(';');
                    if let Some((lhs, rhs)) = clean.split_once('=') {
                        let stmts = vec![RtlStmt::Assign {
                            lhs: lhs.trim().to_string(),
                            rhs: normalize_sv_rtl_expr(rhs.trim()),
                            nonblocking: false,
                        }];
                        let sens = infer_comb_sensitivity(&stmts, &ports, &signals);
                        comb_blocks.push(RtlCombBlock {
                            name: format!("comb_assign_{}", comb_blocks.len()),
                            sensitivity: sens,
                            stmts,
                        });
                    }
                }
            }
            SyntaxKind::ProceduralBlock => {
                let raw = tree.text(ch);
                let trimmed = raw.trim();
                if trimmed.starts_with("always_comb") || trimmed.starts_with("always @(*") {
                    let stmts = lower_proc_block_stmts(tree, ch);
                    if !stmts.is_empty() {
                        let sens = infer_comb_sensitivity(&stmts, &ports, &signals);
                        comb_blocks.push(RtlCombBlock {
                            name: format!("comb_proc_{}", comb_blocks.len()),
                            sensitivity: sens,
                            stmts,
                        });
                    }
                } else if trimmed.starts_with("always_ff")
                    || trimmed.starts_with("always @(posedge")
                {
                    if let Some(sync) = lower_sync_proc_block(tree, ch, sync_blocks.len()) {
                        sync_blocks.push(sync);
                    }
                }
            }
            _ => {}
        }
    }

    // Pure SVA bind/checker modules have zero RTL comb/sync blocks and zero output ports
    let has_outputs = ports.iter().any(|p| p.dir == RtlPortDir::Output);
    if comb_blocks.is_empty() && sync_blocks.is_empty() && !has_outputs {
        return None;
    }

    // Mark signals assigned inside `sync_blocks` as `is_reg = true`
    let mut sync_lhs = BTreeSet::new();
    for sb in &sync_blocks {
        collect_assigned_names(&sb.reset_stmts, &mut sync_lhs);
        collect_assigned_names(&sb.body_stmts, &mut sync_lhs);
    }
    for sig in &mut signals {
        if sync_lhs.contains(&sig.name) {
            sig.is_reg = true;
        }
    }

    Some(RtlModule {
        name,
        ports,
        signals,
        comb_blocks,
        sync_blocks,
        provenance: Provenance {
            source_lang: Some(SourceLang::SystemVerilogUvm),
            file: filename.to_string(),
            line: 1,
            original_macros: Vec::new(),
        },
    })
}

fn parse_ansi_port(raw: &str, cur_dir: &mut RtlPortDir) -> Option<RtlPort> {
    let mut s = raw.trim();
    if let Some(r) = s.strip_prefix("input") {
        *cur_dir = RtlPortDir::Input;
        s = r.trim();
    } else if let Some(r) = s.strip_prefix("output") {
        *cur_dir = RtlPortDir::Output;
        s = r.trim();
    }
    let signed = s.split_whitespace().any(|w| w == "signed");
    let width = parse_packed_width(s);
    let pname = s
        .split(|c: char| c.is_whitespace() || c == ']')
        .rfind(|tok| !tok.is_empty())?
        .trim();
    if pname.is_empty() {
        return None;
    }
    Some(RtlPort {
        name: pname.to_string(),
        dir: *cur_dir,
        width,
        signed,
    })
}

fn strip_rtl_comments(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if raw[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if raw[i..].starts_with("/*") {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(' ');
        } else {
            let ch = raw[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn parse_internal_signals(
    raw: &str,
    ports: &[RtlPort],
    out: &mut Vec<RtlSignal>,
    comb_blocks: &mut Vec<RtlCombBlock>,
) {
    let cleaned = strip_rtl_comments(raw);
    let s = cleaned.trim().trim_end_matches(';');
    if !(s.starts_with("logic")
        || s.starts_with("reg")
        || s.starts_with("wire")
        || s.starts_with("bit"))
    {
        return;
    }
    let before_eq = s.split('=').next().unwrap_or(s);
    let signed = before_eq.split_whitespace().any(|w| w == "signed");
    let width = parse_packed_width(before_eq);
    let after_ty: String = if let Some(rb_pos) = before_eq.rfind(']') {
        s[rb_pos + 1..].to_string()
    } else {
        s.split_whitespace().skip(1).collect::<Vec<_>>().join(" ")
    };
    for decl in after_ty.split(',') {
        let mut parts = decl.splitn(2, '=');
        let nm = parts.next().unwrap_or("").trim();
        let init_rhs = parts.next().map(str::trim);
        if !nm.is_empty() && !ports.iter().any(|p| p.name == nm) {
            out.push(RtlSignal {
                name: nm.to_string(),
                width,
                signed,
                is_reg: false,
            });
            if let Some(rhs) = init_rhs.filter(|r| !r.is_empty()) {
                let stmts = vec![RtlStmt::Assign {
                    lhs: nm.to_string(),
                    rhs: normalize_sv_rtl_expr(rhs),
                    nonblocking: false,
                }];
                let sens = infer_comb_sensitivity(&stmts, ports, out);
                comb_blocks.push(RtlCombBlock {
                    name: format!("comb_wire_init_{}", comb_blocks.len()),
                    sensitivity: sens,
                    stmts,
                });
            }
        }
    }
}

fn parse_packed_width(s: &str) -> u32 {
    if let (Some(lb), Some(rb)) = (s.find('['), s.find(']')) {
        let inner = &s[lb + 1..rb];
        if let Some((m, l)) = inner.split_once(':') {
            let msb = m.trim().parse::<i64>().unwrap_or(0);
            let lsb = l.trim().parse::<i64>().unwrap_or(0);
            return ((msb - lsb).unsigned_abs() as u32) + 1;
        }
    }
    1
}

fn lower_sync_proc_block(tree: &Tree, proc_node: NodeId, idx: usize) -> Option<RtlSyncBlock> {
    let raw = tree.text(proc_node);
    let mut clock = "clk".to_string();
    let mut reset = None;
    let mut reset_active_low = true;

    if let (Some(at), Some(rp)) = (raw.find("@("), raw.find(')')) {
        let ev = &raw[at + 2..rp];
        for part in ev.split("or").flat_map(|p| p.split(',')) {
            let pt = part.trim();
            if let Some(c) = pt.strip_prefix("posedge ") {
                let sig = c.trim();
                if sig.contains("rst") || sig.contains("reset") {
                    reset = Some(sig.to_string());
                    reset_active_low = false;
                } else {
                    clock = sig.to_string();
                }
            } else if let Some(r) = pt.strip_prefix("negedge ") {
                reset = Some(r.trim().to_string());
                reset_active_low = true;
            }
        }
    }

    let all_stmts = lower_proc_block_stmts(tree, proc_node);
    // Split top-level `if (!rst_n) <reset_stmts> else <body_stmts>` when present
    if let [RtlStmt::IfElse {
        cond,
        then_body,
        else_body,
    }] = all_stmts.as_slice()
    {
        let c_trim = cond
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();
        let looks_like_rst = c_trim.contains("rst") || c_trim.contains("reset");
        if looks_like_rst && !else_body.is_empty() {
            if reset.is_none() {
                if let Some(r) = c_trim.strip_prefix('!') {
                    reset = Some(r.trim().to_string());
                    reset_active_low = true;
                } else {
                    reset = Some(c_trim.to_string());
                    reset_active_low = false;
                }
            }
            return Some(RtlSyncBlock {
                name: format!("sync_ff_{idx}"),
                clock,
                reset,
                reset_active_low,
                reset_stmts: then_body.clone(),
                body_stmts: else_body.clone(),
            });
        }
    }

    Some(RtlSyncBlock {
        name: format!("sync_ff_{idx}"),
        clock,
        reset,
        reset_active_low,
        reset_stmts: Vec::new(),
        body_stmts: all_stmts,
    })
}

fn lower_proc_block_stmts(tree: &Tree, node: NodeId) -> Vec<RtlStmt> {
    let mut out = Vec::new();
    for ch in tree.children(node) {
        lower_rtl_stmt_node(tree, ch, &mut out);
    }
    out
}

fn lower_rtl_stmt_node(tree: &Tree, node: NodeId, out: &mut Vec<RtlStmt>) {
    match tree.kind(node) {
        SyntaxKind::TimedStmt | SyntaxKind::SeqBlock | SyntaxKind::Body => {
            for ch in tree.children(node) {
                if tree.kind(ch) != SyntaxKind::TimingControl {
                    lower_rtl_stmt_node(tree, ch, out);
                }
            }
        }
        SyntaxKind::IfStmt => {
            let raw = tree.text(node);
            let cond = extract_first_paren_group(&raw).unwrap_or_else(|| "1".into());
            let mut then_body = Vec::new();
            let mut else_body = Vec::new();
            for ch in tree.children(node) {
                if tree.kind(ch) == SyntaxKind::ElseClause {
                    for ech in tree.children(ch) {
                        lower_rtl_stmt_node(tree, ech, &mut else_body);
                    }
                } else if matches!(
                    tree.kind(ch),
                    SyntaxKind::SeqBlock | SyntaxKind::ExprStmt | SyntaxKind::IfStmt
                ) {
                    lower_rtl_stmt_node(tree, ch, &mut then_body);
                }
            }
            out.push(RtlStmt::IfElse {
                cond: normalize_sv_rtl_expr(&cond),
                then_body,
                else_body,
            });
        }
        SyntaxKind::ExprStmt => {
            let txt = tree.text(node);
            let s = txt.trim().trim_end_matches(';').trim();
            if let Some((lhs, rhs)) = s.split_once("<=") {
                if !s.contains("==") {
                    out.push(RtlStmt::Assign {
                        lhs: lhs.trim().to_string(),
                        rhs: normalize_sv_rtl_expr(rhs.trim()),
                        nonblocking: true,
                    });
                    return;
                }
            }
            if let Some((lhs, rhs)) = split_single_assign(s) {
                out.push(RtlStmt::Assign {
                    lhs: lhs.trim().to_string(),
                    rhs: normalize_sv_rtl_expr(rhs.trim()),
                    nonblocking: false,
                });
            }
        }
        _ => {}
    }
}

fn split_single_assign(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'=' {
            let prev = (i > 0).then(|| bytes[i - 1]);
            let next = bytes.get(i + 1).copied();
            if matches!(prev, Some(b'=' | b'!' | b'<' | b'>')) || next == Some(b'=') {
                continue;
            }
            return Some((&s[..i], &s[i + 1..]));
        }
    }
    None
}

fn extract_first_paren_group(s: &str) -> Option<String> {
    let lp = s.find('(')?;
    let mut depth = 0usize;
    for (i, ch) in s[lp..].char_indices() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(s[lp + 1..lp + i].trim().to_string());
            }
        }
    }
    None
}

/// Normalizes SV sized literals (`32'h0`, `8'd1`, `2'd2`) inside RTL expressions into C/SV-neutral
/// integer literals (`0x0U`, `1U`, `2U`) so `SV <-> C++17` expression text translates cleanly.
pub fn normalize_sv_rtl_expr(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() || bytes[i] == b'\'' {
            let start = i;
            if bytes[i] == b'\'' {
                i += 1;
            }
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'\'')
            {
                i += 1;
            }
            let tok = &raw[start..i];
            if tok.contains('\'') {
                if let Ok((val, _, _)) = crate::crv_parser::parse_sv_literal(tok) {
                    out.push_str(&format!("{val}U"));
                    continue;
                }
            }
            out.push_str(tok);
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn collect_assigned_names(stmts: &[RtlStmt], out: &mut BTreeSet<String>) {
    for st in stmts {
        match st {
            RtlStmt::Assign { lhs, .. } => {
                out.insert(lhs.clone());
            }
            RtlStmt::IfElse {
                then_body,
                else_body,
                ..
            } => {
                collect_assigned_names(then_body, out);
                collect_assigned_names(else_body, out);
            }
        }
    }
}

pub fn infer_comb_sensitivity(
    stmts: &[RtlStmt],
    ports: &[RtlPort],
    signals: &[RtlSignal],
) -> Vec<String> {
    let known: BTreeSet<&str> = ports
        .iter()
        .filter(|p| p.dir == RtlPortDir::Input)
        .map(|p| p.name.as_str())
        .chain(signals.iter().map(|s| s.name.as_str()))
        .collect();
    let mut used = BTreeSet::new();
    collect_read_idents(stmts, &known, &mut used);
    used.into_iter().collect()
}

fn collect_read_idents(stmts: &[RtlStmt], known: &BTreeSet<&str>, out: &mut BTreeSet<String>) {
    for st in stmts {
        match st {
            RtlStmt::Assign { rhs, .. } => scan_idents_in_expr(rhs, known, out),
            RtlStmt::IfElse {
                cond,
                then_body,
                else_body,
            } => {
                scan_idents_in_expr(cond, known, out);
                collect_read_idents(then_body, known, out);
                collect_read_idents(else_body, known, out);
            }
        }
    }
}

fn scan_idents_in_expr(expr: &str, known: &BTreeSet<&str>, out: &mut BTreeSet<String>) {
    for tok in expr.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if !tok.is_empty() && known.contains(tok) {
            out.insert(tok.to_string());
        }
    }
}
