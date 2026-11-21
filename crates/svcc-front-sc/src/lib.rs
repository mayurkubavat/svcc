//! IEEE 1666-2023 SystemC + UVM-SystemC + CRAVE + FC4SC front-end for `UVIR`.
//!
//! Lifts C++17 verification classes, `enum class` types, `uvir_clocking_*` `#1step` samplers,
//! `uvir_sva_*` bitmask pipeline observers, `crave::dist` weighted distributions, `uvm_reg_block`
//! maps, component/TLM topologies, and `sc_spawn` cancel-token concurrency back into [`UvirModule`].

pub mod rtl_lift;

use std::collections::HashMap;
use std::time::Instant;
use svcc_ir::{
    BinKind, ClassDecl, ClockingDecl, ConstraintBlock, ConstraintClause, CovBin, CovergroupDecl,
    CoverpointDecl, CrossDecl, CrvArena, CrvBinaryOp, CrvExpr, CrvUnaryOp, DistItem,
    DistWeightKind, EnumDecl, ExprId, FieldDecl, FieldOps, Frontend, FrontendMetrics, InsideElem,
    JoinKind, MethodDecl, OwnershipMode, PhaseKind, Provenance, RalFieldMeta, RalMapSpec,
    RalRegInstance, ScalarKind, SchedOp, SourceLang, SubComponentDecl, SvaPropertyDecl,
    TlmConnection, TlmPortDecl, TlmPortKind, UvirError, UvirModule, UvirType, UvmRole,
};

pub trait CppAstExtractor: Send + Sync {
    fn backend_id(&self) -> &'static str;
    fn extract(&self, filename: &str, source: &str) -> Result<UvirModule, UvirError>;
}

#[derive(Clone, Debug, Default)]
pub struct StructuralCppExtractor;

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
            properties_lowered: module.properties.len(),
            rtl_modules_lowered: module.rtl_modules.len(),
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
        module.enums = extract_cpp_enums(source);
        module.clockings = extract_cpp_clockings(source);
        module.properties = extract_cpp_sva_observers(source);
        module.rtl_modules = rtl_lift::extract_sc_rtl_modules(filename, source)?;
        module
            .rtl_modules
            .extend(rtl_lift::extract_c2rtl_functions(filename, source)?);

        let enum_widths: HashMap<String, u32> = module
            .enums
            .iter()
            .map(|e| (e.name.clone(), e.width))
            .collect();

        let mut standalone_cgs = Vec::new();
        let mut cg_owners: Vec<(Option<String>, CovergroupDecl)> = Vec::new();

        for (line_no, cls_hdr, body) in scan_cpp_classes(source) {
            let (name, base) = parse_class_header(&cls_hdr);
            if name.starts_with("uvir_clocking_") || name.starts_with("uvir_sva_") {
                continue;
            }
            if base.contains("covergroup") {
                let (owner_ty, cg) = parse_fc4sc_covergroup(&name, &body);
                cg_owners.push((owner_ty, cg));
                continue;
            }
            let (role, item_type) = classify_sc_base(&base);
            let cls = parse_sc_uvm_class(
                filename,
                line_no,
                name,
                base,
                role,
                item_type,
                &body,
                &enum_widths,
                &mut module.crv_arena,
            )?;
            module.classes.push(cls);
        }

        for (owner_opt, cg) in cg_owners {
            if let Some(owner_name) = owner_opt {
                if let Some(cls) = module.classes.iter_mut().find(|c| c.name == owner_name) {
                    cls.covergroups.push(cg);
                    continue;
                }
            }
            if module.classes.len() == 1 {
                module.classes[0].covergroups.push(cg);
            } else {
                standalone_cgs.push(cg);
            }
        }
        module.standalone_covergroups = standalone_cgs;

        Ok(module)
    }
}

