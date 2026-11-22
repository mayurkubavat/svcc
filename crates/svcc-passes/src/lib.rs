//! Middle-end normalization and SV <-> C++17 capability legalization passes for `UVIR`.

use std::time::Instant;
use svcc_ir::{
    CrvArena, CrvBinaryOp, CrvExpr, DistItem, DistWeightKind, ExprId, InsideElem, MemoryModel,
    OwnershipMode, PassStats, TargetCapabilities, TemporalModel, TransformPass, UvirError,
    UvirModule, UvmRole,
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

    pub fn run_all(&self, module: &mut UvirModule) -> Result<Vec<PassStats>, UvirError> {
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

    fn run(&self, module: &mut UvirModule) -> Result<PassStats, UvirError> {
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

    fn run(&self, module: &mut UvirModule) -> Result<PassStats, UvirError> {
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

    fn run(&self, module: &mut UvirModule) -> Result<PassStats, UvirError> {
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

    fn run(&self, module: &mut UvirModule) -> Result<PassStats, UvirError> {
        let t0 = Instant::now();
        let mut modified = 0usize;

        if !self.caps.native_inside_tolerance || !self.caps.exact_dist_split_weights {
            for cls in &mut module.classes {
                for cb in &mut cls.constraints {
                    for clause in &mut cb.clauses {
                        let new_expr = legalize_crv_expr(
                            clause.expr,
                            &self.caps,
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

fn elem_const_span(el: &InsideElem, arena: &CrvArena) -> Option<u128> {
    match el {
        InsideElem::Single(_) => Some(1),
        InsideElem::Range { low, high } => {
            if let (CrvExpr::Const { value: lo, .. }, CrvExpr::Const { value: hi, .. }) =
                (arena.get(*low), arena.get(*high))
            {
                if hi >= lo {
                    (*hi - *lo).checked_add(1)
                } else {
                    Some(0)
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Prediction `P2`: Scales `dist` weights by `L = lcm_i(S_i)` across non-divisible `SplitRange`
/// (`:/`) items so CRAVE's integer `W / S` range division incurs zero truncation skew.
fn legalize_dist_lcm(
    items: &mut [DistItem],
    arena: &CrvArena,
    modified: &mut usize,
) -> Result<(), UvirError> {
    let mut spans = Vec::with_capacity(items.len());
    for it in items.iter() {
        let Some(s) = elem_const_span(&it.elem, arena) else {
            return Ok(());
        };
        spans.push(s);
    }

    let needs_lcm = items.iter().zip(spans.iter()).any(|(it, &s)| {
        it.kind == DistWeightKind::SplitRange
            && it.weight > 0
            && s >= 1
            && (it.weight as u128) % s != 0
    });
    if !needs_lcm {
        return Ok(());
    }

    let mut lcm = 1u128;
    for (it, &s) in items.iter().zip(spans.iter()) {
        if it.kind == DistWeightKind::SplitRange && it.weight > 0 && s >= 1 {
            lcm = lcm_u128(lcm, s).ok_or_else(|| {
                UvirError::new("svcc-passes/dist-lcm", "dist range LCM overflowed u128")
            })?;
        }
    }
    if lcm <= 1 {
        return Ok(());
    }

    let mut scaled: Vec<u128> = items.iter().map(|it| (it.weight as u128) * lcm).collect();
    // Admissible GCD reduction over per-element weights `e_i` (preserves `S_i | W'_i`)
    let mut g = 0u128;
    for ((it, &w_sc), &s) in items.iter().zip(scaled.iter()).zip(spans.iter()) {
        if w_sc > 0 && s >= 1 {
            let elem_w = if it.kind == DistWeightKind::SplitRange {
                w_sc / s
            } else {
                w_sc
            };
            g = if g == 0 { elem_w } else { gcd_u128(g, elem_w) };
        }
    }
    if g > 1 {
        for w_sc in &mut scaled {
            *w_sc /= g;
        }
    }

    let mut checked = Vec::with_capacity(items.len());
    for w_sc in scaled {
        let Ok(w32) = u32::try_from(w_sc) else {
            // Wide 32/64-bit split ranges (e.g. `[1:32'hffffffff] :/ 1000`) cannot fit `W * S`
            // in `u32`; CRAVE's hierarchical `weighted_range` sampler already preserves `W` directly.
            return Ok(());
        };
        checked.push(w32);
    }
    for (it, w32) in items.iter_mut().zip(checked) {
        if it.weight != w32 {
            it.weight = w32;
            *modified += 1;
        }
    }
    Ok(())
}

fn legalize_crv_expr(
    id: ExprId,
    caps: &TargetCapabilities,
    arena: &mut CrvArena,
    modified: &mut usize,
) -> Result<ExprId, UvirError> {
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
                legalize_dist_lcm(&mut new_items, arena, modified)?;
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
            let nl = legalize_crv_expr(lhs, caps, arena, modified)?;
            let nr = legalize_crv_expr(rhs, caps, arena, modified)?;
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
            let na = legalize_crv_expr(arg, caps, arena, modified)?;
            if na != arg {
                Ok(arena.alloc(CrvExpr::Unary { op, arg: na }))
            } else {
                Ok(id)
            }
        }
        _ => Ok(id),
    }
}
