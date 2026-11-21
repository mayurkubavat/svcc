//! IEEE 1800-2023 SystemVerilog & IEEE 1800.2-2020 UVM front-end for `UVIR`.
//!
//! Powered by the pinned pure-Rust [`svlang`] + [`svlang_model`] SDK:
//! 1. Parses and semantically analyzes SV/UVM compilation units via [`svlang::Compilation`].
//! 2. Lifts OOP/UVM hierarchy (`UvmKind`, `PhaseKind`, `Property`, `Method`, `MacroUse`) via
//!    [`svlang_model::Design::from_sv`].
//! 3. Extracts UVM RAL register/field specifications via [`svlang_model::ral::extract_ral_blocks`].
//! 4. Walks the lossless [`svlang::syntax::Tree`] CST (`ConstraintDecl`, `ConstraintItem`,
//!    `CovergroupDecl`, `Coverpoint`, `Cross`) to populate `crv` and `cov` dialects.

pub mod crv_parser;

use std::collections::HashMap;
use std::time::Instant;
use svlang::{Compilation, NodeId, Severity, SyntaxKind, Tree};
use svlang_model::ral::extract_ral_blocks;
use svlang_model::{Design as SvModelDesign, PhaseKind as SvPhaseKind, UvmKind};
use uvir_ir::{
    BinKind, ClassDecl, ConstraintBlock, CovBin, CovergroupDecl, CoverpointDecl, CrossDecl,
    CrvArena, FieldDecl, FieldOps, Frontend, FrontendMetrics, MethodDecl, OwnershipMode, PhaseKind,
    Provenance, RalFieldMeta, ScalarKind, SourceLang, UvirError, UvirModule, UvirType, UvmRole,
};

/// IEEE 1800-2023 SystemVerilog / UVM front-end.
#[derive(Clone, Debug, Default)]
pub struct SvFrontend {
    /// Fail when `svlang` reports syntax errors.
    pub fail_on_syntax_error: bool,
}

