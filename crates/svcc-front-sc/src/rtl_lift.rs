//! Synthesizable IEEE 1666.1 `SC_MODULE` lifter + Untimed C++17 SSA `C2RTL` golden-function
//! compiler with fail-closed IEEE 1666.1 checks.

use std::collections::{BTreeMap, BTreeSet};
use svcc_ir::{
    Provenance, RtlCombBlock, RtlModule, RtlPort, RtlPortDir, RtlSignal, RtlStmt, RtlSyncBlock,
    SourceLang, SvccError,
};

/// Enforces IEEE 1666.1 synthesizability boundaries on RTL `SC_MODULE` and `c2rtl` bodies.
fn check_ieee_1666_1_synthesizable(
    filename: &str,
    base_line: u32,
    body: &str,
) -> Result<(), SvccError> {
    let forbidden = [
        (
            "new ",
            "dynamic heap allocation 'new' is not synthesizable (IEEE 1666.1)",
        ),
        (
            "delete ",
            "dynamic heap deallocation 'delete' is not synthesizable (IEEE 1666.1)",
        ),
        (
            "malloc(",
            "C heap allocation 'malloc' is not synthesizable (IEEE 1666.1)",
        ),
        (
            "std::vector<",
            "dynamic container 'std::vector' is not synthesizable in RTL",
        ),
        (
            "std::map<",
            "dynamic container 'std::map' is not synthesizable in RTL",
        ),
    ];
    for (offset, raw_line) in body.lines().enumerate() {
        let code = raw_line.split("//").next().unwrap_or("");
        for (pat, msg) in forbidden {
            if code.contains(pat) {
                return Err(SvccError::new(
                    "svcc-front-sc/ieee1666.1",
                    format!("{}:{}: {}", filename, base_line + (offset as u32), msg),
                ));
            }
        }
    }
    Ok(())
}

/// Extracts all `SC_MODULE(name) { ... }` RTL modules from C++17 SystemC source.
pub fn extract_sc_rtl_modules(filename: &str, src: &str) -> Result<Vec<RtlModule>, SvccError> {
    let mut modules = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut line = 1u32;

    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if src[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if src[i..].starts_with("SC_MODULE(")
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
        {
            let mod_line = line;
            let after_kw = &src[i + "SC_MODULE(".len()..];
            let Some(rp) = after_kw.find(')') else {
                i += 1;
                continue;
            };
            let mname = after_kw[..rp].trim().to_string();
            while i < bytes.len() && bytes[i] != b'{' {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'{' {
                i += 1;
                let bstart = i;
                let mut depth = 1usize;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'\n' {
                        line += 1;
                    } else if bytes[i] == b'{' {
                        depth += 1;
                    } else if bytes[i] == b'}' {
                        depth -= 1;
                    }
                    i += 1;
                }
                let body = &src[bstart..i.saturating_sub(1)];
                check_ieee_1666_1_synthesizable(filename, mod_line, body)?;
                modules.push(parse_sc_module_body(filename, mod_line, mname, body)?);
                continue;
            }
        }
        i += 1;
    }
    Ok(modules)
}

