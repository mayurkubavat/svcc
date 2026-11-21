//! Middle-end normalization and SV <-> C++17 capability legalization passes for `UVIR`.

use std::time::Instant;
use uvir_ir::{
    CrvArena, CrvBinaryOp, CrvExpr, DistItem, ExprId, InsideElem, MemoryModel, OwnershipMode,
    PassStats, TargetCapabilities, TemporalModel, TransformPass, UvirError, UvirModule, UvmRole,
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

        if !self.caps.native_inside_tolerance {
            for cls in &mut module.classes {
                for cb in &mut cls.constraints {
                    for clause in &mut cb.clauses {
                        let new_expr = expand_tolerance_in_expr(
                            clause.expr,
                            &mut module.crv_arena,
                            &mut modified,
                        );
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
            let delta = if let (
                CrvExpr::Const {
                    value: c_v,
                    width: w,
                    signed: s,
                },
                CrvExpr::Const { value: p_v, .. },
            ) = (arena.get(center).clone(), arena.get(pct).clone())
            {
                arena.alloc(CrvExpr::Const {
                    value: (c_v * p_v) / 100,
                    width: w,
                    signed: s,
                })
            } else {
                arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Mul,
                    lhs: center,
                    rhs: pct,
                })
            };
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

fn expand_tolerance_in_expr(id: ExprId, arena: &mut CrvArena, modified: &mut usize) -> ExprId {
    let node = arena.get(id).clone();
    match node {
        CrvExpr::Inside { target, elems } => {
            let before = *modified;
            let new_elems: Vec<_> = elems
                .into_iter()
                .map(|el| expand_inside_elem(el, arena, modified))
                .collect();
            if *modified != before {
                arena.alloc(CrvExpr::Inside {
                    target,
                    elems: new_elems,
                })
            } else {
                id
            }
        }
        CrvExpr::Dist { target, items } => {
            let before = *modified;
            let new_items: Vec<_> = items
                .into_iter()
                .map(|it| DistItem {
                    elem: expand_inside_elem(it.elem, arena, modified),
                    kind: it.kind,
                    weight: it.weight,
                })
                .collect();
            if *modified != before {
                arena.alloc(CrvExpr::Dist {
                    target,
                    items: new_items,
                })
            } else {
                id
            }
        }
        CrvExpr::Binary { op, lhs, rhs } => {
            let nl = expand_tolerance_in_expr(lhs, arena, modified);
            let nr = expand_tolerance_in_expr(rhs, arena, modified);
            if nl != lhs || nr != rhs {
                arena.alloc(CrvExpr::Binary {
                    op,
                    lhs: nl,
                    rhs: nr,
                })
            } else {
                id
            }
        }
        CrvExpr::Unary { op, arg } => {
            let na = expand_tolerance_in_expr(arg, arena, modified);
            if na != arg {
                arena.alloc(CrvExpr::Unary { op, arg: na })
            } else {
                id
            }
        }
        _ => id,
    }
}