fn extract_cpp_enums(src: &str) -> Vec<EnumDecl> {
    let mut out = Vec::new();
    let mut pending_width: Option<u32> = None;
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        if let Some(rest) = l.strip_prefix("// uvir:enum ") {
            if let Some((_, ws)) = rest.split_once("width=") {
                pending_width = ws.trim().parse::<u32>().ok();
            }
        } else if let Some(rest) = l.strip_prefix("enum class ") {
            let mut block = rest.to_string();
            while !block.contains('}') && i + 1 < lines.len() {
                i += 1;
                block.push(' ');
                block.push_str(lines[i].trim());
            }
            if let (Some(lb), Some(rb)) = (block.find('{'), block.rfind('}')) {
                let hdr = block[..lb].trim();
                let (ename, under) = hdr.split_once(':').unwrap_or((hdr, "uint32_t"));
                let width = pending_width.take().unwrap_or_else(|| match under.trim() {
                    "uint8_t" => 8,
                    "uint16_t" => 16,
                    "uint64_t" => 64,
                    _ => 32,
                });
                let mut variants = Vec::new();
                let mut next_v = 0u64;
                for item in block[lb + 1..rb].split(',') {
                    let it = item.trim();
                    if it.is_empty() {
                        continue;
                    }
                    if let Some((vn, vv)) = it.split_once('=') {
                        let val = parse_cpp_u64(vv).unwrap_or(next_v);
                        variants.push((vn.trim().to_string(), val));
                        next_v = val + 1;
                    } else {
                        variants.push((it.to_string(), next_v));
                        next_v += 1;
                    }
                }
                out.push(EnumDecl {
                    name: ename.trim().to_string(),
                    width,
                    variants,
                });
            }
        }
        i += 1;
    }
    out
}

fn extract_cpp_clockings(src: &str) -> Vec<ClockingDecl> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim().strip_prefix("// uvir:clocking ") else {
            continue;
        };
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let name = parts[0].to_string();
        let mut clk = "clk".to_string();
        let mut ins = Vec::new();
        let mut outs = Vec::new();
        for p in &parts[1..] {
            if let Some(c) = p.strip_prefix("clk=") {
                clk = c.to_string();
            } else if let Some(iv) = p.strip_prefix("in=") {
                ins = iv
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            } else if let Some(ov) = p.strip_prefix("out=") {
                outs = ov
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
        }
        out.push(ClockingDecl {
            name,
            clock_signal: clk,
            input_1step_signals: ins,
            output_signals: outs,
        });
    }
    out
}