fn parse_sc_module_body(
    filename: &str,
    line_no: u32,
    name: String,
    body: &str,
) -> Result<RtlModule, SvccError> {
    let mut ports = Vec::new();
    let mut signals = Vec::new();
    let mut comb_methods: Vec<String> = Vec::new();
    let mut cthreads: Vec<(String, String)> = Vec::new();
    let mut reset_spec: Option<(String, bool)> = None;

    for raw_line in body.lines() {
        let l = raw_line.split("//").next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        if let Some((w, signed, pname)) = parse_sc_port_or_sig(l, "sc_core::sc_in<", "sc_in<") {
            ports.push(RtlPort {
                name: pname,
                dir: RtlPortDir::Input,
                width: w,
                signed,
            });
        } else if let Some((w, signed, pname)) =
            parse_sc_port_or_sig(l, "sc_core::sc_out<", "sc_out<")
        {
            ports.push(RtlPort {
                name: pname,
                dir: RtlPortDir::Output,
                width: w,
                signed,
            });
        } else if let Some((w, signed, sname)) =
            parse_sc_port_or_sig(l, "sc_core::sc_signal<", "sc_signal<")
        {
            signals.push(RtlSignal {
                name: sname,
                width: w,
                signed,
                is_reg: false,
            });
        } else if let Some(m) = l
            .strip_prefix("SC_METHOD(")
            .and_then(|s| s.strip_suffix(");"))
        {
            comb_methods.push(m.trim().to_string());
        } else if let Some(inner) = l
            .strip_prefix("SC_CTHREAD(")
            .and_then(|s| s.strip_suffix(");"))
        {
            if let Some((fnm, edge)) = inner.split_once(',') {
                let clk = edge
                    .trim()
                    .trim_end_matches(".pos()")
                    .trim_end_matches(".posedge_event()")
                    .trim()
                    .to_string();
                cthreads.push((fnm.trim().to_string(), clk));
            }
        } else if let Some(inner) = l
            .strip_prefix("async_reset_signal_is(")
            .or_else(|| l.strip_prefix("reset_signal_is("))
            .and_then(|s| s.strip_suffix(");"))
        {
            if let Some((rn, lvl)) = inner.split_once(',') {
                let active_low = lvl.trim() == "false";
                reset_spec = Some((rn.trim().to_string(), active_low));
            }
        }
    }

    let mut comb_blocks = Vec::new();
    for mname in &comb_methods {
        if let Some(mbody) = extract_void_method_body(body, mname) {
            let stmts = parse_sc_rtl_stmts(&mbody, false);
            let sens = infer_rtl_sens(&stmts, &ports, &signals);
            comb_blocks.push(RtlCombBlock {
                name: mname.clone(),
                sensitivity: sens,
                stmts,
            });
        }
    }

    let mut sync_blocks = Vec::new();
    for (fnm, clk) in &cthreads {
        if let Some(mbody) = extract_void_method_body(body, fnm) {
            let (rst_stmts, loop_stmts) = split_cthread_reset_and_loop(&mbody);
            sync_blocks.push(RtlSyncBlock {
                name: fnm.clone(),
                clock: clk.clone(),
                reset: reset_spec.as_ref().map(|r| r.0.clone()),
                reset_active_low: reset_spec.as_ref().map(|r| r.1).unwrap_or(true),
                reset_stmts: rst_stmts,
                body_stmts: loop_stmts,
            });
        }
    }

    Ok(RtlModule {
        name,
        ports,
        signals,
        comb_blocks,
        sync_blocks,
        provenance: Provenance {
            source_lang: Some(SourceLang::SystemCUvm),
            file: filename.to_string(),
            line: line_no,
            original_macros: Vec::new(),
        },
    })
}

fn parse_sc_port_or_sig(line: &str, pfx1: &str, pfx2: &str) -> Option<(u32, bool, String)> {
    let after = line
        .strip_prefix(pfx1)
        .or_else(|| line.strip_prefix(pfx2))?;
    let mut depth = 1usize;
    let mut end_ty = 0usize;
    for (idx, ch) in after.char_indices() {
        if ch == '<' {
            depth += 1;
        } else if ch == '>' {
            depth -= 1;
            if depth == 0 {
                end_ty = idx;
                break;
            }
        }
    }
    if end_ty == 0 {
        return None;
    }
    let ty_s = after[..end_ty].trim();
    let (w, signed) = parse_cpp_bitwidth_and_sign(ty_s);
    let pname = after[end_ty + 1..]
        .split(['{', '(', ';', ' '])
        .find(|s| !s.is_empty())?
        .trim()
        .to_string();
    Some((w, signed, pname))
}

