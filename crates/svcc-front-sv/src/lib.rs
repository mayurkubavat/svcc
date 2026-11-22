//! IEEE 1800-2023 SystemVerilog & IEEE 1800.2-2020 UVM front-end for `svcc-ir`.
//!
//! Powered by the pinned pure-Rust [`svlang`] + [`svlang_model`] SDK:
//! 1. Parses and semantically analyzes SV/UVM compilation units via [`svlang::Compilation`].
//! 2. Lifts `typedef enum`, `clocking #1step`, and concurrent SVA (`assert/cover property`) items.
//! 3. Lifts OOP/UVM hierarchy (`UvmKind`, `PhaseKind`, `Property`, `Method`, `SubComponentDecl`,
//!    `TlmPortDecl`, `TlmConnection`) + RAL `uvm_reg` and `uvm_reg_block` maps.
//! 4. Walks lossless CST nodes for `ConstraintDecl` (`inside`, `dist`, `soft`, `unique`),
//!    `CovergroupDecl` (`Coverpoint`, `Cross`), and procedural task bodies (`ParBlock` `fork/join`,
//!    `disable fork`, `wait fork`, `@(cb)`, `#delay`, sequence item dispatch).

pub mod crv_parser;
pub mod rtl_lower;

use std::collections::{HashMap, HashSet};
use std::time::Instant;
use svcc_ir::{
    BinKind, ClassDecl, ClockingDecl, ConstraintBlock, CovBin, CovergroupDecl, CoverpointDecl,
    CrossDecl, CrvArena, EnumDecl, FieldDecl, FieldOps, Frontend, FrontendMetrics, IrModule,
    IrType, JoinKind, MethodDecl, OwnershipMode, PhaseKind, Provenance, RalFieldMeta, RalMapSpec,
    RalRegInstance, ScalarKind, SchedOp, SourceLang, SubComponentDecl, SvaPropertyDecl, SvccError,
    TlmConnection, TlmPortDecl, TlmPortKind, UvmRole,
};
use svlang::{Compilation, NodeId, Severity, SyntaxKind, Tree};
use svlang_model::ral::extract_ral_blocks;
use svlang_model::{Design as SvModelDesign, PhaseKind as SvPhaseKind, UvmKind};

#[derive(Clone, Debug)]
pub struct SvFrontend {
    pub fail_on_syntax_error: bool,
    /// Recover written base/property types from lossless CST on standalone VIP snippets (`P6`).
    pub cst_fallback: bool,
}

impl Default for SvFrontend {
    fn default() -> Self {
        Self::new()
    }
}

impl SvFrontend {
    pub fn new() -> Self {
        Self {
            fail_on_syntax_error: true,
            cst_fallback: true,
        }
    }

    pub fn strict_semantic_only() -> Self {
        Self {
            fail_on_syntax_error: true,
            cst_fallback: false,
        }
    }
}