fn extract_cpp_sva_observers(src: &str) -> Vec<SvaPropertyDecl> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim().strip_prefix("// uvir:sva ") else {
            continue;
        };
        let mut kv: HashMap<&str, &str> = HashMap::new();
        let mut tokens = rest.split_whitespace();
        let Some(name) = tokens.next() else {
            continue;
        };
        for tok in tokens {
            if let Some((k, v)) = tok.split_once('=') {
                kv.insert(k, v);
            }
        }
        let dis = kv
            .get("dis")
            .copied()
            .filter(|&d| d != "-")
            .map(str::to_string);
        out.push(SvaPropertyDecl {
            name: name.to_string(),
            clock_signal: kv.get("clk").copied().unwrap_or("clk").to_string(),
            disable_iff: dis,
            antecedent: kv.get("ant").copied().unwrap_or("true").to_string(),
            min_delay: kv.get("min").and_then(|s| s.parse().ok()).unwrap_or(0),
            max_delay: kv.get("max").and_then(|s| s.parse().ok()).unwrap_or(0),
            consequent: kv.get("con").copied().unwrap_or("true").to_string(),
            is_cover: kv.get("kind") == Some(&"cover"),
        });
    }
    out
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
        if src[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if (src[i..].starts_with("class ") || src[i..].starts_with("struct "))
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
            && !src[..i].trim_end().ends_with("enum")
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
                i += 1;
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

fn classify_sc_base(base: &str) -> (UvmRole, Option<String>) {
    let item_ty = if let (Some(lb), Some(rb)) = (base.find('<'), base.rfind('>')) {
        let inner = base[lb + 1..rb].trim();
        let clean = inner.strip_prefix("uvm::").unwrap_or(inner);
        (clean != "uvm_sequence_item").then(|| clean.to_string())
    } else {
        None
    };

    let b = base
        .split('<')
        .next()
        .unwrap_or(base)
        .split("::")
        .last()
        .unwrap_or(base)
        .trim();
    let role = match b {
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
    };
    (role, item_ty)
}

#[allow(clippy::too_many_arguments)]
fn parse_sc_uvm_class(
    filename: &str,
    line_no: u32,
    name: String,
    base: String,
    role: UvmRole,
    item_type: Option<String>,
    body: &str,
    enum_widths: &HashMap<String, u32>,
    arena: &mut CrvArena,
) -> Result<ClassDecl, UvirError> {
    let mut fields: Vec<FieldDecl> = Vec::new();
    let mut constraints: Vec<ConstraintBlock> = Vec::new();
    let mut subcomponents: Vec<SubComponentDecl> = Vec::new();
    let mut tlm_ports: Vec<TlmPortDecl> = Vec::new();
    let mut tlm_connections: Vec<TlmConnection> = Vec::new();
    let mut ral_map: Option<RalMapSpec> = None;
    let mut ral_ptr_types: HashMap<String, String> = HashMap::new();
    let mut next_is_randc: Option<String> = None;
    let mut next_enum_field: Option<(String, String)> = None;
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
        if let Some(rest) = trimmed_raw.strip_prefix("// uvir:randc ") {
            next_is_randc = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed_raw.strip_prefix("// uvir:enum_field ") {
            let mut sp = rest.split_whitespace();
            if let (Some(fnm), Some(enm)) = (sp.next(), sp.next()) {
                next_enum_field = Some((fnm.to_string(), enm.to_string()));
            }
            continue;
        }
        if let Some(rest) = trimmed_raw.strip_prefix("// uvir:subcomp ") {
            let mut sp = rest.split_whitespace();
            if let (Some(cty), Some(cnm)) = (sp.next(), sp.next()) {
                subcomponents.push(SubComponentDecl {
                    name: cnm.to_string(),
                    class_name: cty.to_string(),
                });
            }
            continue;
        }
        if let Some(rest) = trimmed_raw.strip_prefix("// uvir:connect ") {
            if let Some((lhs, rhs)) = rest.split_once("->") {
                tlm_connections.push(TlmConnection {
                    initiator: lhs.trim().to_string(),
                    target: rhs.trim().to_string(),
                });
            }
            continue;
        }
        if let Some(rest) = trimmed_raw.strip_prefix("// uvir:ral_map ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                ral_map = Some(RalMapSpec {
                    map_name: parts[0].to_string(),
                    base_addr: parse_cpp_u64(parts[1]).unwrap_or(0),
                    addr_unit_bytes: parts[2].parse().unwrap_or(4),
                    registers: Vec::new(),
                });
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

        // RegBlock register pointer declarations `<RegClass>* <inst> = nullptr;`
        if role == UvmRole::RegBlock && line.contains("* ") && line.ends_with("nullptr;") {
            if let Some((cty, rest)) = line.split_once('*') {
                let inst = rest.split('=').next().unwrap_or("").trim().to_string();
                ral_ptr_types.insert(inst, cty.trim().to_string());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("this->default_map->add_reg(") {
            if let Some(inner) = rest.strip_suffix(");") {
                let args: Vec<&str> = inner.split(',').map(str::trim).collect();
                if args.len() >= 3 {
                    let inst = args[0].to_string();
                    let offset = parse_cpp_u64(args[1]).unwrap_or(0);
                    let rights = args[2].trim_matches('"').to_string();
                    let cls_ty = ral_ptr_types.get(&inst).cloned().unwrap_or_default();
                    if let Some(rm) = ral_map.as_mut() {
                        rm.registers.push(RalRegInstance {
                            inst_name: inst,
                            class_name: cls_ty,
                            offset,
                            rights,
                        });
                    }
                }
            }
            continue;
        }

        // TLM ports
        if let Some(port) = parse_cpp_tlm_port(line) {
            tlm_ports.push(port);
            continue;
        }

        if line.contains("crv_variable<") {
            if let Some(mut f) = parse_crv_variable_decl(line, enum_widths) {
                if next_is_randc.as_deref() == Some(f.name.as_str()) {
                    f.is_randc = true;
                    next_is_randc = None;
                }
                if let Some((ef_name, ename)) = &next_enum_field {
                    if ef_name == &f.name {
                        let ew = enum_widths.get(ename).copied().unwrap_or(f.ty.width);
                        f.ty = UvirType::enum_ty(ename.clone(), ew);
                        next_enum_field = None;
                    }
                }
                fields.push(f);
                continue;
            }
        }

        if let Some(f) = parse_plain_cpp_member(line, enum_widths) {
            fields.push(f);
            continue;
        }

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
    }

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

    let methods = extract_cpp_sched_methods(body, role);

    Ok(ClassDecl {
        name,
        package_or_namespace: String::new(),
        role,
        base_class: (!base.is_empty()).then_some(base),
        item_type,
        ownership: OwnershipMode::ScValueCopy,
        sv_signedness,
        needs_value_copy_ctor: has_copy_ctor || fields.iter().any(|f| f.is_rand),
        synthesize_field_methods: role == UvmRole::SequenceItem || role == UvmRole::Object,
        reg_width_bits: (role == UvmRole::Reg).then_some(32),
        ral_map,
        subcomponents,
        tlm_ports,
        tlm_connections,
        fields,
        constraints,
        covergroups: Vec::new(),
        methods,
        provenance: Provenance {
            source_lang: Some(SourceLang::SystemCUvm),
            file: filename.to_string(),
            line: line_no,
            original_macros: Vec::new(),
        },
    })
}

fn parse_cpp_tlm_port(line: &str) -> Option<TlmPortDecl> {
    let kinds = [
        ("uvm::uvm_analysis_port<", TlmPortKind::AnalysisPort),
        ("uvm::uvm_analysis_export<", TlmPortKind::AnalysisExport),
        ("uvm::uvm_seq_item_pull_port<", TlmPortKind::SeqItemPullPort),
    ];
    for (pfx, kind) in kinds {
        if let Some(rest) = line.strip_prefix(pfx) {
            let (item_ty, after_gt) = rest.split_once('>')?;
            let pname = after_gt.split(['{', '(', ';']).next()?.trim().to_string();
            return Some(TlmPortDecl {
                name: pname,
                kind,
                item_type: item_ty.trim().to_string(),
            });
        }
    }
    None
}

fn extract_cpp_sched_methods(body: &str, role: UvmRole) -> Vec<MethodDecl> {
    let mut out = Vec::new();
    for target in ["run_phase", "body"] {
        let sig = format!("void {target}(");
        let Some(pos) = body.find(&sig) else {
            continue;
        };
        let after = &body[pos..];
        let Some(lb) = after.find('{') else {
            continue;
        };
        let mut depth = 1usize;
        let mut end_b = lb + 1;
        for (i, ch) in after[lb + 1..].char_indices() {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    end_b = lb + 1 + i;
                    break;
                }
            }
        }
        let mbody = &after[lb + 1..end_b];
        let ops = parse_cpp_sched_block(mbody);
        if !ops.is_empty() {
            out.push(MethodDecl {
                name: target.to_string(),
                phase: if target == "body" {
                    PhaseKind::SequenceBody
                } else {
                    PhaseKind::Run
                },
                is_task: role.is_component() || role == UvmRole::Sequence,
                is_virtual: true,
                body_ops: ops,
            });
        }
    }
    out
}