fn parse_cpp_bitwidth_and_sign(ty_s: &str) -> (u32, bool) {
    let t = ty_s.trim();
    match t {
        "bool" => (1, false),
        "uint8_t" => (8, false),
        "int8_t" => (8, true),
        "uint16_t" => (16, false),
        "int16_t" => (16, true),
        "uint32_t" | "unsigned" | "unsigned int" => (32, false),
        "int32_t" | "int" => (32, true),
        "uint64_t" => (64, false),
        "int64_t" => (64, true),
        _ => {
            if let (Some(lb), Some(rb)) = (t.find('<'), t.rfind('>')) {
                let w = t[lb + 1..rb]
                    .split(',')
                    .next()
                    .unwrap_or("32")
                    .trim()
                    .parse::<u32>()
                    .unwrap_or(32);
                let signed = t.contains("sc_int") || t.contains("ac_int") && t.ends_with("true>");
                (w, signed)
            } else {
                (32, false)
            }
        }
    }
}

fn extract_void_method_body(cls_body: &str, mname: &str) -> Option<String> {
    let pat = format!("void {mname}()");
    let idx = cls_body.find(&pat)?;
    let after = &cls_body[idx + pat.len()..];
    let lb = after.find('{')?;
    let mut depth = 1usize;
    for (i, ch) in after[lb + 1..].char_indices() {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(after[lb + 1..lb + 1 + i].to_string());
            }
        }
    }
    None
}

fn split_cthread_reset_and_loop(mbody: &str) -> (Vec<RtlStmt>, Vec<RtlStmt>) {
    if let Some(w_idx) = mbody.find("while") {
        let pre = &mbody[..w_idx];
        let rst_stmts = parse_sc_rtl_stmts(pre, true);
        let after_w = &mbody[w_idx..];
        let loop_stmts = if let Some(lb) = after_w.find('{') {
            let mut depth = 1usize;
            let mut end_l = after_w.len();
            for (i, ch) in after_w[lb + 1..].char_indices() {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        end_l = lb + 1 + i;
                        break;
                    }
                }
            }
            parse_sc_rtl_stmts(&after_w[lb + 1..end_l], true)
        } else {
            Vec::new()
        };
        (rst_stmts, loop_stmts)
    } else {
        (Vec::new(), parse_sc_rtl_stmts(mbody, true))
    }
}

fn parse_sc_rtl_stmts(block: &str, nonblocking: bool) -> Vec<RtlStmt> {
    let lines: Vec<&str> = block.lines().map(str::trim).collect();
    let (stmts, _) = parse_sc_rtl_lines(&lines, 0, nonblocking);
    stmts
}

fn parse_sc_rtl_lines(lines: &[&str], mut i: usize, nonblocking: bool) -> (Vec<RtlStmt>, usize) {
    let mut out = Vec::new();
    while i < lines.len() {
        let l = lines[i];
        if l.is_empty()
            || l.starts_with("//")
            || l.starts_with("sc_core::wait")
            || l.starts_with("wait(")
        {
            i += 1;
            continue;
        }
        if l == "}" || l.starts_with("} else") {
            break;
        }
        if let Some(after_if) = l.strip_prefix("if") {
            let after_if = after_if.trim();
            if after_if.starts_with('(') {
                let cond = strip_sc_reads(extract_paren(after_if).unwrap_or_default().as_str());
                let (then_body, next_i) = parse_sc_rtl_lines(lines, i + 1, nonblocking);
                i = next_i;
                let mut else_body = Vec::new();
                if i < lines.len() && lines[i].starts_with("} else") {
                    let (eb, next_e) = parse_sc_rtl_lines(lines, i + 1, nonblocking);
                    else_body = eb;
                    i = next_e;
                }
                if i < lines.len() && lines[i] == "}" {
                    i += 1;
                }
                out.push(RtlStmt::IfElse {
                    cond,
                    then_body,
                    else_body,
                });
                continue;
            }
        }
        // `<lhs>.write(<rhs>);`
        if let Some((lhs, rest)) = l.split_once(".write(") {
            if let Some(rhs) = rest.strip_suffix(");") {
                out.push(RtlStmt::Assign {
                    lhs: lhs.trim().to_string(),
                    rhs: strip_sc_reads(rhs.trim()),
                    nonblocking,
                });
            }
        }
        i += 1;
    }
    (out, i)
}

