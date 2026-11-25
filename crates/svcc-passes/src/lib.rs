//! Middle-end normalization and SV <-> C++17 capability legalization passes for `svcc-ir`.

pub mod upf_lower;

use std::collections::HashMap;
use std::time::Instant;
use svcc_ir::{
    ClassDecl, CrvArena, CrvBinaryOp, CrvExpr, DistItem, DistWeightKind, ExprId, InsideElem,
    IrModule, MemoryModel, OwnershipMode, PassStats, ScalarKind, SvccError, TargetCapabilities,
    TemporalModel, TransformPass, UvmRole,
};

#[derive(Default)]
pub struct PassPipeline {
    passes: Vec<Box<dyn TransformPass>>,
}

impl PassPipeline {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn for_target(caps: TargetCapabilities) -> Self {
        let mut p = Self::new();
        p.add_pass(Box::new(WidthSignednessPass));
        p.add_pass(Box::new(FieldOpSynthesisPass));
        p.add_pass(Box::new(OwnershipPass {
            memory_model: caps.memory_model,
        }));
        p.add_pass(Box::new(TargetLegalizerPass { caps }));
        p
    }

    pub fn add_pass(&mut self, pass: Box<dyn TransformPass>) {
        self.passes.push(pass);
    }

    pub fn run_all(&self, module: &mut IrModule) -> Result<Vec<PassStats>, SvccError> {
        let mut stats = Vec::with_capacity(self.passes.len());
        for pass in &self.passes {
            stats.push(pass.run(module)?);
        }
        Ok(stats)
    }
}

pub struct OwnershipPass {
    pub memory_model: MemoryModel,
}

impl TransformPass for OwnershipPass {
    fn name(&self) -> &'static str {
        "ownership-lowering"
    }

    fn run(&self, module: &mut IrModule) -> Result<PassStats, SvccError> {
        let t0 = Instant::now();
        let mut modified = 0usize;

        for cls in &mut module.classes {
            let (new_mode, needs_copy) = match self.memory_model {
                MemoryModel::GarbageCollected => (OwnershipMode::SvGcHandle, false),
                MemoryModel::ValueAndRaii => {
                    if cls.role.is_component() || cls.role == UvmRole::RegBlock {
                        (OwnershipMode::ScFactoryRawPtr, false)
                    } else {
                        let has_rand = cls.fields.iter().any(|f| f.is_rand);
                        (
                            OwnershipMode::ScValueCopy,
                            has_rand && cls.role != UvmRole::Reg,
                        )
                    }
                }
            };
            if cls.ownership != new_mode || cls.needs_value_copy_ctor != needs_copy {
                cls.ownership = new_mode;
                cls.needs_value_copy_ctor = needs_copy;
                modified += 1;
            }
        }

        Ok(PassStats {
            pass_name: self.name().into(),
            elapsed_us: t0.elapsed().as_micros() as u64,
            items_modified: modified,
        })
    }
}

pub struct WidthSignednessPass;

impl TransformPass for WidthSignednessPass {
    fn name(&self) -> &'static str {
        "width-signedness-propagation"
    }

    fn run(&self, module: &mut IrModule) -> Result<PassStats, SvccError> {
        let t0 = Instant::now();
        let mut modified = 0usize;

        for cls in &mut module.classes {
            if !cls.sv_signedness && cls.fields.iter().any(|f| f.ty.signed) {
                cls.sv_signedness = true;
                modified += 1;
            }
        }

        Ok(PassStats {
            pass_name: self.name().into(),
            elapsed_us: t0.elapsed().as_micros() as u64,
            items_modified: modified,
        })
    }
}

pub struct FieldOpSynthesisPass;

impl TransformPass for FieldOpSynthesisPass {
    fn name(&self) -> &'static str {
        "field-op-synthesis"
    }

    fn run(&self, module: &mut IrModule) -> Result<PassStats, SvccError> {
        let t0 = Instant::now();
        let mut modified = 0usize;
        for cls in &mut module.classes {
            let should_synth = matches!(cls.role, UvmRole::SequenceItem | UvmRole::Object);
            if cls.synthesize_field_methods != should_synth {
                cls.synthesize_field_methods = should_synth;
                modified += 1;
            }
        }
        Ok(PassStats {
            pass_name: self.name().into(),
            elapsed_us: t0.elapsed().as_micros() as u64,
            items_modified: modified,
        })
    }
}