fn parse_cpp_sched_block(mbody: &str) -> Vec<SchedOp> {
    let mut ops = Vec::new();
    let lines: Vec<&str> = mbody.lines().map(str::trim).collect();
    let mut i = 0usize;
    while i < lines.len() {
        let l = lines[i];
        if let Some(rest) = l.strip_prefix("// uvir:fork ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            let join = match parts.first().copied().unwrap_or("join_all") {
                "join_any" => JoinKind::JoinAny,
                "join_none" => JoinKind::JoinNone,
                _ => JoinKind::JoinAll,
            };
            let cancel = parts.contains(&"cancel=true");
            let mut branches = Vec::new();
            i += 1;
            while i < lines.len() {
                if lines[i].starts_with("sc_core::sc_spawn(") {
                    let mut b_lines = Vec::new();
                    let mut depth = 1usize;
                    i += 1;
                    while i < lines.len() && depth > 0 {
                        let cur = lines[i];
                        if cur.contains('{') {
                            depth += cur.matches('{').count();
                        }
                        if cur.contains('}') {
                            depth = depth.saturating_sub(cur.matches('}').count());
                        }
                        if depth > 0 {
                            b_lines.push(cur);
                        }
                        i += 1;
                    }
                    branches.push(parse_cpp_sched_block(&b_lines.join("\n")));
                    continue;
                }
                if lines[i].contains("->store(true)")
                    || lines[i].starts_with("sc_core::wait(done_ev_")
                {
                    i += 1;
                    if !cancel
                        || lines
                            .get(i.saturating_sub(1))
                            .is_some_and(|s| s.contains("->store(true)"))
                    {
                        break;
                    }
                    continue;
                }
                if lines[i].starts_with("auto cancel_tok_")
                    || lines[i].starts_with("sc_core::sc_event done_ev_")
                {
                    i += 1;
                    continue;
                }
                break;
            }
            ops.push(SchedOp::Fork {
                join,
                branches,
                cancel_on_exit: cancel,
            });
            continue;
        }
        if let Some(sig) = l.strip_prefix("// uvir:wait_posedge ") {
            ops.push(SchedOp::WaitPosEdge {
                signal: sig.trim().to_string(),
            });
            i += 2; // skip companion wait
            continue;
        }
        if let Some(cb) = l.strip_prefix("// uvir:sample_1step ") {
            ops.push(SchedOp::SamplePreTimestep {
                clocking_name: cb.trim().to_string(),
            });
            i += 2;
            continue;
        }
        if let Some(rest) = l.strip_prefix("sc_core::wait(") {
            if let Some((ns_s, _)) = rest.split_once(", sc_core::SC_NS)") {
                if let Ok(ns) = ns_s.trim().parse::<u64>() {
                    ops.push(SchedOp::WaitDelayNs { ns });
                }
            }
        } else if let Some(v) = l
            .strip_prefix("this->start_item(")
            .and_then(|s| s.strip_suffix(");"))
        {
            ops.push(SchedOp::StartItem {
                item_var: v.trim().to_string(),
            });
        } else if let Some(v) = l
            .strip_prefix("this->finish_item(")
            .and_then(|s| s.strip_suffix(");"))
        {
            ops.push(SchedOp::FinishItem {
                item_var: v.trim().to_string(),
            });
        } else if let Some(v) = l
            .strip_prefix("(void)")
            .and_then(|s| s.strip_suffix("->randomize();"))
        {
            ops.push(SchedOp::RandomizeItem {
                item_var: v.trim().to_string(),
            });
        } else if let Some(rest) = l.strip_prefix("this->") {
            if let Some((port, rhs)) = rest.split_once(".write(") {
                if let Some(arg) = rhs.strip_suffix(");") {
                    ops.push(SchedOp::AnalysisWrite {
                        port: port.trim().to_string(),
                        expr: arg.trim().to_string(),
                    });
                }
            }
        }
        i += 1;
    }
    ops
}