fn strip_sc_reads(s: &str) -> String {
    s.replace(".read()", "")
}

fn extract_paren(s: &str) -> Option<String> {
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

fn infer_rtl_sens(stmts: &[RtlStmt], ports: &[RtlPort], signals: &[RtlSignal]) -> Vec<String> {
    let known: BTreeSet<&str> = ports
        .iter()
        .filter(|p| p.dir == RtlPortDir::Input)
        .map(|p| p.name.as_str())
        .chain(signals.iter().map(|s| s.name.as_str()))
        .collect();
    let mut out = BTreeSet::new();
    collect_sens_rec(stmts, &known, &mut out);
    out.into_iter().collect()
}

fn collect_sens_rec(stmts: &[RtlStmt], known: &BTreeSet<&str>, out: &mut BTreeSet<String>) {
    for st in stmts {
        match st {
            RtlStmt::Assign { rhs, .. } => {
                for tok in rhs.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                    if known.contains(tok) {
                        out.insert(tok.to_string());
                    }
                }
            }
            RtlStmt::IfElse {
                cond,
                then_body,
                else_body,
            } => {
                for tok in cond.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                    if known.contains(tok) {
                        out.insert(tok.to_string());
                    }
                }
                collect_sens_rec(then_body, known, out);
                collect_sens_rec(else_body, known, out);
            }
        }
    }
}

// ============================================================================
// Mode B: Untimed C++17 Reference Function -> SSA Combinational `RtlModule` (`C2RTL`)
// ============================================================================

/// Compiles `// svcc:c2rtl`-annotated untimed C++17 golden functions into SSA combinational
/// [`RtlModule`]s with source-line provenance for `InFormal` `check_sec` / `svcc prove-sec`.
pub fn extract_c2rtl_functions(filename: &str, src: &str) -> Result<Vec<RtlModule>, SvccError> {
    let mut out = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        if lines[i].trim().starts_with("// svcc:c2rtl") {
            let fn_line = (i + 2) as u32;
            i += 1;
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            if i >= lines.len() {
                break;
            }
            // Collect signature + `{ ... }` body
            let mut chunk = String::new();
            let mut depth = 0usize;
            let mut seen_brace = false;
            let body_start_line = (i + 1) as u32;
            while i < lines.len() {
                let ln = lines[i];
                for ch in ln.chars() {
                    if ch == '{' {
                        depth += 1;
                        seen_brace = true;
                    } else if ch == '}' {
                        depth = depth.saturating_sub(1);
                    }
                }
                chunk.push_str(ln);
                chunk.push('\n');
                i += 1;
                if seen_brace && depth == 0 {
                    break;
                }
            }
            let lb = chunk
                .find('{')
                .ok_or_else(|| SvccError::new("c2rtl", "missing '{' in c2rtl function"))?;
            let rb = chunk.rfind('}').unwrap_or(chunk.len());
            let sig = chunk[..lb].trim();
            let fbody = &chunk[lb + 1..rb];
            check_ieee_1666_1_synthesizable(filename, body_start_line, fbody)?;
            out.push(lower_c2rtl_function_ssa(filename, fn_line, sig, fbody)?);
            continue;
        }
        i += 1;
    }
    Ok(out)
}

#[derive(Clone)]
struct SsaState {
    vars: BTreeMap<String, String>,
    ret_expr: Option<String>,
    ret_taken: Option<String>,
}