/// Capability-driven legalizer lowering SV constructs lacking direct C++17 equivalents
/// (and vice versa).
pub struct TargetLegalizerPass {
    pub caps: TargetCapabilities,
}

impl TransformPass for TargetLegalizerPass {
    fn name(&self) -> &'static str {
        "target-capability-legalizer"
    }

    fn run(&self, module: &mut IrModule) -> Result<PassStats, SvccError> {
        let t0 = Instant::now();
        let mut modified = 0usize;

        if !self.caps.native_inside_tolerance || !self.caps.exact_dist_split_weights {
            for cls in &mut module.classes {
                let hard_bounds = if !self.caps.exact_dist_split_weights {
                    collect_hard_var_bounds(cls, &module.crv_arena)
                } else {
                    HashMap::new()
                };
                for cb in &mut cls.constraints {
                    for clause in &mut cb.clauses {
                        let new_expr = legalize_crv_expr(
                            clause.expr,
                            &self.caps,
                            &hard_bounds,
                            &mut module.crv_arena,
                            &mut modified,
                        )?;
                        clause.expr = new_expr;
                    }
                }
            }
        }

        if self.caps.temporal_model == TemporalModel::LoweredAutomaton
            && !module.properties.is_empty()
        {
            modified += module.properties.len();
        }

        Ok(PassStats {
            pass_name: self.name().into(),
            elapsed_us: t0.elapsed().as_micros() as u64,
            items_modified: modified,
        })
    }
}

fn expand_inside_elem(el: InsideElem, arena: &mut CrvArena, modified: &mut usize) -> InsideElem {
    match el {
        InsideElem::ToleranceAbs { center, delta } => {
            *modified += 1;
            if let (
                CrvExpr::Const {
                    value: c_v,
                    width: w,
                    signed: s,
                },
                CrvExpr::Const { value: d_v, .. },
            ) = (arena.get(center).clone(), arena.get(delta).clone())
            {
                let lo_v = c_v.saturating_sub(d_v);
                let hi_v = c_v.saturating_add(d_v);
                let low = arena.alloc(CrvExpr::Const {
                    value: lo_v,
                    width: w,
                    signed: s,
                });
                let high = arena.alloc(CrvExpr::Const {
                    value: hi_v,
                    width: w,
                    signed: s,
                });
                return InsideElem::Range { low, high };
            }
            let low = arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Sub,
                lhs: center,
                rhs: delta,
            });
            let high = arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Add,
                lhs: center,
                rhs: delta,
            });
            InsideElem::Range { low, high }
        }
        InsideElem::TolerancePct { center, pct } => {
            *modified += 1;
            if let (
                CrvExpr::Const {
                    value: c_v,
                    width: w,
                    signed: s,
                },
                CrvExpr::Const { value: p_v, .. },
            ) = (arena.get(center).clone(), arena.get(pct).clone())
            {
                let d_v = (c_v * p_v) / 100;
                let low = arena.alloc(CrvExpr::Const {
                    value: c_v.saturating_sub(d_v),
                    width: w,
                    signed: s,
                });
                let high = arena.alloc(CrvExpr::Const {
                    value: c_v.saturating_add(d_v),
                    width: w,
                    signed: s,
                });
                return InsideElem::Range { low, high };
            }
            let delta = arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Mul,
                lhs: center,
                rhs: pct,
            });
            let low = arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Sub,
                lhs: center,
                rhs: delta,
            });
            let high = arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Add,
                lhs: center,
                rhs: delta,
            });
            InsideElem::Range { low, high }
        }
        other => other,
    }
}

fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

fn lcm_u128(a: u128, b: u128) -> Option<u128> {
    if a == 0 || b == 0 {
        return Some(0);
    }
    (a / gcd_u128(a, b)).checked_mul(b)
}