impl Frontend for SvFrontend {
    fn name(&self) -> &'static str {
        "svlang-1800-2023"
    }

    fn lower_str(
        &self,
        filename: &str,
        source: &str,
    ) -> Result<(IrModule, FrontendMetrics), SvccError> {
        let t_parse = Instant::now();
        let comp = Compilation::from_source(filename, source);
        let trees = comp.trees();
        if self.fail_on_syntax_error {
            let diags = comp.diagnostics();
            let syntax_errs: Vec<_> = diags
                .iter()
                .filter(|d| {
                    if d.severity != Severity::Error {
                        return false;
                    }
                    match d.stage {
                        svlang::DiagnosticStage::Parser => true,
                        svlang::DiagnosticStage::Preprocessor => {
                            !d.message.contains("unknown macro or directive `uvm_")
                                && !(self.cst_fallback
                                    && d.message.contains("cannot find include file"))
                        }
                        _ => false,
                    }
                })
                .collect();
            if !syntax_errs.is_empty() {
                let first = syntax_errs[0];
                return Err(SvccError::new(
                    "svcc-front-sv/parse",
                    format!(
                        "{}:{}:{}: {}",
                        first.file, first.line, first.column, first.message
                    ),
                ));
            }
        }
        let sv_design = SvModelDesign::from_sv(comp.design());
        let parse_us = t_parse.elapsed().as_micros() as u64;

        let t_lower = Instant::now();
        let ral_blocks = extract_ral_blocks(trees);
        let mut ral_regs_by_class = HashMap::new();
        let mut ral_blocks_by_class = HashMap::new();
        for blk in &ral_blocks {
            ral_blocks_by_class.insert(blk.name.clone(), blk.clone());
            for reg in &blk.registers {
                ral_regs_by_class.insert(reg.class_name.clone(), reg.clone());
            }
        }

        let mut module = IrModule::new(filename);
        let mut total_constraints = 0usize;
        let mut total_covergroups = 0usize;

        let mut cst_classes: HashMap<String, (&Tree, NodeId)> = HashMap::new();
        let mut named_sva_props: HashMap<String, String> = HashMap::new();
        for tree in trees {
            collect_top_cst_items(
                filename,
                tree,
                tree.root,
                &mut cst_classes,
                &mut module.enums,
                &mut module.clockings,
                &mut named_sva_props,
                &mut module.properties,
                &mut module.rtl_modules,
                &mut module.standalone_covergroups,
                &mut total_covergroups,
            );
        }

        let enum_widths: HashMap<String, u32> = module
            .enums
            .iter()
            .map(|e| (e.name.clone(), e.width))
            .collect();
        let mut enum_consts: Vec<(String, u64, u32)> = module
            .enums
            .iter()
            .flat_map(|e| {
                e.variants
                    .iter()
                    .map(move |(vn, vv)| (vn.clone(), *vv, e.width))
            })
            .collect();

        // Resolve `parameter` and `localparam` symbolic constants across packages, classes, and modules
        // via `svlang::semantic::oracle::TypeOracle::eval`!
        let oracle = comp.type_oracle();
        for (tree_idx, tree) in trees.iter().enumerate() {
            collect_oracle_param_consts(&oracle, tree_idx, tree, tree.root, &mut enum_consts);
        }

        // Enrich parameterized RTL port/signal bit-widths (`[DW-1:0]`, `typedef`s) via `comp.elaborate()`
        let elab = comp.elaborate();
        enrich_rtl_modules_from_elaboration(&elab, &mut module.rtl_modules);
        let clocking_names: HashSet<String> =
            module.clockings.iter().map(|c| c.name.clone()).collect();

        // Collect all user class names & roles first so component children resolve even if forward/external
        let mut known_component_classes: HashSet<String> = HashSet::new();
        for cls in sv_design.classes() {
            if cls.is_library || cls.is_interface {
                continue;
            }
            let cst_entry = cst_classes.get(&cls.name).copied();
            let eff_base = if !cls.base_name.is_empty() || !self.cst_fallback {
                cls.base_name.clone()
            } else if let Some((tree, cls_node)) = cst_entry {
                extract_cst_extends(tree, cls_node).0.unwrap_or_default()
            } else {
                String::new()
            };
            if map_uvm_role(cls.uvm, &eff_base).is_component() {
                known_component_classes.insert(cls.name.clone());
            }
        }

        for cls in sv_design.classes() {
            if cls.is_library || cls.is_interface {
                continue;
            }
            let cst_entry = cst_classes.get(&cls.name).copied();
            let (cst_base, item_type) = if self.cst_fallback {
                if let Some((tree, cls_node)) = cst_entry {
                    extract_cst_extends(tree, cls_node)
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
            let effective_base = if !cls.base_name.is_empty() {
                cls.base_name.clone()
            } else {
                cst_base.unwrap_or_default()
            };

            let role = map_uvm_role(cls.uvm, &effective_base);
            let ral_reg = ral_regs_by_class.get(&cls.name);
            let ral_map = ral_blocks_by_class.get(&cls.name).map(|blk| RalMapSpec {
                map_name: "default_map".into(),
                base_addr: blk.base_addr,
                addr_unit_bytes: blk.addr_unit_bytes,
                registers: blk
                    .registers
                    .iter()
                    .map(|r| RalRegInstance {
                        inst_name: r.name.clone(),
                        class_name: r.class_name.clone(),
                        offset: r.offset,
                        rights: r.rights.clone(),
                    })
                    .collect(),
            });

            let mut covergroups = Vec::new();
            if let Some((tree, cls_node)) = cst_entry {
                for ch in tree.children(cls_node) {
                    if tree.kind(ch) == SyntaxKind::CovergroupDecl {
                        if let Some(cg) = extract_covergroup_from_cst(tree, ch) {
                            total_covergroups += 1;
                            covergroups.push(cg);
                        }
                    }
                }
            }

            let mut fields = Vec::new();
            let mut subcomponents = Vec::new();
            let mut tlm_ports = Vec::new();

            if let Some(reg_spec) = ral_reg {
                for rf in &reg_spec.fields {
                    fields.push(FieldDecl {
                        name: rf.name.clone(),
                        ty: IrType::bitvec(rf.width, false),
                        is_rand: rf.access != "RO",
                        is_randc: false,
                        ops: FieldOps::default(),
                        ral_meta: Some(RalFieldMeta {
                            lsb: rf.lsb,
                            width: rf.width,
                            access: rf.access.clone(),
                            volatile: rf.volatile,
                            reset: rf.reset,
                            has_reset: rf.has_reset,
                        }),
                    });
                }
            } else if role != UvmRole::RegBlock {
                for prop in &cls.properties {
                    let resolved_ty = if prop.ty.trim() == "<error>" && self.cst_fallback {
                        written_property_type(cst_entry, &prop.name)
                            .unwrap_or_else(|| prop.ty.clone())
                    } else {
                        prop.ty.clone()
                    };
                    let pty = resolved_ty.trim();
                    if pty == "uvm_reg_field"
                        || prop.is_virtual_interface
                        || covergroups.iter().any(|cg| cg.name == prop.name)
                    {
                        continue;
                    }
                    if let Some(port) = parse_tlm_port_decl(&prop.name, pty) {
                        tlm_ports.push(port);
                        continue;
                    }
                    let base_ty = pty.split('#').next().unwrap_or(pty).trim();
                    if role.is_component() && known_component_classes.contains(base_ty) {
                        subcomponents.push(SubComponentDecl {
                            name: prop.name.clone(),
                            class_name: base_ty.to_string(),
                        });
                        continue;
                    }
                    let ty = parse_sv_property_type(pty, &enum_widths);
                    let is_randc = source_property_is_randc(cst_entry, &prop.name);
                    fields.push(FieldDecl {
                        name: prop.name.clone(),
                        ty,
                        is_rand: prop.is_rand || is_randc,
                        is_randc,
                        ops: FieldOps::default(),
                        ral_meta: None,
                    });
                }
            }

            apply_uvm_field_macros_from_source(source, &cls.name, &mut fields);

            let mut constraints = Vec::new();
            if let Some((tree, cls_node)) = cst_entry {
                constraints =
                    extract_class_constraints(tree, cls_node, &mut module.crv_arena, &enum_consts)?;
                total_constraints += constraints.len();
            }

            let tlm_connections = extract_tlm_connections_from_source(source, &cls.name);
            let cst_method_bodies = cst_entry
                .map(|(tree, cls_node)| extract_cst_method_bodies(tree, cls_node, &clocking_names))
                .unwrap_or_default();

            let methods = cls
                .methods
                .iter()
                .filter(|m| !m.is_builtin && m.name != "new")
                .map(|m| MethodDecl {
                    name: m.name.clone(),
                    phase: map_phase_kind(&m.name, m.phase),
                    is_task: m.is_task,
                    is_virtual: m.is_virtual,
                    body_ops: cst_method_bodies.get(&m.name).cloned().unwrap_or_default(),
                })
                .collect();

            let macros = cls.macros.iter().map(|m| m.name.clone()).collect();
            module.classes.push(ClassDecl {
                name: cls.name.clone(),
                package_or_namespace: cls.scope.clone(),
                role,
                base_class: (!effective_base.is_empty()).then_some(effective_base),
                item_type,
                ownership: OwnershipMode::SvGcHandle,
                sv_signedness: true,
                needs_value_copy_ctor: false,
                synthesize_field_methods: role == UvmRole::SequenceItem || role == UvmRole::Object,
                reg_width_bits: ral_reg
                    .map(|r| r.n_bits)
                    .or_else(|| (role == UvmRole::Reg).then_some(32)),
                ral_map,
                subcomponents,
                tlm_ports,
                tlm_connections,
                fields,
                constraints,
                covergroups,
                methods,
                provenance: Provenance {
                    source_lang: Some(SourceLang::SystemVerilogUvm),
                    file: filename.to_string(),
                    line: cls.loc.line,
                    original_macros: macros,
                },
            });
        }

        let lower_us = t_lower.elapsed().as_micros() as u64;
        let metrics = FrontendMetrics {
            frontend: self.name().to_string(),
            parse_us,
            lower_us,
            classes_lowered: module.classes.len(),
            constraints_lowered: total_constraints,
            covergroups_lowered: total_covergroups,
            properties_lowered: module.properties.len(),
            rtl_modules_lowered: module.rtl_modules.len(),
        };
        Ok((module, metrics))
    }
}

fn collect_oracle_param_consts(
    oracle: &svlang::semantic::oracle::TypeOracle,
    tree_idx: usize,
    tree: &Tree,
    node: NodeId,
    out: &mut Vec<(String, u64, u32)>,
) {
    if tree.kind(node) == SyntaxKind::ParamDecl {
        for ch in tree.children(node) {
            if tree.kind(ch) == SyntaxKind::Declarator {
                let pname = tree
                    .child(ch, SyntaxKind::Name)
                    .map(|n| tree.bare_text(n).trim().to_string());
                let val_node = tree
                    .children(ch)
                    .find(|&c| !matches!(tree.kind(c), SyntaxKind::Name | SyntaxKind::Dim));
                if let (Some(name), Some(vn)) = (pname, val_node) {
                    if let Some((ival, w, _signed)) = oracle.eval(tree_idx, vn) {
                        out.push((name, ival as u64, w.max(1)));
                    }
                }
            }
        }
    }
    for ch in tree.children(node) {
        collect_oracle_param_consts(oracle, tree_idx, tree, ch, out);
    }
}

fn enrich_rtl_modules_from_elaboration(
    elab: &svlang::Elaborated,
    rtl_modules: &mut [svcc_ir::RtlModule],
) {
    for rtl_m in rtl_modules {
        let Some(inst) = elab
            .design
            .instances
            .iter()
            .find(|i| i.definition == rtl_m.name)
        else {
            continue;
        };
        for &sid in &inst.ports {
            let sym = elab.design.symbol(sid);
            if let Some(bw) = sym.ty.bit_width() {
                if let Some(p) = rtl_m.ports.iter_mut().find(|p| p.name == sym.name) {
                    p.width = bw.max(1);
                    p.signed = sym.ty.is_signed();
                }
            }
        }
        for &sid in &inst.symbols {
            let sym = elab.design.symbol(sid);
            if let Some(bw) = sym.ty.bit_width() {
                if let Some(sig) = rtl_m.signals.iter_mut().find(|s| s.name == sym.name) {
                    sig.width = bw.max(1);
                    sig.signed = sym.ty.is_signed();
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_top_cst_items<'a>(
    filename: &str,
    tree: &'a Tree,
    node: NodeId,
    cst_classes: &mut HashMap<String, (&'a Tree, NodeId)>,
    enums: &mut Vec<EnumDecl>,
    clockings: &mut Vec<ClockingDecl>,
    named_props: &mut HashMap<String, String>,
    sva_props: &mut Vec<SvaPropertyDecl>,
    rtl_modules: &mut Vec<svcc_ir::RtlModule>,
    standalone_cgs: &mut Vec<CovergroupDecl>,
    total_cgs: &mut usize,
) {
    match tree.kind(node) {
        SyntaxKind::ModuleDecl => {
            if let Some(rtl_m) = rtl_lower::try_lower_sv_module(filename, tree, node) {
                rtl_modules.push(rtl_m);
            }
            for ch in tree.children(node) {
                collect_top_cst_items(
                    filename,
                    tree,
                    ch,
                    cst_classes,
                    enums,
                    clockings,
                    named_props,
                    sva_props,
                    rtl_modules,
                    standalone_cgs,
                    total_cgs,
                );
            }
        }
        SyntaxKind::ClassDecl => {
            if let Some(nm) = tree.child(node, SyntaxKind::Name) {
                cst_classes.insert(tree.bare_text(nm).trim().to_string(), (tree, node));
            }
        }
        SyntaxKind::TypedefDecl => {
            if let Some(en) = parse_typedef_enum(&tree.text(node)) {
                enums.push(en);
            }
        }
        SyntaxKind::ClockingDecl => {
            if let Some(cb) = parse_clocking_decl(tree, node) {
                clockings.push(cb);
            }
        }
        SyntaxKind::PropertyDecl => {
            if let Some((pname, body)) = parse_named_property_decl(tree, node) {
                named_props.insert(pname, body);
            }
        }
        SyntaxKind::ConcurrentAssert => {
            if let Some(prop) = parse_concurrent_assert(tree, node, named_props, sva_props.len()) {
                sva_props.push(prop);
            }
        }
        SyntaxKind::CovergroupDecl => {
            if let Some(cg) = extract_covergroup_from_cst(tree, node) {
                *total_cgs += 1;
                standalone_cgs.push(cg);
            }
        }
        _ => {
            for ch in tree.children(node) {
                collect_top_cst_items(
                    filename,
                    tree,
                    ch,
                    cst_classes,
                    enums,
                    clockings,
                    named_props,
                    sva_props,
                    rtl_modules,
                    standalone_cgs,
                    total_cgs,
                );
            }
        }
    }
}

fn parse_typedef_enum(raw: &str) -> Option<EnumDecl> {
    if !raw.contains("enum") {
        return None;
    }
    let lb = raw.find('{')?;
    let rb = raw.rfind('}')?;
    let header = &raw[..lb];
    let name = raw[rb + 1..]
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    let width = if let (Some(bl), Some(br)) = (header.find('['), header.find(']')) {
        let rng = &header[bl + 1..br];
        if let Some((m, l)) = rng.split_once(':') {
            let msb = m.trim().parse::<i64>().unwrap_or(1);
            let lsb = l.trim().parse::<i64>().unwrap_or(0);
            ((msb - lsb).unsigned_abs() as u32) + 1
        } else {
            32
        }
    } else {
        32
    };

    let mut variants = Vec::new();
    let mut next_val = 0u64;
    for item in raw[lb + 1..rb].split(',') {
        let it = item.trim();
        if it.is_empty() {
            continue;
        }
        if let Some((vname, vexpr)) = it.split_once('=') {
            let val = crv_parser::parse_sv_literal(vexpr.trim())
                .map(|t| t.0 as u64)
                .unwrap_or(next_val);
            variants.push((vname.trim().to_string(), val));
            next_val = val + 1;
        } else {
            variants.push((it.to_string(), next_val));
            next_val += 1;
        }
    }
    Some(EnumDecl {
        name,
        width,
        variants,
    })
}

fn parse_clocking_decl(tree: &Tree, node: NodeId) -> Option<ClockingDecl> {
    let raw = tree.text(node);
    let name = tree
        .child(node, SyntaxKind::Name)
        .map(|n| tree.bare_text(n).trim().to_string())
        .unwrap_or_else(|| "cb".into());
    let mut clock_signal = "clk".to_string();
    if let Some(at_idx) = raw.find("@(") {
        if let Some(end_p) = raw[at_idx + 2..].find(')') {
            let ev = raw[at_idx + 2..at_idx + 2 + end_p].trim();
            clock_signal = ev
                .trim_start_matches("posedge")
                .trim_start_matches("negedge")
                .trim()
                .to_string();
        }
    }

    let mut input_1step = Vec::new();
    let mut outputs = Vec::new();
    for line in raw.lines() {
        let l = line.trim().trim_end_matches(';');
        if l.starts_with("default ") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("input") {
            let clean = rest.trim().trim_start_matches("#1step").trim();
            for sig in clean.split(',') {
                let s = sig.split_whitespace().last().unwrap_or("").trim();
                if !s.is_empty() {
                    input_1step.push(s.to_string());
                }
            }
        } else if let Some(rest) = l.strip_prefix("output") {
            let clean = rest
                .trim()
                .trim_start_matches("#0")
                .trim_start_matches("#1")
                .trim();
            for sig in clean.split(',') {
                let s = sig.split_whitespace().last().unwrap_or("").trim();
                if !s.is_empty() {
                    outputs.push(s.to_string());
                }
            }
        }
    }
    Some(ClockingDecl {
        name,
        clock_signal,
        input_1step_signals: input_1step,
        output_signals: outputs,
    })
}

fn parse_named_property_decl(tree: &Tree, node: NodeId) -> Option<(String, String)> {
    let name = tree
        .child(node, SyntaxKind::Name)
        .map(|n| tree.bare_text(n).trim().to_string())?;
    let raw = tree.text(node);
    let (_, after_semi) = raw.split_once(';')?;
    let body = after_semi
        .rsplit_once("endproperty")
        .map(|(b, _)| b.trim().trim_end_matches(';').trim().to_string())?;
    Some((name, body))
}

fn parse_concurrent_assert(
    tree: &Tree,
    node: NodeId,
    named_props: &HashMap<String, String>,
    idx: usize,
) -> Option<SvaPropertyDecl> {
    let raw = tree.text(node);
    let trimmed = raw.trim();
    let (label, rest) = if let Some(lbl) = tree.child(node, SyntaxKind::StmtLabel) {
        let l = tree.bare_text(lbl).trim().trim_end_matches(':').to_string();
        let after = trimmed
            .split_once(':')
            .map(|x| x.1.trim())
            .unwrap_or(trimmed);
        (l, after)
    } else {
        (format!("sva_prop_{idx}"), trimmed)
    };

    let is_cover = rest.starts_with("cover ");
    let lp = rest.find('(')?;
    // Extract outer `property ( ... )` balanced parens
    let mut depth = 0usize;
    let mut rp = None;
    for (i, ch) in rest[lp..].char_indices() {
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
    let inner_raw = rest[lp + 1..rp?].trim();
    let spec = named_props
        .get(inner_raw)
        .map(String::as_str)
        .unwrap_or(inner_raw);
    parse_sva_property_body(label, is_cover, spec)
}

pub fn parse_sva_property_body(
    name: String,
    is_cover: bool,
    spec: &str,
) -> Option<SvaPropertyDecl> {
    let mut rem = spec.trim();
    let mut clock_signal = "clk".to_string();
    if let Some(after_at) = rem.strip_prefix("@(") {
        let end_p = after_at.find(')')?;
        let ev = after_at[..end_p].trim();
        clock_signal = ev
            .trim_start_matches("posedge")
            .trim_start_matches("negedge")
            .trim()
            .to_string();
        rem = after_at[end_p + 1..].trim();
    }

    let mut disable_iff = None;
    if let Some(after_dis) = rem.strip_prefix("disable iff") {
        let after_dis = after_dis.trim();
        if let Some(after_lp) = after_dis.strip_prefix('(') {
            let mut depth = 1usize;
            let mut end_d = 0usize;
            for (i, ch) in after_lp.char_indices() {
                if ch == '(' {
                    depth += 1;
                } else if ch == ')' {
                    depth -= 1;
                    if depth == 0 {
                        end_d = i;
                        break;
                    }
                }
            }
            disable_iff = Some(after_lp[..end_d].trim().to_string());
            rem = after_lp[end_d + 1..].trim();
        }
    }

    let (antecedent, default_min, rhs_raw) = if let Some((a, b)) = rem.split_once("|->") {
        (a.trim().to_string(), 0u32, b.trim())
    } else if let Some((a, b)) = rem.split_once("|=>") {
        (a.trim().to_string(), 1u32, b.trim())
    } else {
        return None;
    };

    let (min_delay, max_delay, consequent) = if let Some(after_h) = rhs_raw.strip_prefix("##[") {
        let rb = after_h.find(']')?;
        let rng = &after_h[..rb];
        let (lo_s, hi_s) = rng.split_once(':')?;
        let lo = lo_s.trim().parse::<u32>().unwrap_or(default_min) + default_min;
        let hi = hi_s.trim().parse::<u32>().unwrap_or(lo) + default_min;
        (lo, hi, after_h[rb + 1..].trim().to_string())
    } else if let Some(after_h) = rhs_raw.strip_prefix("##") {
        let num_end = after_h
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after_h.len());
        let d = after_h[..num_end].parse::<u32>().unwrap_or(0) + default_min;
        (d, d, after_h[num_end..].trim().to_string())
    } else {
        (default_min, default_min, rhs_raw.to_string())
    };

    Some(SvaPropertyDecl {
        name,
        clock_signal,
        disable_iff,
        antecedent,
        min_delay,
        max_delay,
        consequent,
        is_cover,
    })
}

fn parse_tlm_port_decl(name: &str, pty: &str) -> Option<TlmPortDecl> {
    let item_ty = if let (Some(lp), Some(rp)) = (pty.find('('), pty.rfind(')')) {
        pty[lp + 1..rp]
            .split(',')
            .next()
            .unwrap_or("uvm_sequence_item")
            .trim()
            .to_string()
    } else {
        "uvm_sequence_item".into()
    };

    if pty.starts_with("uvm_analysis_port") {
        Some(TlmPortDecl {
            name: name.to_string(),
            kind: TlmPortKind::AnalysisPort,
            item_type: item_ty,
        })
    } else if pty.starts_with("uvm_analysis_imp") || pty.starts_with("uvm_analysis_export") {
        Some(TlmPortDecl {
            name: name.to_string(),
            kind: TlmPortKind::AnalysisExport,
            item_type: item_ty,
        })
    } else if pty.starts_with("uvm_seq_item_pull_port") {
        Some(TlmPortDecl {
            name: name.to_string(),
            kind: TlmPortKind::SeqItemPullPort,
            item_type: item_ty,
        })
    } else {
        None
    }
}

fn extract_tlm_connections_from_source(source: &str, class_name: &str) -> Vec<TlmConnection> {
    let mut out = Vec::new();
    let marker = format!("class {class_name}");
    let Some(start) = source.find(&marker) else {
        return out;
    };
    let body = source[start..].split("endclass").next().unwrap_or("");
    for line in body.lines() {
        let l = line
            .split("//")
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches(';');
        if let Some((lhs, rhs)) = l.split_once(".connect(") {
            if let Some(target) = rhs.strip_suffix(')') {
                out.push(TlmConnection {
                    initiator: lhs.trim().to_string(),
                    target: target.trim().to_string(),
                });
            }
        }
    }
    out
}

fn extract_cst_method_bodies(
    tree: &Tree,
    cls_node: NodeId,
    clocking_names: &HashSet<String>,
) -> HashMap<String, Vec<SchedOp>> {
    let mut map = HashMap::new();
    for ch in tree.children(cls_node) {
        let k = tree.kind(ch);
        if matches!(k, SyntaxKind::FunctionDecl | SyntaxKind::TaskDecl) {
            let Some(nm) = tree.child(ch, SyntaxKind::Name) else {
                continue;
            };
            let mname = tree.bare_text(nm).trim().to_string();
            if matches!(
                mname.as_str(),
                "new" | "build" | "build_phase" | "connect_phase" | "do_copy" | "do_compare"
            ) {
                continue;
            }
            if let Some(body_node) = tree.child(ch, SyntaxKind::Body) {
                let ops = lower_stmt_children(tree, body_node, clocking_names);
                if !ops.is_empty() {
                    map.insert(mname, ops);
                }
            }
        }
    }
    map
}

fn lower_stmt_children(
    tree: &Tree,
    container: NodeId,
    clocking_names: &HashSet<String>,
) -> Vec<SchedOp> {
    let mut ops = Vec::new();
    for stmt in tree.children(container) {
        lower_single_stmt(tree, stmt, clocking_names, &mut ops);
    }
    // Fold `Fork { JoinAny, .. }` immediately followed by `DisableFork` into `cancel_on_exit = true`
    let mut folded: Vec<SchedOp> = Vec::with_capacity(ops.len());
    for op in ops {
        if matches!(op, SchedOp::DisableFork) {
            if let Some(SchedOp::Fork { cancel_on_exit, .. }) = folded.last_mut() {
                *cancel_on_exit = true;
                continue;
            }
        }
        folded.push(op);
    }
    folded
}

fn lower_single_stmt(
    tree: &Tree,
    stmt: NodeId,
    clocking_names: &HashSet<String>,
    out: &mut Vec<SchedOp>,
) {
    match tree.kind(stmt) {
        SyntaxKind::SeqBlock | SyntaxKind::Body => {
            for ch in tree.children(stmt) {
                lower_single_stmt(tree, ch, clocking_names, out);
            }
        }
        SyntaxKind::ParBlock => {
            let full = tree.text(stmt);
            let join = if full.trim_end().ends_with("join_any") {
                JoinKind::JoinAny
            } else if full.trim_end().ends_with("join_none") {
                JoinKind::JoinNone
            } else {
                JoinKind::JoinAll
            };
            let mut branches = Vec::new();
            for ch in tree.children(stmt) {
                let mut b_ops = Vec::new();
                lower_single_stmt(tree, ch, clocking_names, &mut b_ops);
                if !b_ops.is_empty() {
                    branches.push(b_ops);
                }
            }
            out.push(SchedOp::Fork {
                join,
                branches,
                cancel_on_exit: false,
            });
        }
        SyntaxKind::DisableStmt => {
            let txt = tree.text(stmt);
            if txt.contains("fork") {
                out.push(SchedOp::DisableFork);
            }
        }
        SyntaxKind::WaitStmt => {
            let txt = tree.text(stmt);
            if txt.contains("wait fork") {
                out.push(SchedOp::WaitFork);
            }
        }
        SyntaxKind::TimedStmt => {
            if let Some(tc) = tree.child(stmt, SyntaxKind::TimingControl) {
                lower_timing_control(&tree.text(tc), clocking_names, out);
            }
            for ch in tree.children(stmt) {
                if tree.kind(ch) != SyntaxKind::TimingControl {
                    lower_single_stmt(tree, ch, clocking_names, out);
                }
            }
        }
        SyntaxKind::TimingControl => {
            lower_timing_control(&tree.text(stmt), clocking_names, out);
        }
        SyntaxKind::LoopStmt => {
            let raw = tree.text(stmt);
            let trimmed = raw.trim();
            if let Some(rest) = trimmed.strip_prefix("repeat") {
                if let (Some(lp), Some(rp)) = (rest.find('('), rest.find(')')) {
                    let count = rest[lp + 1..rp].trim().parse::<u32>().unwrap_or(1);
                    let mut body = Vec::new();
                    for ch in tree.children(stmt) {
                        lower_single_stmt(tree, ch, clocking_names, &mut body);
                    }
                    out.push(SchedOp::Repeat { count, body });
                }
            }
        }
        SyntaxKind::ExprStmt => {
            let txt = tree
                .text(stmt)
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
            if txt.is_empty() {
                return;
            }
            if let Some(inner) = txt
                .strip_prefix("start_item(")
                .and_then(|s| s.strip_suffix(')'))
            {
                out.push(SchedOp::StartItem {
                    item_var: inner.trim().to_string(),
                });
            } else if let Some(inner) = txt
                .strip_prefix("finish_item(")
                .and_then(|s| s.strip_suffix(')'))
            {
                out.push(SchedOp::FinishItem {
                    item_var: inner.trim().to_string(),
                });
            } else if txt.contains(".randomize()") {
                let var = txt
                    .split(".randomize()")
                    .next()
                    .unwrap_or("")
                    .rsplit(['(', ' ', '!'])
                    .next()
                    .unwrap_or("req")
                    .trim();
                out.push(SchedOp::RandomizeItem {
                    item_var: var.to_string(),
                });
            } else if let Some((port, rest)) = txt.split_once(".write(") {
                if let Some(arg) = rest.strip_suffix(')') {
                    out.push(SchedOp::AnalysisWrite {
                        port: port.trim().to_string(),
                        expr: arg.trim().to_string(),
                    });
                }
            } else if let Some((lhs, rhs)) = txt.split_once("<=") {
                out.push(SchedOp::Assign {
                    lhs: lhs.trim().to_string(),
                    rhs: rhs.trim().to_string(),
                    nonblocking: true,
                });
            } else if let Some((lhs, rhs)) = txt.split_once('=') {
                if !lhs.ends_with('=') && !lhs.ends_with('!') {
                    out.push(SchedOp::Assign {
                        lhs: lhs.trim().to_string(),
                        rhs: rhs.trim().to_string(),
                        nonblocking: false,
                    });
                }
            } else {
                out.push(SchedOp::Call { stmt: txt });
            }
        }
        _ => {}
    }
}

fn lower_timing_control(raw: &str, clocking_names: &HashSet<String>, out: &mut Vec<SchedOp>) {
    let t = raw.trim();
    if let Some(delay_s) = t.strip_prefix('#') {
        let digits: String = delay_s.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(ns) = digits.parse::<u64>() {
            out.push(SchedOp::WaitDelayNs { ns });
        }
    } else if let Some(ev_s) = t.strip_prefix("@(").and_then(|s| s.strip_suffix(')')) {
        let ev = ev_s.trim();
        if clocking_names.contains(ev) || ev.ends_with(".cb") || ev == "cb" {
            out.push(SchedOp::SamplePreTimestep {
                clocking_name: ev.to_string(),
            });
        } else {
            let sig = ev
                .trim_start_matches("posedge")
                .trim_start_matches("negedge")
                .trim()
                .to_string();
            out.push(SchedOp::WaitPosEdge { signal: sig });
        }
    }
}

fn map_uvm_role(uvm: UvmKind, base_name: &str) -> UvmRole {
    let b = base_name.split("::").last().unwrap_or(base_name).trim();
    if b == "uvm_reg" {
        return UvmRole::Reg;
    }
    if b == "uvm_reg_block" {
        return UvmRole::RegBlock;
    }
    match uvm {
        UvmKind::SequenceItem => return UvmRole::SequenceItem,
        UvmKind::Sequence => return UvmRole::Sequence,
        UvmKind::Test => return UvmRole::Test,
        UvmKind::Env => return UvmRole::Env,
        UvmKind::Agent => return UvmRole::Agent,
        UvmKind::Driver => return UvmRole::Driver,
        UvmKind::Monitor => return UvmRole::Monitor,
        UvmKind::Sequencer => return UvmRole::Sequencer,
        UvmKind::Scoreboard => return UvmRole::Scoreboard,
        UvmKind::Subscriber => return UvmRole::Subscriber,
        UvmKind::Component => return UvmRole::Component,
        UvmKind::Object => return UvmRole::Object,
        UvmKind::None => {}
    }
    match b {
        "uvm_sequence_item" => UvmRole::SequenceItem,
        "uvm_sequence" | "uvm_sequence_base" => UvmRole::Sequence,
        "uvm_driver" => UvmRole::Driver,
        "uvm_monitor" => UvmRole::Monitor,
        "uvm_sequencer" => UvmRole::Sequencer,
        "uvm_agent" => UvmRole::Agent,
        "uvm_env" => UvmRole::Env,
        "uvm_scoreboard" => UvmRole::Scoreboard,
        "uvm_subscriber" => UvmRole::Subscriber,
        "uvm_test" => UvmRole::Test,
        "uvm_component" => UvmRole::Component,
        "uvm_object" | "uvm_transaction" => UvmRole::Object,
        _ => UvmRole::PlainClass,
    }
}

fn map_phase_kind(name: &str, sv_phase: SvPhaseKind) -> PhaseKind {
    match name {
        "build_phase" => PhaseKind::Build,
        "connect_phase" => PhaseKind::Connect,
        "end_of_elaboration_phase" => PhaseKind::EndOfElaboration,
        "start_of_simulation_phase" => PhaseKind::StartOfSimulation,
        "run_phase" | "main_phase" => PhaseKind::Run,
        "extract_phase" => PhaseKind::Extract,
        "check_phase" => PhaseKind::Check,
        "report_phase" => PhaseKind::Report,
        "final_phase" => PhaseKind::Final,
        "body" => PhaseKind::SequenceBody,
        _ => match sv_phase {
            SvPhaseKind::Task => PhaseKind::Run,
            SvPhaseKind::Function | SvPhaseKind::None => PhaseKind::Custom,
        },
    }
}

pub fn parse_sv_property_type(raw_ty: &str, enum_widths: &HashMap<String, u32>) -> IrType {
    let t = raw_ty.trim();
    if let Some(&ew) = enum_widths.get(t) {
        return IrType::enum_ty(t, ew);
    }
    if t == "string" {
        return IrType {
            kind: ScalarKind::String,
            width: 0,
            signed: false,
            is_four_state: false,
        };
    }
    if t == "real" || t == "shortreal" {
        return IrType {
            kind: ScalarKind::Real,
            width: 64,
            signed: true,
            is_four_state: false,
        };
    }

    let explicit_unsigned = t.contains("unsigned");
    let explicit_signed = !explicit_unsigned && t.split_whitespace().any(|w| w == "signed");

    let base_word = t
        .split(|c: char| c.is_whitespace() || c == '[')
        .next()
        .unwrap_or("");

    let (default_w, default_signed, four_state) = match base_word {
        "bit" => (1, false, false),
        "logic" | "reg" => (1, false, true),
        "byte" => (8, true, false),
        "shortint" => (16, true, false),
        "int" => (32, true, false),
        "longint" => (64, true, false),
        "integer" => (32, true, true),
        "time" => (64, false, true),
        other if !other.is_empty() => {
            return IrType {
                kind: ScalarKind::ClassRef(other.to_string()),
                width: 64,
                signed: false,
                is_four_state: false,
            };
        }
        _ => (32, false, false),
    };

    let width = if let (Some(lb), Some(rb)) = (t.find('['), t.find(']')) {
        let range = &t[lb + 1..rb];
        if let Some((msb_s, lsb_s)) = range.split_once(':') {
            let msb = msb_s
                .trim()
                .parse::<i64>()
                .unwrap_or((default_w - 1) as i64);
            let lsb = lsb_s.trim().parse::<i64>().unwrap_or(0);
            ((msb - lsb).unsigned_abs() as u32) + 1
        } else {
            default_w
        }
    } else {
        default_w
    };

    let signed = if explicit_unsigned {
        false
    } else if explicit_signed {
        true
    } else {
        default_signed
    };

    if four_state {
        IrType::logicvec(width, signed)
    } else {
        IrType::bitvec(width, signed)
    }
}

fn source_property_is_randc(cst_cls: Option<(&Tree, NodeId)>, prop_name: &str) -> bool {
    let Some((tree, cls_node)) = cst_cls else {
        return false;
    };
    for ch in tree.children(cls_node) {
        let txt = tree.text(ch);
        if txt.contains("randc") && txt.contains(prop_name) {
            return true;
        }
    }
    false
}

fn written_property_type(cst_cls: Option<(&Tree, NodeId)>, prop_name: &str) -> Option<String> {
    let (tree, cls_node) = cst_cls?;
    for ch in tree.children(cls_node) {
        if tree.kind(ch) == SyntaxKind::DataDecl {
            let txt = tree.text(ch);
            if let Some(dt) = tree.child(ch, SyntaxKind::DataType) {
                // Verify one of the declarators matches `prop_name`
                let matches_decl = tree
                    .children(ch)
                    .filter(|&n| tree.kind(n) == SyntaxKind::Declarator)
                    .any(|d| {
                        tree.child(d, SyntaxKind::Name)
                            .is_some_and(|nm| tree.bare_text(nm).trim() == prop_name)
                    });
                if matches_decl || txt.contains(prop_name) {
                    return Some(tree.text(dt).trim().to_string());
                }
            }
        }
    }
    None
}

/// Returns `(base_class_name, optional_parameter_item_type)` from `class X extends Base #(Item);`.
fn extract_cst_extends(tree: &Tree, cls_node: NodeId) -> (Option<String>, Option<String>) {
    let raw = tree.text(cls_node);
    let Some(header) = raw.split(';').next() else {
        return (None, None);
    };
    let Some((_, after)) = header.split_once("extends") else {
        return (None, None);
    };
    let base = after
        .trim()
        .split(['#', '(', ' ', '\n', '\t'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let item = if let (Some(lp), Some(rp)) = (after.find("#("), after.rfind(')')) {
        let param = after[lp + 2..rp]
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        (!param.is_empty()).then_some(param)
    } else {
        None
    };
    ((!base.is_empty()).then_some(base), item)
}

fn apply_uvm_field_macros_from_source(source: &str, class_name: &str, fields: &mut [FieldDecl]) {
    let marker = format!("class {class_name}");
    let Some(start) = source.find(&marker) else {
        return;
    };
    let tail = &source[start..];
    let body = tail.split("endclass").next().unwrap_or(tail);
    for line in body.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("`uvm_field_int(") {
            if let Some(inner) = rest.strip_suffix(')') {
                if let Some((fname, flags)) = inner.split_once(',') {
                    let fname = fname.trim();
                    if let Some(f) = fields.iter_mut().find(|f| f.name == fname) {
                        if flags.contains("UVM_NOCOMPARE") {
                            f.ops.compare = false;
                        }
                        if flags.contains("UVM_NOCOPY") {
                            f.ops.copy = false;
                        }
                        if flags.contains("UVM_NOPRINT") {
                            f.ops.print = false;
                        }
                        if flags.contains("UVM_NOPACK") {
                            f.ops.pack = false;
                        }
                    }
                }
            }
        }
    }
}

fn extract_class_constraints(
    tree: &Tree,
    cls_node: NodeId,
    arena: &mut CrvArena,
    enum_consts: &[(String, u64, u32)],
) -> Result<Vec<ConstraintBlock>, SvccError> {
    let mut blocks = Vec::new();
    for ch in tree.children(cls_node) {
        if tree.kind(ch) != SyntaxKind::ConstraintDecl {
            continue;
        }
        let cname = tree
            .child(ch, SyntaxKind::Name)
            .map(|n| tree.bare_text(n).trim().to_string())
            .unwrap_or_else(|| format!("c_{}", blocks.len()));
        let mut clauses = Vec::new();
        if let Some(cblock) = tree.child(ch, SyntaxKind::ConstraintBlock) {
            collect_constraint_clauses(tree, cblock, arena, enum_consts, &mut clauses)?;
        }
        blocks.push(ConstraintBlock {
            name: cname,
            clauses,
        });
    }
    Ok(blocks)
}

fn collect_constraint_clauses(
    tree: &Tree,
    block_node: NodeId,
    arena: &mut CrvArena,
    enum_consts: &[(String, u64, u32)],
    out: &mut Vec<svcc_ir::ConstraintClause>,
) -> Result<(), SvccError> {
    for item in tree.children(block_node) {
        if tree.kind(item) != SyntaxKind::ConstraintItem {
            continue;
        }
        if let Some(nested) = tree.child(item, SyntaxKind::ConstraintBlock) {
            let item_txt = tree.text(item);
            let trimmed = item_txt.trim();
            if trimmed.starts_with('{') {
                collect_constraint_clauses(tree, nested, arena, enum_consts, out)?;
                continue;
            }
        }
        let raw = tree.text(item);
        let stripped_item = strip_sv_comments(&raw);
        let st_trim = stripped_item.trim();
        // IEEE 1800-2023 §18.5.10 `solve ... before ...` is a distribution hint that never changes
        // the satisfiable solution set; skip non-value ordering hints cleanly.
        if st_trim.starts_with("solve ") {
            continue;
        }
        if let Some(clause) = crv_parser::lower_sv_constraint_item(&raw, arena, enum_consts)? {
            out.push(clause);
        }
    }
    Ok(())
}

fn extract_covergroup_from_cst(tree: &Tree, cg_node: NodeId) -> Option<CovergroupDecl> {
    let name = tree
        .child(cg_node, SyntaxKind::Name)
        .map(|n| tree.bare_text(n).trim().to_string())?;
    let mut coverpoints = Vec::new();
    let mut crosses = Vec::new();

    for ch in tree.children(cg_node) {
        match tree.kind(ch) {
            SyntaxKind::Coverpoint => {
                let raw = tree.text(ch);
                let cp = parse_coverpoint_text(&raw, coverpoints.len());
                coverpoints.push(cp);
            }
            SyntaxKind::Cross => {
                let raw = tree.text(ch);
                if let Some(cr) = parse_cross_text(&raw, crosses.len()) {
                    crosses.push(cr);
                }
            }
            _ => {}
        }
    }

    Some(CovergroupDecl {
        name,
        coverpoints,
        crosses,
    })
}

fn parse_coverpoint_text(raw: &str, idx: usize) -> CoverpointDecl {
    let head = raw
        .split('{')
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_end_matches(';');
    let (cp_name, after_kw) = if let Some((lhs, rhs)) = head.split_once("coverpoint") {
        let lbl = lhs
            .trim()
            .trim_end_matches(':')
            .split_whitespace()
            .last()
            .unwrap_or("");
        let name = if lbl.is_empty() {
            format!("cp_{idx}")
        } else {
            lbl.to_string()
        };
        (name, rhs.trim())
    } else {
        (format!("cp_{idx}"), head)
    };

    let (target_expr, iff_guard) = if let Some((expr_part, iff_part)) = after_kw.split_once("iff") {
        let guard = iff_part
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim()
            .to_string();
        (expr_part.trim().to_string(), Some(guard))
    } else {
        (after_kw.trim().to_string(), None)
    };

    let mut bins = Vec::new();
    if let (Some(lb), Some(rb)) = (raw.find('{'), raw.rfind('}')) {
        let body = &raw[lb + 1..rb];
        for stmt in body.split(';') {
            let s = stmt.trim();
            if s.is_empty() {
                continue;
            }
            if let Some(bin) = parse_bin_stmt(s) {
                bins.push(bin);
            }
        }
    }

    CoverpointDecl {
        name: cp_name,
        target_expr,
        iff_guard,
        bins,
    }
}

fn parse_bin_stmt(stmt: &str) -> Option<CovBin> {
    let (lhs, rhs) = stmt.split_once('=')?;
    let lhs_words: Vec<&str> = lhs.split_whitespace().collect();
    let kind_kw = *lhs_words.first()?;
    let bname = lhs_words.last()?.trim_end_matches("[]").to_string();
    let set_body = rhs
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();

    if set_body.starts_with('[') && set_body.ends_with(']') {
        let inner = &set_body[1..set_body.len() - 1];
        if let Some((lo_s, hi_s)) = inner.split_once(':') {
            let low = crv_parser::parse_sv_literal(lo_s.trim()).ok()?.0 as u64;
            let high = crv_parser::parse_sv_literal(hi_s.trim()).ok()?.0 as u64;
            let kind = if kind_kw == "ignore_bins" {
                BinKind::IgnoreRange { low, high }
            } else {
                BinKind::Range { low, high }
            };
            return Some(CovBin { name: bname, kind });
        }
    }

    let mut vals = Vec::new();
    for part in set_body.split(',') {
        let p = part.trim().trim_start_matches('[').trim_end_matches(']');
        if let Ok((v, _, _)) = crv_parser::parse_sv_literal(p) {
            vals.push(v as u64);
        }
    }
    if vals.is_empty() {
        return None;
    }
    let kind = if kind_kw == "illegal_bins" {
        BinKind::IllegalValues(vals)
    } else {
        BinKind::Values(vals)
    };
    Some(CovBin { name: bname, kind })
}

fn strip_sv_comments(raw: &str) -> String {
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

fn parse_cross_text(raw: &str, idx: usize) -> Option<CrossDecl> {
    let cleaned = strip_sv_comments(raw);
    let head = cleaned
        .split('{')
        .next()
        .unwrap_or(&cleaned)
        .trim()
        .trim_end_matches(';');
    let (lhs, rhs) = head.split_once("cross")?;
    let lbl = lhs
        .trim()
        .trim_end_matches(':')
        .split_whitespace()
        .last()
        .unwrap_or("");
    let name = if lbl.is_empty() {
        format!("cross_{idx}")
    } else {
        lbl.to_string()
    };
    let cps: Vec<String> = rhs
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Some(CrossDecl {
        name,
        coverpoints: cps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_enum_dist_fork_clocking_and_sva() {
        let sv = r#"
typedef enum bit [1:0] { AXI_FIXED = 0, AXI_INCR = 1, AXI_WRAP = 2 } burst_e;

interface axi_if(input logic clk, input logic rst_n);
  logic req;
  logic ack;
  clocking cb @(posedge clk);
    default input #1step output #0;
    input ack;
    output req;
  endclocking
  ap_handshake: assert property (@(posedge clk) disable iff (!rst_n) req |-> ##[1:3] ack);
endinterface

class axi_item extends uvm_sequence_item;
  rand burst_e burst;
  randc bit [2:0] id;
  rand bit [7:0] len;
  constraint c_burst {
    burst dist { AXI_FIXED := 10, [AXI_INCR : AXI_WRAP] :/ 90 };
  }
endclass

class axi_driver extends uvm_driver #(axi_item);
  uvm_analysis_port #(axi_item) ap;
  virtual task run_phase(uvm_phase phase);
    fork
      begin
        @(cb);
      end
      begin
        #100ns;
      end
    join_any
    disable fork;
  endtask
endclass
"#;
        let (m, metrics) = SvFrontend::new().lower_str("axi.sv", sv).expect("lower");
        assert_eq!(m.enums.len(), 1);
        assert_eq!(m.clockings.len(), 1);
        assert_eq!(metrics.properties_lowered, 1);
        assert_eq!(m.properties[0].min_delay, 1);
        assert_eq!(m.properties[0].max_delay, 3);

        let item = m.find_class("axi_item").unwrap();
        assert!(
            item.fields
                .iter()
                .find(|f| f.name == "id")
                .unwrap()
                .is_randc
        );

        let drv = m.find_class("axi_driver").unwrap();
        assert_eq!(drv.tlm_ports.len(), 1);
        assert_eq!(drv.methods[0].body_ops.len(), 1);
        match &drv.methods[0].body_ops[0] {
            SchedOp::Fork {
                join,
                branches,
                cancel_on_exit,
            } => {
                assert_eq!(*join, JoinKind::JoinAny);
                assert_eq!(branches.len(), 2);
                assert!(*cancel_on_exit);
            }
            other => panic!("expected Fork, got {other:?}"),
        }
    }
}