fn lower_c2rtl_function_ssa(
    filename: &str,
    line_no: u32,
    sig: &str,
    fbody: &str,
) -> Result<RtlModule, SvccError> {
    let lp = sig
        .find('(')
        .ok_or_else(|| SvccError::new("c2rtl", format!("bad c2rtl signature: {sig}")))?;
    let rp = sig
        .rfind(')')
        .ok_or_else(|| SvccError::new("c2rtl", format!("bad c2rtl signature: {sig}")))?;
    let ret_and_name = sig[..lp].trim();
    let last_sp = ret_and_name
        .rfind(' ')
        .ok_or_else(|| SvccError::new("c2rtl", "missing return type on c2rtl function"))?;
    let ret_ty_s = ret_and_name[..last_sp].trim();
    let fn_name = ret_and_name[last_sp + 1..].trim().to_string();
    let (ret_w, ret_signed) = parse_cpp_bitwidth_and_sign(ret_ty_s);

    let mut ports = Vec::new();
    let mut state = SsaState {
        vars: BTreeMap::new(),
        ret_expr: None,
        ret_taken: None,
    };

    let params_raw = sig[lp + 1..rp].trim();
    if !params_raw.is_empty() {
        for p_decl in split_c_params(params_raw) {
            let clean = p_decl
                .replace("const ", "")
                .replace('&', "")
                .trim()
                .to_string();
            if let Some(sp) = clean.rfind(' ') {
                let pty = clean[..sp].trim();
                let pname = clean[sp + 1..].trim().to_string();
                let (pw, ps) = parse_cpp_bitwidth_and_sign(pty);
                ports.push(RtlPort {
                    name: pname.clone(),
                    dir: RtlPortDir::Input,
                    width: pw,
                    signed: ps,
                });
                state.vars.insert(pname.clone(), pname);
            }
        }
    }
    ports.push(RtlPort {
        name: "result".into(),
        dir: RtlPortDir::Output,
        width: ret_w,
        signed: ret_signed,
    });

    let raw_lines: Vec<String> = fbody
        .lines()
        .map(|ln| ln.split("//").next().unwrap_or("").trim().to_string())
        .filter(|ln| !ln.is_empty())
        .collect();
    let line_refs: Vec<&str> = raw_lines.iter().map(String::as_str).collect();
    let (final_state, _) = eval_c2rtl_lines(&line_refs, 0, state, filename, line_no)?;

    let result_rhs = final_state.ret_expr.unwrap_or_else(|| "0".into());
    let stmts = vec![RtlStmt::Assign {
        lhs: "result".into(),
        rhs: result_rhs,
        nonblocking: false,
    }];
    let sens = infer_rtl_sens(&stmts, &ports, &[]);

    Ok(RtlModule {
        name: fn_name,
        ports,
        signals: Vec::new(),
        comb_blocks: vec![RtlCombBlock {
            name: "comb_c2rtl_ssa".into(),
            sensitivity: sens,
            stmts,
        }],
        sync_blocks: Vec::new(),
        provenance: Provenance {
            source_lang: Some(SourceLang::SystemCUvm),
            file: filename.to_string(),
            line: line_no,
            original_macros: Vec::new(),
        },
    })
}