fn collect_hard_var_bounds(cls: &ClassDecl, arena: &CrvArena) -> HashMap<String, (u128, u128)> {
    let mut bounds = HashMap::new();
    for f in &cls.fields {
        if !f.ty.signed && matches!(f.ty.kind, ScalarKind::BitVector | ScalarKind::LogicVector) {
            let max_v = if f.ty.width >= 128 {
                u128::MAX
            } else {
                (1u128 << f.ty.width) - 1
            };
            bounds.insert(f.name.clone(), (0u128, max_v));
        }
    }
    for cb in &cls.constraints {
        for clause in &cb.clauses {
            if !clause.is_soft {
                refine_bounds_from_expr(clause.expr, arena, &mut bounds);
            }
        }
    }
    bounds
}

fn apply_var_cmp_const(op: CrvBinaryOp, c: u128, lo: &mut u128, hi: &mut u128) {
    match op {
        CrvBinaryOp::Ge => *lo = (*lo).max(c),
        CrvBinaryOp::Gt => *lo = (*lo).max(c.saturating_add(1)),
        CrvBinaryOp::Le => *hi = (*hi).min(c),
        CrvBinaryOp::Lt => {
            if c == 0 {
                *lo = 1;
                *hi = 0;
            } else {
                *hi = (*hi).min(c - 1);
            }
        }
        CrvBinaryOp::Eq => {
            *lo = (*lo).max(c);
            *hi = (*hi).min(c);
        }
        _ => {}
    }
}

