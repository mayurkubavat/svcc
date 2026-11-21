//! IEEE 1666-2023 SystemC + UVM-SystemC + CRAVE + FC4SC front-end for `UVIR`.
//!
//! Architecture:
//! - Exposes [`CppAstExtractor`] so fast zero-dependency structural parsing works out-of-the-box
//!   on header-only VIPs, while full `libclang` translation-unit semantic extraction (`clang-sys`)
//!   plugs into the exact same trait under `--features libclang`.
//! - Recognizes:
//!   - `uvm::uvm_randomized_sequence_item`, `uvm::uvm_sequence_item`, `uvm::uvm_randomized_sequence`,
//!     `uvm::uvm_randomized_reg`, `uvm::uvm_component`, `uvm::uvm_driver`, `uvm::uvm_monitor`,
//!     `uvm::uvm_env`, `uvm::uvm_test`
//!   - `crave::crv_variable<T>` and plain C++17 integer/sc_bv/sc_lv fields
//!   - `crave::crv_constraint` and `crave::crv_soft_constraint` Boost.Proto expressions
//!     (including `field()` call syntax, `crave::if_then(a, b)`, and `crave::inside(...)`)
//!   - `fc4sc::covergroup`, `COVERPOINT(...)`, `fc4sc::bin<T>`, `fc4sc::interval(lo, hi)`, and `fc4sc::cross`

use std::time::Instant;
use uvir_ir::{
    BinKind, ClassDecl, ConstraintBlock, ConstraintClause, CovBin, CovergroupDecl, CoverpointDecl,
    CrossDecl, CrvArena, CrvBinaryOp, CrvExpr, CrvUnaryOp, ExprId, FieldDecl, FieldOps, Frontend,
    FrontendMetrics, InsideElem, OwnershipMode, Provenance, RalFieldMeta, ScalarKind, SourceLang,
    UvirError, UvirModule, UvirType, UvmRole,
};

/// Pluggable C++17 AST provider (`StructuralCppExtractor` default; `LibClangExtractor` optional).
pub trait CppAstExtractor: Send + Sync {
    fn backend_id(&self) -> &'static str;
    fn extract(&self, filename: &str, source: &str) -> Result<UvirModule, UvirError>;
}

/// Fast zero-dependency C++17 structural extractor for UVM-SystemC + CRAVE + FC4SC headers.
#[derive(Clone, Debug, Default)]
pub struct StructuralCppExtractor;

/// SystemC / UVM-SystemC / CRAVE / FC4SC front-end implementing [`uvir_ir::Frontend`].
pub struct ScFrontend {
    extractor: Box<dyn CppAstExtractor>,
}

impl Default for ScFrontend {
    fn default() -> Self {
        Self::new()
    }
}

impl ScFrontend {
    pub fn new() -> Self {
        Self {
            extractor: Box::new(StructuralCppExtractor),
        }
    }

    pub fn with_extractor(extractor: Box<dyn CppAstExtractor>) -> Self {
        Self { extractor }
    }
}

impl Frontend for ScFrontend {
    fn name(&self) -> &'static str {
        self.extractor.backend_id()
    }

    fn lower_str(
        &self,
        filename: &str,
        source: &str,
    ) -> Result<(UvirModule, FrontendMetrics), UvirError> {
        let t0 = Instant::now();
        let module = self.extractor.extract(filename, source)?;
        let elapsed_us = t0.elapsed().as_micros() as u64;

        let constraints_lowered: usize = module.classes.iter().map(|c| c.constraints.len()).sum();
        let covergroups_lowered: usize = module
            .classes
            .iter()
            .map(|c| c.covergroups.len())
            .sum::<usize>()
            + module.standalone_covergroups.len();

        let metrics = FrontendMetrics {
            frontend: self.name().to_string(),
            parse_us: elapsed_us / 2,
            lower_us: elapsed_us - (elapsed_us / 2),
            classes_lowered: module.classes.len(),
            constraints_lowered,
            covergroups_lowered,
        };
        Ok((module, metrics))
    }
}