impl SvFrontend {
    pub fn new() -> Self {
        Self {
            fail_on_syntax_error: true,
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
    ) -> Result<(UvirModule, FrontendMetrics), UvirError> {
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
                        }
                        _ => false,
                    }
                })
                .collect();
            if !syntax_errs.is_empty() {
                let first = syntax_errs[0];
                return Err(UvirError::new(
                    "uvir-front-sv/parse",
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
        for blk in &ral_blocks {
            for reg in &blk.registers {
                ral_regs_by_class.insert(reg.class_name.clone(), reg.clone());
            }
        }

        let mut module = UvirModule::new(filename);
        let mut total_constraints = 0usize;
        let mut total_covergroups = 0usize;

        // Index CST ClassDecl nodes by name, and collect standalone CovergroupDecl nodes outside classes
        let mut cst_classes: HashMap<String, (&Tree, NodeId)> = HashMap::new();
        for tree in trees {
            collect_top_cst_items(
                tree,
                tree.root,
                &mut cst_classes,
                &mut module.standalone_covergroups,
                &mut total_covergroups,
            );
        }

        for cls in sv_design.classes() {
            if cls.is_library {
                continue;
            }
            let cst_entry = cst_classes.get(&cls.name).copied();
            let effective_base = if !cls.base_name.is_empty() {
                cls.base_name.clone()
            } else if let Some((tree, cls_node)) = cst_entry {
                extract_cst_extends(tree, cls_node).unwrap_or_default()
            } else {
                String::new()
            };

            let role = map_uvm_role(cls.uvm, &effective_base);
            let ral_reg = ral_regs_by_class.get(&cls.name);

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
            if let Some(reg_spec) = ral_reg {
                for rf in &reg_spec.fields {
                    fields.push(FieldDecl {
                        name: rf.name.clone(),
                        ty: UvirType::bitvec(rf.width, false),
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
            } else {
                for prop in &cls.properties {
                    if prop.ty.trim() == "uvm_reg_field"
                        || covergroups.iter().any(|cg| cg.name == prop.name)
                    {
                        continue;
                    }
                    let ty = parse_sv_property_type(&prop.ty);
                    let is_randc = source_property_is_randc(cst_entry, &prop.name);
                    fields.push(FieldDecl {
                        name: prop.name.clone(),
                        ty,
                        is_rand: prop.is_rand,
                        is_randc,
                        ops: FieldOps::default(),
                        ral_meta: None,
                    });
                }
            }

            apply_uvm_field_macros_from_source(source, &cls.name, &mut fields);

            let field_widths: Vec<(String, u32, bool)> = fields
                .iter()
                .map(|f| (f.name.clone(), f.ty.width, f.ty.signed))
                .collect();

            let mut constraints = Vec::new();
            if let Some((tree, cls_node)) = cst_entry {
                constraints = extract_class_constraints(
                    tree,
                    cls_node,
                    &mut module.crv_arena,
                    &field_widths,
                )?;
                total_constraints += constraints.len();
            }

            let methods = cls
                .methods
                .iter()
                .filter(|m| !m.is_builtin && m.name != "new")
                .map(|m| MethodDecl {
                    name: m.name.clone(),
                    phase: map_phase_kind(&m.name, m.phase),
                    is_task: m.is_task,
                    is_virtual: m.is_virtual,
                    body_ops: Vec::new(),
                })
                .collect();

            let macros = cls.macros.iter().map(|m| m.name.clone()).collect();
            module.classes.push(ClassDecl {
                name: cls.name.clone(),
                package_or_namespace: cls.scope.clone(),
                role,
                base_class: (!effective_base.is_empty()).then_some(effective_base),
                ownership: OwnershipMode::SvGcHandle,
                sv_signedness: true,
                needs_value_copy_ctor: false,
                synthesize_field_methods: role == UvmRole::SequenceItem || role == UvmRole::Object,
                reg_width_bits: ral_reg
                    .map(|r| r.n_bits)
                    .or_else(|| (role == UvmRole::Reg).then_some(32)),
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
        };
        Ok((module, metrics))
    }
}

fn collect_top_cst_items<'a>(
    tree: &'a Tree,
    node: NodeId,
    cst_classes: &mut HashMap<String, (&'a Tree, NodeId)>,
    standalone_cgs: &mut Vec<CovergroupDecl>,
    total_cgs: &mut usize,
) {
    match tree.kind(node) {
        SyntaxKind::ClassDecl => {
            if let Some(nm) = tree.child(node, SyntaxKind::Name) {
                cst_classes.insert(tree.bare_text(nm).trim().to_string(), (tree, node));
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
                collect_top_cst_items(tree, ch, cst_classes, standalone_cgs, total_cgs);
            }
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
        "uvm_reg" => UvmRole::Reg,
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

/// Resolves SV property type text (`bit [31:0]`, `logic signed [15:0]`, `int unsigned`, `byte`, etc.)
/// into a canonical [`UvirType`].
pub fn parse_sv_property_type(raw_ty: &str) -> UvirType {
    let t = raw_ty.trim();
    if t == "string" {
        return UvirType {
            kind: ScalarKind::String,
            width: 0,
            signed: false,
            is_four_state: false,
        };
    }
    if t == "real" || t == "shortreal" {
        return UvirType {
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
            return UvirType {
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
        UvirType::logicvec(width, signed)
    } else {
        UvirType::bitvec(width, signed)
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

fn extract_cst_extends(tree: &Tree, cls_node: NodeId) -> Option<String> {
    let raw = tree.text(cls_node);
    let header = raw.split(';').next()?;
    let (_, after) = header.split_once("extends")?;
    let base = after
        .trim()
        .split(['#', '(', ' ', '\n', '\t'])
        .next()?
        .trim()
        .to_string();
    (!base.is_empty()).then_some(base)
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
    field_widths: &[(String, u32, bool)],
) -> Result<Vec<ConstraintBlock>, UvirError> {
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
            collect_constraint_clauses(tree, cblock, arena, field_widths, &mut clauses)?;
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
    field_widths: &[(String, u32, bool)],
    out: &mut Vec<uvir_ir::ConstraintClause>,
) -> Result<(), UvirError> {
    for item in tree.children(block_node) {
        if tree.kind(item) != SyntaxKind::ConstraintItem {
            continue;
        }
        if let Some(nested) = tree.child(item, SyntaxKind::ConstraintBlock) {
            let item_txt = tree.text(item);
            let trimmed = item_txt.trim();
            // If it's a pure `{ ... }` grouping without `if` or `->`, recurse; otherwise parse whole item.
            if trimmed.starts_with('{') {
                collect_constraint_clauses(tree, nested, arena, field_widths, out)?;
                continue;
            }
        }
        let raw = tree.text(item);
        if let Some(clause) = crv_parser::lower_sv_constraint_item(&raw, arena, field_widths)? {
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

fn parse_cross_text(raw: &str, idx: usize) -> Option<CrossDecl> {
    let head = raw
        .split('{')
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_end_matches(';');
    let (lhs, rhs) = head.split_once("cross")?;
    let lbl = lhs.trim().trim_end_matches(':').trim();
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
    fn lowers_sv_uvm_sequence_item_constraints_and_covergroup() {
        let sv = r#"
class apb_item extends uvm_sequence_item;
  rand bit [31:0] addr;
  rand bit [7:0]  len;
  rand bit        write;
  bit [31:0]      tag;

  `uvm_object_utils_begin(apb_item)
    `uvm_field_int(addr, UVM_ALL_ON)
    `uvm_field_int(tag, UVM_NOCOMPARE)
  `uvm_object_utils_end

  constraint c_align {
    (addr & 32'h3) == 32'h0;
    addr inside {[32'h1000 : 32'h1FFF]};
    write -> (len <= 8'd16);
    soft len == 8'd4;
  }

  covergroup cg_apb;
    cp_addr: coverpoint addr {
      bins low_page = {[32'h1000:32'h17FF]};
      bins high_page = {[32'h1800:32'h1FFF]};
    }
    cp_write: coverpoint write {
      bins rd = {0};
      bins wr = {1};
    }
    cx_rw_addr: cross cp_addr, cp_write;
  endgroup
endclass
"#;
        let fe = SvFrontend::new();
        let (m, metrics) = fe.lower_str("apb_item.sv", sv).expect("lower sv");
        assert_eq!(metrics.classes_lowered, 1);
        assert_eq!(metrics.constraints_lowered, 1);
        assert_eq!(metrics.covergroups_lowered, 1);

        let cls = &m.classes[0];
        assert_eq!(cls.role, UvmRole::SequenceItem);
        assert_eq!(cls.fields.len(), 4);
        assert!(
            !cls.fields
                .iter()
                .find(|f| f.name == "tag")
                .unwrap()
                .ops
                .compare
        );
        assert_eq!(cls.constraints[0].clauses.len(), 4);
        assert!(cls.constraints[0].clauses[3].is_soft);
        assert_eq!(cls.covergroups[0].coverpoints.len(), 2);
        assert_eq!(cls.covergroups[0].crosses.len(), 1);
    }
}