fn refine_bounds_from_expr(
    id: ExprId,
    arena: &CrvArena,
    bounds: &mut HashMap<String, (u128, u128)>,
) {
    match arena.get(id) {
        CrvExpr::Binary {
            op: CrvBinaryOp::LogAnd,
            lhs,
            rhs,
        } => {
            refine_bounds_from_expr(*lhs, arena, bounds);
            refine_bounds_from_expr(*rhs, arena, bounds);
        }
        CrvExpr::Binary { op, lhs, rhs } => match (arena.get(*lhs), arena.get(*rhs)) {
            (
                CrvExpr::Var(v),
                CrvExpr::Const {
                    value: c,
                    signed: false,
                    ..
                },
            ) => {
                if let Some((lo, hi)) = bounds.get_mut(v) {
                    apply_var_cmp_const(*op, *c, lo, hi);
                }
            }
            (
                CrvExpr::Const {
                    value: c,
                    signed: false,
                    ..
                },
                CrvExpr::Var(v),
            ) => {
                let flipped = match op {
                    CrvBinaryOp::Le => Some(CrvBinaryOp::Ge),
                    CrvBinaryOp::Lt => Some(CrvBinaryOp::Gt),
                    CrvBinaryOp::Ge => Some(CrvBinaryOp::Le),
                    CrvBinaryOp::Gt => Some(CrvBinaryOp::Lt),
                    CrvBinaryOp::Eq => Some(CrvBinaryOp::Eq),
                    _ => None,
                };
                if let (Some(fop), Some((lo, hi))) = (flipped, bounds.get_mut(v)) {
                    apply_var_cmp_const(fop, *c, lo, hi);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

struct DistConstBounds {
    lo: u128,
    hi: u128,
    width: u32,
}

fn elem_unsigned_const_bounds(el: &InsideElem, arena: &CrvArena) -> Option<DistConstBounds> {
    match el {
        InsideElem::Single(id) => {
            if let CrvExpr::Const {
                value,
                width,
                signed: false,
            } = arena.get(*id)
            {
                Some(DistConstBounds {
                    lo: *value,
                    hi: *value,
                    width: *width,
                })
            } else {
                None
            }
        }
        InsideElem::Range { low, high } => {
            if let (
                CrvExpr::Const {
                    value: lo,
                    width: w_lo,
                    signed: false,
                },
                CrvExpr::Const {
                    value: hi,
                    width: w_hi,
                    signed: false,
                },
            ) = (arena.get(*low), arena.get(*high))
            {
                Some(DistConstBounds {
                    lo: *lo,
                    hi: *hi,
                    width: (*w_lo).max(*w_hi),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Prediction `P2`: Clips `dist` intervals against hard single-variable bounds (eliminating
/// CRAVE `weighted_range` boundary mass clamping) and scales weights by `L = lcm_i(den_i)` across
/// `SplitRange` (`:/`) items so integer `W / S` division incurs zero truncation skew.
fn legalize_dist_sliced_and_lcm(
    target_var: Option<&str>,
    hard_bounds: &HashMap<String, (u128, u128)>,
    items: &mut Vec<DistItem>,
    arena: &mut CrvArena,
    modified: &mut usize,
) -> Result<(), SvccError> {
    if items.is_empty() || items.iter().any(|it| it.weight == 0) {
        return Ok(());
    }

    struct Survivor {
        orig_idx: usize,
        orig_lo: u128,
        orig_hi: u128,
        clip_lo: u128,
        clip_hi: u128,
        clip_span: u128,
        width: u32,
        kind: DistWeightKind,
        orig_weight: u32,
        num: u128,
        den: u128,
    }

    let var_bound = target_var.and_then(|v| hard_bounds.get(v).copied());
    let mut survivors = Vec::with_capacity(items.len());
    let mut was_clipped = false;

    for (idx, it) in items.iter().enumerate() {
        let Some(b) = elem_unsigned_const_bounds(&it.elem, arena) else {
            return Ok(());
        };
        if b.hi < b.lo {
            return Ok(());
        }
        let Some(orig_span) = (b.hi - b.lo).checked_add(1) else {
            return Ok(());
        };
        let (clip_lo, clip_hi) = if let Some((blo, bhi)) = var_bound {
            (b.lo.max(blo), b.hi.min(bhi))
        } else {
            (b.lo, b.hi)
        };
        if clip_lo > clip_hi {
            was_clipped = true;
            continue;
        }
        if clip_lo != b.lo || clip_hi != b.hi {
            was_clipped = true;
        }
        let Some(clip_span) = (clip_hi - clip_lo).checked_add(1) else {
            return Ok(());
        };
        let (num, den) = if it.kind == DistWeightKind::SplitRange {
            let g = gcd_u128(it.weight as u128, orig_span);
            ((it.weight as u128) / g, orig_span / g)
        } else {
            (it.weight as u128, 1u128)
        };
        survivors.push(Survivor {
            orig_idx: idx,
            orig_lo: b.lo,
            orig_hi: b.hi,
            clip_lo,
            clip_hi,
            clip_span,
            width: b.width,
            kind: it.kind,
            orig_weight: it.weight,
            num,
            den,
        });
    }

    if survivors.is_empty() {
        return Ok(());
    }

    let needs_lcm = survivors.iter().any(|s| {
        s.kind == DistWeightKind::SplitRange && (s.orig_weight as u128) % s.clip_span != 0
    });
    if !was_clipped && !needs_lcm {
        return Ok(());
    }

    let mut lcm_den = 1u128;
    for s in &survivors {
        lcm_den = lcm_u128(lcm_den, s.den).ok_or_else(|| {
            SvccError::new("svcc-passes/dist-lcm", "dist range LCM overflowed u128")
        })?;
    }

    let mut elem_weights = Vec::with_capacity(survivors.len());
    let mut g_all = 0u128;
    for s in &survivors {
        let Some(ew) = s.num.checked_mul(lcm_den / s.den) else {
            return Ok(());
        };
        g_all = if g_all == 0 { ew } else { gcd_u128(g_all, ew) };
        elem_weights.push(ew);
    }
    if g_all > 1 {
        for ew in &mut elem_weights {
            *ew /= g_all;
        }
    }

    let mut final_weights = Vec::with_capacity(survivors.len());
    for (s, &ew) in survivors.iter().zip(elem_weights.iter()) {
        let w_u128 = if s.kind == DistWeightKind::SplitRange && s.clip_span > 1 {
            let Some(prod) = ew.checked_mul(s.clip_span) else {
                return Ok(());
            };
            prod
        } else {
            ew
        };
        let Ok(w32) = u32::try_from(w_u128) else {
            // Wide 32/64-bit split ranges (e.g. `[1:32'hffffffff] :/ 1000`) cannot fit `W * S`
            // in `u32`; CRAVE's hierarchical `weighted_range` sampler preserves `W` directly.
            return Ok(());
        };
        final_weights.push(w32);
    }

    let mut rebuilt = Vec::with_capacity(survivors.len());
    for (s, w32) in survivors.into_iter().zip(final_weights) {
        let orig_elem = items[s.orig_idx].elem.clone();
        let (new_elem, new_kind) =
            if s.clip_lo == s.clip_hi && (s.clip_lo != s.orig_lo || s.clip_hi != s.orig_hi) {
                let c = arena.alloc(CrvExpr::Const {
                    value: s.clip_lo,
                    width: s.width,
                    signed: false,
                });
                *modified += 1;
                (InsideElem::Single(c), DistWeightKind::PerValue)
            } else if s.clip_lo != s.orig_lo || s.clip_hi != s.orig_hi {
                let low = arena.alloc(CrvExpr::Const {
                    value: s.clip_lo,
                    width: s.width,
                    signed: false,
                });
                let high = arena.alloc(CrvExpr::Const {
                    value: s.clip_hi,
                    width: s.width,
                    signed: false,
                });
                *modified += 1;
                (InsideElem::Range { low, high }, s.kind)
            } else {
                (orig_elem, s.kind)
            };
        if w32 != s.orig_weight || new_kind != s.kind {
            *modified += 1;
        }
        rebuilt.push(DistItem {
            elem: new_elem,
            kind: new_kind,
            weight: w32,
        });
    }

    if rebuilt.len() != items.len() {
        *modified += 1;
    }
    *items = rebuilt;
    Ok(())
}

fn legalize_crv_expr(
    id: ExprId,
    caps: &TargetCapabilities,
    hard_bounds: &HashMap<String, (u128, u128)>,
    arena: &mut CrvArena,
    modified: &mut usize,
) -> Result<ExprId, SvccError> {
    let node = arena.get(id).clone();
    match node {
        CrvExpr::Inside { target, elems } => {
            if caps.native_inside_tolerance {
                return Ok(id);
            }
            let before = *modified;
            let new_elems: Vec<_> = elems
                .into_iter()
                .map(|el| expand_inside_elem(el, arena, modified))
                .collect();
            if *modified != before {
                Ok(arena.alloc(CrvExpr::Inside {
                    target,
                    elems: new_elems,
                }))
            } else {
                Ok(id)
            }
        }
        CrvExpr::Dist { target, items } => {
            let before = *modified;
            let mut new_items: Vec<_> = items
                .into_iter()
                .map(|it| DistItem {
                    elem: if caps.native_inside_tolerance {
                        it.elem
                    } else {
                        expand_inside_elem(it.elem, arena, modified)
                    },
                    kind: it.kind,
                    weight: it.weight,
                })
                .collect();
            if !caps.exact_dist_split_weights {
                let target_var = match arena.get(target) {
                    CrvExpr::Var(v) => Some(v.clone()),
                    _ => None,
                };
                legalize_dist_sliced_and_lcm(
                    target_var.as_deref(),
                    hard_bounds,
                    &mut new_items,
                    arena,
                    modified,
                )?;
            }
            if *modified != before {
                Ok(arena.alloc(CrvExpr::Dist {
                    target,
                    items: new_items,
                }))
            } else {
                Ok(id)
            }
        }
        CrvExpr::Binary { op, lhs, rhs } => {
            let nl = legalize_crv_expr(lhs, caps, hard_bounds, arena, modified)?;
            let nr = legalize_crv_expr(rhs, caps, hard_bounds, arena, modified)?;
            if nl != lhs || nr != rhs {
                Ok(arena.alloc(CrvExpr::Binary {
                    op,
                    lhs: nl,
                    rhs: nr,
                }))
            } else {
                Ok(id)
            }
        }
        CrvExpr::Unary { op, arg } => {
            let na = legalize_crv_expr(arg, caps, hard_bounds, arena, modified)?;
            if na != arg {
                Ok(arena.alloc(CrvExpr::Unary { op, arg: na }))
            } else {
                Ok(id)
            }
        }
        _ => Ok(id),
    }
}