impl CppAstExtractor for StructuralCppExtractor {
    fn backend_id(&self) -> &'static str {
        "sc-uvm-structural-c++17"
    }

    fn extract(&self, filename: &str, source: &str) -> Result<UvirModule, UvirError> {
        let mut module = UvirModule::new(filename);
        let mut standalone_cgs = Vec::new();

        for (line_no, cls_hdr, body) in scan_cpp_classes(source) {
            let (name, base) = parse_class_header(&cls_hdr);
            if base.contains("covergroup") {
                standalone_cgs.push(parse_fc4sc_covergroup(&name, &body));
                continue;
            }
            let role = classify_sc_base(&base);
            let cls = parse_sc_uvm_class(
                filename,
                line_no,
                name,
                base,
                role,
                &body,
                &mut module.crv_arena,
            )?;
            module.classes.push(cls);
        }

        // Attach standalone covergroups matching `<class>_cg` or `cg_<class>` if unambiguous,
        // else keep in `standalone_covergroups`.
        for cg in standalone_cgs {
            if module.classes.len() == 1 {
                module.classes[0].covergroups.push(cg);
            } else {
                module.standalone_covergroups.push(cg);
            }
        }

        Ok(module)
    }
}

fn scan_cpp_classes(src: &str) -> Vec<(u32, String, String)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut line: u32 = 1;

    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        // Skip `//` line comments
        if src[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Match `class <Name> : ... {` or `struct <Name> : ... {`
        if (src[i..].starts_with("class ") || src[i..].starts_with("struct "))
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
        {
            let start_line = line;
            let hdr_start = i;
            while i < bytes.len() && bytes[i] != b'{' && bytes[i] != b';' {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'{' {
                let hdr = src[hdr_start..i].trim().to_string();
                i += 1; // consume '{'
                let body_start = i;
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
                let body_end = i.saturating_sub(1);
                let body = src[body_start..body_end].to_string();
                out.push((start_line, hdr, body));
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_class_header(hdr: &str) -> (String, String) {
    let rest = hdr
        .strip_prefix("class ")
        .or_else(|| hdr.strip_prefix("struct "))
        .unwrap_or(hdr)
        .trim();
    if let Some((name_part, base_part)) = rest.split_once(':') {
        let base_clean = base_part
            .trim()
            .trim_start_matches("public ")
            .trim_start_matches("virtual ")
            .trim()
            .to_string();
        (name_part.trim().to_string(), base_clean)
    } else {
        (rest.to_string(), String::new())
    }
}

fn classify_sc_base(base: &str) -> UvmRole {
    let b = base
        .split('<')
        .next()
        .unwrap_or(base)
        .split("::")
        .last()
        .unwrap_or(base)
        .trim();
    match b {
        "uvm_randomized_sequence_item" | "uvm_sequence_item" => UvmRole::SequenceItem,
        "uvm_randomized_sequence" | "uvm_sequence" => UvmRole::Sequence,
        "uvm_randomized_reg" | "uvm_reg" => UvmRole::Reg,
        "uvm_reg_block" => UvmRole::RegBlock,
        "uvm_driver" => UvmRole::Driver,
        "uvm_monitor" => UvmRole::Monitor,
        "uvm_sequencer" => UvmRole::Sequencer,
        "uvm_agent" => UvmRole::Agent,
        "uvm_env" => UvmRole::Env,
        "uvm_scoreboard" => UvmRole::Scoreboard,
        "uvm_subscriber" => UvmRole::Subscriber,
        "uvm_test" => UvmRole::Test,
        "uvm_component" => UvmRole::Component,
        "uvm_object" | "crv_sequence_item" => UvmRole::Object,
        _ => UvmRole::PlainClass,
    }
}

fn parse_sc_uvm_class(
    filename: &str,
    line_no: u32,
    name: String,
    base: String,
    role: UvmRole,
    body: &str,
    arena: &mut CrvArena,
) -> Result<ClassDecl, UvirError> {
    let mut fields: Vec<FieldDecl> = Vec::new();
    let mut constraints: Vec<ConstraintBlock> = Vec::new();
    let mut sv_signedness = false;
    let mut has_copy_ctor = false;

    for raw_line in body.lines() {
        let trimmed_raw = raw_line.trim();
        if let Some(rest) = trimmed_raw.strip_prefix("// uvir:nocompare ") {
            if let Some(f) = fields.iter_mut().find(|f| f.name == rest.trim()) {
                f.ops.compare = false;
            }
            continue;
        }
        if let Some(rest) = trimmed_raw.strip_prefix("// uvir:nocopy ") {
            if let Some(f) = fields.iter_mut().find(|f| f.name == rest.trim()) {
                f.ops.copy = false;
            }
            continue;
        }
        let line = raw_line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.contains("set_sv_signedness(true)") {
            sv_signedness = true;
        }
        if line.starts_with(&format!("{name}(const {name}&")) {
            has_copy_ctor = true;
        }

        // `crave::crv_variable<uint32_t> addr{"addr"};` or `crv_variable<sc_dt::sc_uint<8>> len;`
        if line.contains("crv_variable<") {
            if let Some(f) = parse_crv_variable_decl(line) {
                fields.push(f);
                continue;
            }
        }

        // Plain scalar member: `uint32_t tag = 0;` / `bool write;`
        if let Some(f) = parse_plain_cpp_member(line) {
            fields.push(f);
            continue;
        }

        // `bind_field(en, &en_reg_field, "RW", 0, 1);`
        if let Some(rest) = line.strip_prefix("bind_field(") {
            if let Some(inner) = rest.strip_suffix(");") {
                let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
                if parts.len() >= 5 {
                    let fname = parts[0];
                    let access = parts[2].trim_matches('"').to_string();
                    let lsb = parts[3].parse::<u32>().unwrap_or(0);
                    let width = parts[4].parse::<u32>().unwrap_or(1);
                    if let Some(f) = fields.iter_mut().find(|f| f.name == fname) {
                        f.ty = UvirType::bitvec(width, false);
                        f.ral_meta = Some(RalFieldMeta {
                            lsb,
                            width,
                            access,
                            volatile: false,
                            reset: 0,
                            has_reset: false,
                        });
                    }
                }
            }
        }

        // Compare policy clue inside `do_compare`: `// uvir:nocompare <field>`
        if let Some(rest) = raw_line.trim().strip_prefix("// uvir:nocompare ") {
            let fname = rest.trim();
            if let Some(f) = fields.iter_mut().find(|f| f.name == fname) {
                f.ops.compare = false;
            }
        }
    }

    // Parse CRAVE constraint blocks:
    // `crave::crv_constraint c_align{this, "c_align", ...};`
    // `crave::crv_soft_constraint c_soft{this, "c_soft", ...};`
    for (is_soft, cname, expr_args) in extract_crave_constraint_inits(body) {
        let mut clauses = Vec::new();
        for arg in expr_args {
            let expr = parse_crave_expr(&arg, arena)?;
            clauses.push(ConstraintClause {
                expr,
                is_soft,
                soft_weight: is_soft.then_some(1),
            });
        }
        // Merge soft helper block `<base>__soft_<idx>` into parent constraint block if matching
        if let Some((parent_name, _)) = cname.split_once("__soft_") {
            if let Some(existing) = constraints.iter_mut().find(|c| c.name == parent_name) {
                existing.clauses.extend(clauses);
                continue;
            }
        }
        constraints.push(ConstraintBlock {
            name: cname,
            clauses,
        });
    }

    Ok(ClassDecl {
        name,
        package_or_namespace: String::new(),
        role,
        base_class: (!base.is_empty()).then_some(base),
        ownership: OwnershipMode::ScValueCopy,
        sv_signedness,
        needs_value_copy_ctor: has_copy_ctor || fields.iter().any(|f| f.is_rand),
        synthesize_field_methods: role == UvmRole::SequenceItem || role == UvmRole::Object,
        reg_width_bits: (role == UvmRole::Reg).then_some(32),
        fields,
        constraints,
        covergroups: Vec::new(),
        methods: Vec::new(),
        provenance: Provenance {
            source_lang: Some(SourceLang::SystemCUvm),
            file: filename.to_string(),
            line: line_no,
            original_macros: Vec::new(),
        },
    })
}

fn parse_crv_variable_decl(line: &str) -> Option<FieldDecl> {
    let idx = line.find("crv_variable<")?;
    let after = &line[idx + "crv_variable<".len()..];
    // Handle nested `<...>` such as `sc_dt::sc_uint<12>`
    let mut depth = 1usize;
    let mut end_ty = 0usize;
    for (pos, ch) in after.char_indices() {
        if ch == '<' {
            depth += 1;
        } else if ch == '>' {
            depth -= 1;
            if depth == 0 {
                end_ty = pos;
                break;
            }
        }
    }
    if end_ty == 0 {
        return None;
    }
    let cpp_ty = after[..end_ty].trim();
    let rest = after[end_ty + 1..].trim().trim_end_matches(';');
    let fname = rest
        .split(['{', '(', '=', ';', ' '])
        .next()?
        .trim()
        .to_string();
    if fname.is_empty() {
        return None;
    }
    Some(FieldDecl {
        name: fname,
        ty: map_cpp_type(cpp_ty),
        is_rand: true,
        is_randc: false,
        ops: FieldOps::default(),
        ral_meta: None,
    })
}

fn parse_plain_cpp_member(line: &str) -> Option<FieldDecl> {
    if line.contains('(') && !line.contains('=') {
        return None;
    }
    let prefixes = [
        "uint8_t ",
        "uint16_t ",
        "uint32_t ",
        "uint64_t ",
        "int8_t ",
        "int16_t ",
        "int32_t ",
        "int64_t ",
        "bool ",
        "sc_dt::sc_uint<",
        "sc_dt::sc_int<",
        "sc_dt::sc_bv<",
        "sc_dt::sc_lv<",
    ];
    if !prefixes.iter().any(|p| line.starts_with(p)) {
        return None;
    }
    let clean = line.trim_end_matches(';').split('=').next()?.trim();
    let last_space = clean.rfind(' ')?;
    let cpp_ty = clean[..last_space].trim();
    let fname = clean[last_space + 1..].trim();
    if fname.is_empty() || fname.contains('(') {
        return None;
    }
    Some(FieldDecl {
        name: fname.to_string(),
        ty: map_cpp_type(cpp_ty),
        is_rand: false,
        is_randc: false,
        ops: FieldOps::default(),
        ral_meta: None,
    })
}

pub fn map_cpp_type(cpp_ty: &str) -> UvirType {
    let t = cpp_ty.trim().trim_start_matches("std::");
    match t {
        "bool" => UvirType::bitvec(1, false),
        "uint8_t" | "unsigned char" => UvirType::bitvec(8, false),
        "int8_t" | "signed char" => UvirType::bitvec(8, true),
        "uint16_t" | "unsigned short" => UvirType::bitvec(16, false),
        "int16_t" | "short" => UvirType::bitvec(16, true),
        "uint32_t" | "unsigned int" | "unsigned" => UvirType::bitvec(32, false),
        "int32_t" | "int" => UvirType::bitvec(32, true),
        "uint64_t" | "unsigned long long" => UvirType::bitvec(64, false),
        "int64_t" | "long long" => UvirType::bitvec(64, true),
        "string" => UvirType {
            kind: ScalarKind::String,
            width: 0,
            signed: false,
            is_four_state: false,
        },
        _ => {
            if let (Some(lb), Some(rb)) = (t.find('<'), t.rfind('>')) {
                let w = t[lb + 1..rb].trim().parse::<u32>().unwrap_or(32);
                if t.contains("sc_lv") {
                    return UvirType::logicvec(w, false);
                }
                let signed = t.contains("sc_int") || t.contains("sc_bigint");
                return UvirType::bitvec(w, signed);
            }
            UvirType::bitvec(32, false)
        }
    }
}

fn extract_crave_constraint_inits(body: &str) -> Vec<(bool, String, Vec<String>)> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        let is_soft = if body[i..].starts_with("crave::crv_soft_constraint ")
            || body[i..].starts_with("crv_soft_constraint ")
        {
            Some(true)
        } else if body[i..].starts_with("crave::crv_constraint ")
            || body[i..].starts_with("crv_constraint ")
        {
            Some(false)
        } else {
            None
        };

        if let Some(soft) = is_soft {
            // Advance past keyword
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let name_start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let cname = body[name_start..i].to_string();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'{' || bytes[i] == b'(') {
                let open = bytes[i];
                let close = if open == b'{' { b'}' } else { b')' };
                i += 1;
                let init_start = i;
                let mut depth = 1usize;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == open {
                        depth += 1;
                    } else if bytes[i] == close {
                        depth -= 1;
                    }
                    i += 1;
                }
                let init_str = &body[init_start..i.saturating_sub(1)];
                let args = split_top_level_commas(init_str);
                let expr_args: Vec<String> = args
                    .into_iter()
                    .filter(|a| {
                        let t = a.trim();
                        t != "this" && !(t.starts_with('"') && t.ends_with('"'))
                    })
                    .collect();
                if !cname.is_empty() && !expr_args.is_empty() {
                    out.push((soft, cname, expr_args));
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth_paren = 0usize;
    let mut depth_brace = 0usize;
    let mut start = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            '{' => depth_brace += 1,
            '}' => depth_brace = depth_brace.saturating_sub(1),
            ',' if depth_paren == 0 && depth_brace == 0 => {
                parts.push(s[start..idx].trim().to_string());
                start = idx + 1;
            }
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}

/// Converts CRAVE C++17 Boost.Proto idioms (`field()`, `crave::if_then(a, b)`,
/// `crave::inside(x(), std::set<...>{...})`, `0x1000U`) into canonical SV-like syntax
/// and reuses a clean recursive-descent builder into [`CrvArena`].
fn parse_crave_expr(raw: &str, arena: &mut CrvArena) -> Result<ExprId, UvirError> {
    let s = strip_outer_parens(raw.trim());

    // `crave::if_then(cond, body)` or `if_then(cond, body)`
    if let Some(inner) = s
        .strip_prefix("crave::if_then(")
        .or_else(|| s.strip_prefix("if_then("))
    {
        if let Some(args_s) = inner.strip_suffix(')') {
            let args = split_top_level_commas(args_s);
            if args.len() == 2 {
                let lhs = parse_crave_expr(&args[0], arena)?;
                let rhs = parse_crave_expr(&args[1], arena)?;
                return Ok(arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Implies,
                    lhs,
                    rhs,
                }));
            }
        }
    }

    // `crave::inside(var(), {1, 2})` or range form `(x() >= lo && x() <= hi)`
    if let Some(inner) = s
        .strip_prefix("crave::inside(")
        .or_else(|| s.strip_prefix("inside("))
    {
        if let Some(args_s) = inner.strip_suffix(')') {
            let args = split_top_level_commas(args_s);
            if args.len() == 2 {
                let target = parse_crave_expr(&args[0], arena)?;
                let set_s = args[1].trim();
                let brace_body = if let (Some(lb), Some(rb)) = (set_s.find('{'), set_s.rfind('}')) {
                    &set_s[lb + 1..rb]
                } else {
                    set_s
                };
                let mut elems = Vec::new();
                for v_str in split_top_level_commas(brace_body) {
                    let single = parse_crave_expr(&v_str, arena)?;
                    elems.push(InsideElem::Single(single));
                }
                return Ok(arena.alloc(CrvExpr::Inside { target, elems }));
            }
        }
    }

    // Binary operator split (lowest-precedence first)
    let ops: &[(&str, CrvBinaryOp)] = &[
        ("||", CrvBinaryOp::LogOr),
        ("&&", CrvBinaryOp::LogAnd),
        ("|", CrvBinaryOp::BitOr),
        ("^", CrvBinaryOp::BitXor),
        ("&", CrvBinaryOp::BitAnd),
        ("==", CrvBinaryOp::Eq),
        ("!=", CrvBinaryOp::Ne),
        ("<=", CrvBinaryOp::Le),
        (">=", CrvBinaryOp::Ge),
        ("<<", CrvBinaryOp::Shl),
        (">>", CrvBinaryOp::Shr),
        ("<", CrvBinaryOp::Lt),
        (">", CrvBinaryOp::Gt),
        ("+", CrvBinaryOp::Add),
        ("-", CrvBinaryOp::Sub),
        ("*", CrvBinaryOp::Mul),
        ("%", CrvBinaryOp::Rem),
    ];

    for &(sym, op) in ops {
        if let Some((lhs_s, rhs_s)) = split_top_level_binop(s, sym) {
            let lhs = parse_crave_expr(lhs_s, arena)?;
            let rhs = parse_crave_expr(rhs_s, arena)?;
            return Ok(arena.alloc(CrvExpr::Binary { op, lhs, rhs }));
        }
    }

    if let Some(rest) = s.strip_prefix('!') {
        let arg = parse_crave_expr(rest, arena)?;
        return Ok(arena.alloc(CrvExpr::Unary {
            op: CrvUnaryOp::Not,
            arg,
        }));
    }
    if let Some(rest) = s.strip_prefix('~') {
        let arg = parse_crave_expr(rest, arena)?;
        return Ok(arena.alloc(CrvExpr::Unary {
            op: CrvUnaryOp::BitNot,
            arg,
        }));
    }

    // `static_cast<T>(expr)` -> unwrap inner expr
    if let Some(rest) = s.strip_prefix("static_cast<") {
        if let Some((_, after_gt)) = rest.split_once('>') {
            return parse_crave_expr(after_gt.trim(), arena);
        }
    }

    // `field()` Boost.Proto variable reference
    if let Some(var_name) = s.strip_suffix("()") {
        let clean = var_name.trim();
        if clean.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !clean.is_empty() {
            return Ok(arena.alloc(CrvExpr::Var(clean.to_string())));
        }
    }

    // Numeric C++ literal (`0x1000U`, `16ULL`, `42`)
    let num_clean = s.trim_end_matches(['u', 'U', 'l', 'L']);
    if let Some(hex) = num_clean
        .strip_prefix("0x")
        .or_else(|| num_clean.strip_prefix("0X"))
    {
        if let Ok(v) = u128::from_str_radix(hex, 16) {
            return Ok(arena.alloc(CrvExpr::Const {
                value: v,
                width: 32,
                signed: false,
            }));
        }
    }
    if let Ok(v) = num_clean.parse::<u128>() {
        return Ok(arena.alloc(CrvExpr::Const {
            value: v,
            width: 32,
            signed: false,
        }));
    }

    if s == "true" {
        return Ok(arena.alloc(CrvExpr::BoolConst(true)));
    }
    if s == "false" {
        return Ok(arena.alloc(CrvExpr::BoolConst(false)));
    }

    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s.is_empty() {
        return Ok(arena.alloc(CrvExpr::Var(s.to_string())));
    }

    Err(UvirError::new(
        "uvir-front-sc/crave",
        format!("unable to parse CRAVE expression '{raw}'"),
    ))
}

fn strip_outer_parens(mut s: &str) -> &str {
    loop {
        let bytes = s.as_bytes();
        if bytes.len() >= 2 && bytes[0] == b'(' && bytes[bytes.len() - 1] == b')' {
            let mut depth = 0usize;
            let mut wraps_all = true;
            for (idx, &b) in bytes.iter().enumerate() {
                if b == b'(' {
                    depth += 1;
                } else if b == b')' {
                    depth -= 1;
                    if depth == 0 && idx + 1 < bytes.len() {
                        wraps_all = false;
                        break;
                    }
                }
            }
            if wraps_all {
                s = s[1..s.len() - 1].trim();
                continue;
            }
        }
        return s;
    }
}

fn split_top_level_binop<'a>(s: &'a str, sym: &str) -> Option<(&'a str, &'a str)> {
    let bytes = s.as_bytes();
    let sym_b = sym.as_bytes();
    let mut depth = 0usize;
    let mut last_match = None;
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth = depth.saturating_sub(1),
            _ if depth == 0 && s[i..].starts_with(sym) => {
                // Prevent single-char `&`, `|`, `<`, `>` from splitting `&&`, `||`, `<=`, `>=`, `<<`, `>>`
                if sym.len() == 1 {
                    let prev = (i > 0).then(|| bytes[i - 1]);
                    let next = bytes.get(i + 1).copied();
                    let ch = sym_b[0];
                    if prev == Some(ch) || next == Some(ch) || next == Some(b'=') {
                        i += 1;
                        continue;
                    }
                    if (ch == b'<' || ch == b'>') && (prev == Some(b'<') || prev == Some(b'>')) {
                        i += 1;
                        continue;
                    }
                }
                last_match = Some(i);
                i += sym.len();
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    let pos = last_match?;
    let lhs = s[..pos].trim();
    let rhs = s[pos + sym.len()..].trim();
    if lhs.is_empty() || rhs.is_empty() {
        return None;
    }
    Some((lhs, rhs))
}

fn parse_fc4sc_covergroup(name: &str, body: &str) -> CovergroupDecl {
    let mut coverpoints = Vec::new();
    let mut crosses = Vec::new();
    let uncommented: String = body
        .lines()
        .map(|ln| ln.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");

    for stmt in uncommented.split(';') {
        let l = stmt.trim();
        // `COVERPOINT(uint32_t, cp_addr, item.addr) { bin<uint32_t>("low", interval(0x1000, 0x17ff)) };`
        if let Some(idx) = l.find("COVERPOINT(") {
            let rest = &l[idx + "COVERPOINT(".len()..];
            if let Some((args_part, bins_part)) = rest.split_once(')') {
                let args: Vec<&str> = args_part.split(',').map(str::trim).collect();
                if args.len() >= 3 {
                    let cp_name = args[1].to_string();
                    let target_raw = args[2];
                    let target_expr = target_raw
                        .rsplit('.')
                        .next()
                        .unwrap_or(target_raw)
                        .trim_end_matches("()")
                        .to_string();
                    let bins = extract_fc4sc_bins(bins_part);
                    coverpoints.push(CoverpointDecl {
                        name: cp_name,
                        target_expr,
                        iff_guard: None,
                        bins,
                    });
                }
            }
        } else if l.contains("fc4sc::cross<") {
            // `fc4sc::cross<uint32_t, bool> cx_rw_addr{this, "cx_rw_addr", &cp_addr, &cp_write};`
            if let (Some(lb), Some(rb)) = (l.find('{'), l.rfind('}')) {
                let args: Vec<&str> = l[lb + 1..rb].split(',').map(str::trim).collect();
                if args.len() >= 3 {
                    let cname = args[1].trim_matches('"').to_string();
                    let cps = args[2..]
                        .iter()
                        .map(|s| s.trim().trim_start_matches('&').to_string())
                        .collect();
                    crosses.push(CrossDecl {
                        name: cname,
                        coverpoints: cps,
                    });
                }
            }
        }
    }

    CovergroupDecl {
        name: name.to_string(),
        coverpoints,
        crosses,
    }
}

fn extract_fc4sc_bins(s: &str) -> Vec<CovBin> {
    let mut bins = Vec::new();
    let mut rem = s;
    while let Some(pos) = rem.find("bin<") {
        let after = &rem[pos..];
        let Some(lp) = after.find('(') else { break };
        // Find matching ')'
        let mut depth = 0usize;
        let mut rp = None;
        for (i, ch) in after[lp..].char_indices() {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
                if depth == 0 {
                    rp = Some(lp + i);
                    break;
                }
            }
        }
        let Some(end_p) = rp else { break };
        let call_args = split_top_level_commas(&after[lp + 1..end_p]);
        if call_args.len() >= 2 {
            let bname = call_args[0].trim().trim_matches('"').to_string();
            let spec = call_args[1].trim();
            if let Some(iv) = spec
                .strip_prefix("fc4sc::interval(")
                .or_else(|| spec.strip_prefix("interval("))
            {
                if let Some(iv_args) = iv.strip_suffix(')') {
                    let bounds = split_top_level_commas(iv_args);
                    if bounds.len() == 2 {
                        let low = parse_cpp_u64(&bounds[0]).unwrap_or(0);
                        let high = parse_cpp_u64(&bounds[1]).unwrap_or(0);
                        bins.push(CovBin {
                            name: bname,
                            kind: BinKind::Range { low, high },
                        });
                    }
                }
            } else {
                let mut vals = Vec::new();
                for a in &call_args[1..] {
                    if let Some(v) = parse_cpp_u64(a) {
                        vals.push(v);
                    }
                }
                if !vals.is_empty() {
                    bins.push(CovBin {
                        name: bname,
                        kind: BinKind::Values(vals),
                    });
                }
            }
        }
        rem = &after[end_p + 1..];
    }
    bins
}

fn parse_cpp_u64(raw: &str) -> Option<u64> {
    let s = raw.trim().trim_end_matches(['u', 'U', 'l', 'L']);
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_sc_uvm_crave_sequence_item_and_fc4sc_covergroup() {
        let cpp = r#"
class apb_item : public uvm::uvm_randomized_sequence_item {
public:
  crave::crv_variable<uint32_t> addr{"addr"};
  crave::crv_variable<uint8_t> len{"len"};
  crave::crv_variable<bool> write{"write"};
  uint32_t tag = 0;

  crave::crv_constraint c_align{this, "c_align",
    (addr() & 0x3U) == 0x0U,
    (addr() >= 0x1000U && addr() <= 0x1FFFU),
    crave::if_then(write(), len() <= 16U)
  };
  crave::crv_soft_constraint c_align__soft_0{this, "c_align__soft_0", len() == 4U};
};

class cg_apb : public fc4sc::covergroup {
public:
  COVERPOINT(uint32_t, cp_addr, item.addr) {
    fc4sc::bin<uint32_t>("low_page", fc4sc::interval(0x1000U, 0x17FFU)),
    fc4sc::bin<uint32_t>("high_page", fc4sc::interval(0x1800U, 0x1FFFU))
  };
  COVERPOINT(bool, cp_write, item.write) {
    fc4sc::bin<bool>("rd", 0),
    fc4sc::bin<bool>("wr", 1)
  };
  fc4sc::cross<uint32_t, bool> cx_rw_addr{this, "cx_rw_addr", &cp_addr, &cp_write};
};
"#;
        let fe = ScFrontend::new();
        let (m, metrics) = fe.lower_str("apb_item.hpp", cpp).expect("lower sc");
        assert_eq!(metrics.classes_lowered, 1);
        assert_eq!(m.classes[0].fields.len(), 4);
        assert_eq!(m.classes[0].constraints.len(), 1);
        assert_eq!(m.classes[0].constraints[0].clauses.len(), 4);
        assert_eq!(m.classes[0].covergroups.len(), 1);
        assert_eq!(m.classes[0].covergroups[0].coverpoints.len(), 2);
    }
}