fn split_c_params(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        if ch == '<' {
            depth += 1;
        } else if ch == '>' {
            depth = depth.saturating_sub(1);
        } else if ch == ',' && depth == 0 {
            out.push(s[start..i].trim().to_string());
            start = i + 1;
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn eval_c2rtl_lines(
    lines: &[&str],
    mut i: usize,
    mut st: SsaState,
    filename: &str,
    line_no: u32,
) -> Result<(SsaState, usize), SvccError> {
    while i < lines.len() {
        let l = lines[i];
        if l == "}" || l.starts_with("} else") {
            break;
        }

        // Bounded `for (int idx = 0; idx < N; ++idx) { ... }` unroller
        if let Some(after_for) = l.strip_prefix("for") {
            let hdr = extract_paren(after_for.trim())
                .ok_or_else(|| SvccError::new("c2rtl/loop", "malformed for-loop header"))?;
            let (ivar, lo, hi) = parse_bounded_for_header(&hdr).ok_or_else(|| {
                SvccError::new(
                    "c2rtl/loop",
                    format!("{filename}:{line_no}: C2RTL loop bound must be a compile-time constant <= 64: '{hdr}'"),
                )
            })?;
            // Locate matching `}` of loop body
            let body_start = i + 1;
            let (_, body_end) = skip_braced_lines(lines, body_start);
            let loop_slice = &lines[body_start..body_end];
            for iter_v in lo..hi {
                st.vars.insert(ivar.clone(), format!("{iter_v}"));
                let (next_st, _) = eval_c2rtl_lines(loop_slice, 0, st, filename, line_no)?;
                st = next_st;
            }
            st.vars.remove(&ivar);
            i = (body_end + 1).min(lines.len());
            continue;
        }

        // `if (cond) { ... } [else { ... }]` symbolic mux join
        if let Some(after_if) = l.strip_prefix("if") {
            let after_if = after_if.trim();
            if after_if.starts_with('(') {
                let raw_cond = extract_paren(after_if).unwrap_or_else(|| "1".into());
                let sym_cond = subst_ssa_vars(&raw_cond, &st.vars);
                let (then_st, mut next_i) =
                    eval_c2rtl_lines(lines, i + 1, st.clone(), filename, line_no)?;
                let else_st = if next_i < lines.len() && lines[next_i].starts_with("} else") {
                    let (est, ni) =
                        eval_c2rtl_lines(lines, next_i + 1, st.clone(), filename, line_no)?;
                    next_i = ni;
                    est
                } else {
                    st.clone()
                };
                if next_i < lines.len() && lines[next_i] == "}" {
                    next_i += 1;
                }
                st = mux_join_ssa(&sym_cond, &st, then_st, else_st);
                i = next_i;
                continue;
            }
        }

        // `return <expr>;`
        if let Some(ret_s) = l.strip_prefix("return ") {
            let val = subst_ssa_vars(ret_s.trim().trim_end_matches(';'), &st.vars);
            if let Some(already) = &st.ret_taken {
                let prev_ret = st.ret_expr.clone().unwrap_or_else(|| "0".into());
                st.ret_expr = Some(format!("({already} ? {prev_ret} : ({val}))"));
            } else {
                st.ret_expr = Some(format!("({val})"));
                st.ret_taken = Some("1'b1".into());
            }
            i += 1;
            continue;
        }

        // Local declaration or mutation (`uint32_t res = a + b;`, `res += c;`, `res = a - b;`)
        let stmt = l.trim_end_matches(';').trim();
        if let Some((lhs_raw, op_kind, rhs_raw)) = split_c_assignment(stmt) {
            let vname = lhs_raw
                .split_whitespace()
                .next_back()
                .unwrap_or(lhs_raw)
                .trim()
                .to_string();
            let rhs_sym = subst_ssa_vars(rhs_raw, &st.vars);
            let combined = match op_kind {
                "=" => format!("({rhs_sym})"),
                "+=" => {
                    let prev = st.vars.get(&vname).cloned().unwrap_or_else(|| "0".into());
                    format!("({prev} + ({rhs_sym}))")
                }
                "-=" => {
                    let prev = st.vars.get(&vname).cloned().unwrap_or_else(|| "0".into());
                    format!("({prev} - ({rhs_sym}))")
                }
                "^=" => {
                    let prev = st.vars.get(&vname).cloned().unwrap_or_else(|| "0".into());
                    format!("({prev} ^ ({rhs_sym}))")
                }
                "|=" => {
                    let prev = st.vars.get(&vname).cloned().unwrap_or_else(|| "0".into());
                    format!("({prev} | ({rhs_sym}))")
                }
                "&=" => {
                    let prev = st.vars.get(&vname).cloned().unwrap_or_else(|| "0".into());
                    format!("({prev} & ({rhs_sym}))")
                }
                _ => rhs_sym,
            };
            let guarded = if let Some(already) = &st.ret_taken {
                let prev = st.vars.get(&vname).cloned().unwrap_or_else(|| "0".into());
                format!("({already} ? {prev} : {combined})")
            } else {
                combined
            };
            st.vars.insert(vname, guarded);
        }

        i += 1;
    }
    Ok((st, i))
}

fn parse_bounded_for_header(hdr: &str) -> Option<(String, u32, u32)> {
    let parts: Vec<&str> = hdr.split(';').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }
    let (init_l, init_r) = parts[0].split_once('=')?;
    let ivar = init_l.split_whitespace().next_back()?.trim().to_string();
    let lo = init_r
        .trim()
        .trim_end_matches(['u', 'U'])
        .parse::<u32>()
        .ok()?;
    let (_, bound_s) = parts[1].split_once('<')?;
    let hi = bound_s
        .trim()
        .trim_start_matches('=')
        .trim()
        .trim_end_matches(['u', 'U'])
        .parse::<u32>()
        .ok()?;
    let hi_excl = if parts[1].contains("<=") { hi + 1 } else { hi };
    if hi_excl < lo || (hi_excl - lo) > 64 {
        return None;
    }
    Some((ivar, lo, hi_excl))
}

fn skip_braced_lines(lines: &[&str], mut i: usize) -> ((), usize) {
    let mut depth = 1usize;
    while i < lines.len() {
        let l = lines[i];
        if l.ends_with('{') {
            depth += 1;
        }
        if l.starts_with('}') {
            depth -= 1;
            if depth == 0 {
                return ((), i);
            }
        }
        i += 1;
    }
    ((), i)
}

fn split_c_assignment(stmt: &str) -> Option<(&str, &'static str, &str)> {
    for op in ["+=", "-=", "^=", "|=", "&="] {
        if let Some((l, r)) = stmt.split_once(op) {
            return Some((l.trim(), op, r.trim()));
        }
    }
    let bytes = stmt.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'=' {
            let prev = (i > 0).then(|| bytes[i - 1]);
            let next = bytes.get(i + 1).copied();
            if matches!(prev, Some(b'=' | b'!' | b'<' | b'>')) || next == Some(b'=') {
                continue;
            }
            return Some((stmt[..i].trim(), "=", stmt[i + 1..].trim()));
        }
    }
    None
}