fn parse_crv_variable_decl(line: &str, enum_widths: &HashMap<String, u32>) -> Option<FieldDecl> {
    let idx = line.find("crv_variable<")?;
    let after = &line[idx + "crv_variable<".len()..];
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
        ty: map_cpp_type(cpp_ty, enum_widths),
        is_rand: true,
        is_randc: false,
        ops: FieldOps::default(),
        ral_meta: None,
    })
}

fn parse_plain_cpp_member(line: &str, enum_widths: &HashMap<String, u32>) -> Option<FieldDecl> {
    if line.contains('(') && !line.contains('=') {
        return None;
    }
    if line.contains("_randc_ring_") || line.contains("pending_window_") || line.contains("_count_")
    {
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
        ty: map_cpp_type(cpp_ty, enum_widths),
        is_rand: false,
        is_randc: false,
        ops: FieldOps::default(),
        ral_meta: None,
    })
}

pub fn map_cpp_type(cpp_ty: &str, enum_widths: &HashMap<String, u32>) -> UvirType {
    let t = cpp_ty.trim().trim_start_matches("std::");
    if let Some(&ew) = enum_widths.get(t) {
        return UvirType::enum_ty(t, ew);
    }
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

fn parse_crave_expr(raw: &str, arena: &mut CrvArena) -> Result<ExprId, UvirError> {
    let s = strip_outer_parens(raw.trim());

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

    // `crave::dist(var(), crave::distribution<T>().add(0U, 10U).add_weighted_range(1U, 2U, 90U))`
    if let Some(inner) = s
        .strip_prefix("crave::dist(")
        .or_else(|| s.strip_prefix("dist("))
    {
        if let Some(args_s) = inner.strip_suffix(')') {
            let args = split_top_level_commas(args_s);
            if args.len() == 2 {
                let target = parse_crave_expr(&args[0], arena)?;
                let items = parse_crave_dist_chain(&args[1], arena)?;
                return Ok(arena.alloc(CrvExpr::Dist { target, items }));
            }
        }
    }

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

    if let Some(rest) = s.strip_prefix("static_cast<") {
        if let Some((_, after_gt)) = rest.split_once('>') {
            return parse_crave_expr(after_gt.trim(), arena);
        }
    }

    if let Some(var_name) = s.strip_suffix("()") {
        let clean = var_name.trim();
        if clean.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !clean.is_empty() {
            return Ok(arena.alloc(CrvExpr::Var(clean.to_string())));
        }
    }

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
        "svcc-front-sc/crave",
        format!("unable to parse CRAVE expression '{raw}'"),
    ))
}