fn subst_ssa_vars(expr: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(expr.len() + 16);
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &expr[start..i];
            if let Some(sub) = vars.get(ident) {
                out.push_str(sub);
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

fn mux_join_ssa(cond: &str, base: &SsaState, t: SsaState, e: SsaState) -> SsaState {
    let mut keys: BTreeSet<String> = base.vars.keys().cloned().collect();
    keys.extend(t.vars.keys().cloned());
    keys.extend(e.vars.keys().cloned());

    let mut vars = BTreeMap::new();
    for k in keys {
        let tv = t
            .vars
            .get(&k)
            .or_else(|| base.vars.get(&k))
            .cloned()
            .unwrap_or_else(|| "0".into());
        let ev = e
            .vars
            .get(&k)
            .or_else(|| base.vars.get(&k))
            .cloned()
            .unwrap_or_else(|| "0".into());
        if tv == ev {
            vars.insert(k, tv);
        } else {
            vars.insert(k, format!("(({cond}) ? {tv} : {ev})"));
        }
    }

    let (ret_expr, ret_taken) = match (&t.ret_expr, &e.ret_expr) {
        (Some(tr), Some(er)) => {
            let tt = t.ret_taken.as_deref().unwrap_or("1'b1");
            let et = e.ret_taken.as_deref().unwrap_or("1'b1");
            (
                Some(format!("(({cond}) ? {tr} : {er})")),
                Some(format!("(({cond}) ? {tt} : {et})")),
            )
        }
        (Some(tr), None) => {
            let tt = t.ret_taken.as_deref().unwrap_or("1'b1");
            (Some(tr.clone()), Some(format!("(({cond}) ? {tt} : 1'b0)")))
        }
        (None, Some(er)) => {
            let et = e.ret_taken.as_deref().unwrap_or("1'b1");
            (Some(er.clone()), Some(format!("(({cond}) ? 1'b0 : {et})")))
        }
        (None, None) => (base.ret_expr.clone(), base.ret_taken.clone()),
    };

    SsaState {
        vars,
        ret_expr,
        ret_taken,
    }
}