fn parse_crave_dist_chain(chain: &str, arena: &mut CrvArena) -> Result<Vec<DistItem>, UvirError> {
    let mut items = Vec::new();
    let mut rem = chain;
    while let Some(dot) = rem.find(".add") {
        let after = &rem[dot + 1..];
        let Some(lp) = after.find('(') else {
            break;
        };
        let mname = &after[..lp];
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
        let Some(end_p) = rp else {
            break;
        };
        let args = split_top_level_commas(&after[lp + 1..end_p]);
        match (mname, args.as_slice()) {
            ("add", [val_s, w_s]) => {
                let v = parse_crave_expr(val_s, arena)?;
                let w = parse_cpp_u64(w_s).unwrap_or(1) as u32;
                items.push(DistItem {
                    elem: InsideElem::Single(v),
                    kind: DistWeightKind::PerValue,
                    weight: w,
                });
            }
            ("add_range", [lo_s, hi_s, w_s]) => {
                let low = parse_crave_expr(lo_s, arena)?;
                let high = parse_crave_expr(hi_s, arena)?;
                let w = parse_cpp_u64(w_s).unwrap_or(1) as u32;
                items.push(DistItem {
                    elem: InsideElem::Range { low, high },
                    kind: DistWeightKind::PerValue,
                    weight: w,
                });
            }
            ("add_weighted_range", [lo_s, hi_s, w_s]) => {
                let low = parse_crave_expr(lo_s, arena)?;
                let high = parse_crave_expr(hi_s, arena)?;
                let w = parse_cpp_u64(w_s).unwrap_or(1) as u32;
                items.push(DistItem {
                    elem: InsideElem::Range { low, high },
                    kind: DistWeightKind::SplitRange,
                    weight: w,
                });
            }
            _ => {}
        }
        rem = &after[end_p + 1..];
    }
    Ok(items)
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

fn parse_fc4sc_covergroup(name: &str, body: &str) -> (Option<String>, CovergroupDecl) {
    let mut coverpoints = Vec::new();
    let mut crosses = Vec::new();
    let mut owner_ty = None;

    for ln in body.lines() {
        let clean = ln.split("//").next().unwrap_or("").trim();
        if let Some(before) = clean.strip_suffix("sample_Point{};") {
            let ty = before.trim();
            if !ty.is_empty() && ty != "uint32_t" {
                owner_ty = Some(ty.to_string());
            }
        }
    }

    let uncommented: String = body
        .lines()
        .map(|ln| ln.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");

    for stmt in uncommented.split(';') {
        let l = stmt.trim();
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

    (
        owner_ty,
        CovergroupDecl {
            name: name.to_string(),
            coverpoints,
            crosses,
        },
    )
}

fn extract_fc4sc_bins(s: &str) -> Vec<CovBin> {
    let mut bins = Vec::new();
    let mut rem = s;
    while let Some(pos) = rem.find("bin<") {
        let after = &rem[pos..];
        let Some(lp) = after.find('(') else {
            break;
        };
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
        let Some(end_p) = rp else {
            break;
        };
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
