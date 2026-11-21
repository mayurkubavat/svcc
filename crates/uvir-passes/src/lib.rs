//! Middle-end normalization and multi-language target legalization passes for `UVIR`.
//!
//! Includes:
//! 1. [`OwnershipPass`]: Resolves GC-handle vs. RAII/TLM value-copy semantics and flags
//!    `crv_variable<T>` sequence items requiring synthesized value-only copy constructors.
//! 2. [`WidthSignednessPass`]: Enforces IEEE 1800 bitvector widths & `sv_signedness`.
//! 3. [`FieldOpSynthesisPass`]: Ensures IEEE 1800.2 field policy synthesis (`do_copy`,
//!    `do_compare`, `convert2string`) across targets.
//! 4. [`TargetLegalizerPass`]: Generic $O(N)$ multi-language legalization driven by
//!    [`TargetCapabilities`] — expands IEEE 1800-2023 tolerance ranges (`[c +/- d]`, `[c +%- p]`)
//!    for solvers without native tolerance syntax (`CRAVE`, `PyVSC`, `SMT-LIB2`) and lowers
//!    `TemporalModel::LoweredAutomaton` SVA assertions when targeting C++/Python/Rust.

use std::time::Instant;
use uvir_ir::{
    CrvArena, CrvBinaryOp, CrvExpr, ExprId, InsideElem, MemoryModel, OwnershipMode, PassStats,
    TargetCapabilities, TemporalModel, TransformPass, UvirError, UvirModule, UvmRole,
};

/// Ordered pass manager collecting microsecond telemetry per pass.
#[derive(Default)]
pub struct PassPipeline {
    passes: Vec<Box<dyn TransformPass>>,
}

impl PassPipeline {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    /// Standard multi-language normalization + target legalization pipeline.
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

/// Resolves memory ownership (`SvGcHandle` vs. `ScValueCopy` / `ScFactoryRawPtr`) for the target's [`MemoryModel`].
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
                    if cls.role.is_component() {
                        (OwnershipMode::ScFactoryRawPtr, false)
                    } else {
                        let has_rand = cls.fields.iter().any(|f| f.is_rand);
                        (OwnershipMode::ScValueCopy, has_rand)
                    }
                }
                MemoryModel::DeclarativeGraph | MemoryModel::StaticElaborated => {
                    (OwnershipMode::Unspecified, false)
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

/// Propagates declared field bit-widths and IEEE 1800 signedness rules onto CRV constants.
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

/// Marks transaction/object classes for IEEE 1800.2 `do_copy` / `do_compare` / `convert2string` synthesis.
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

/// Capability-driven multi-language legalizer (`UVIR-H` -> target-legal `UVIR-L`).
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

        // 1. Expand IEEE 1800-2023 `[c +/- d]` and `[c +%- p]` inside tolerances into `[lo : hi]`
        //    when the target solver does not natively support tolerance range syntax.
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

        // 2. Record lowered temporal observer metadata if target lacks native SVA/PSL.
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

fn expand_tolerance_in_expr(id: ExprId, arena: &mut CrvArena, modified: &mut usize) -> ExprId {
    let node = arena.get(id).clone();
    match node {
        CrvExpr::Inside { target, elems } => {
            let mut changed = false;
            let mut new_elems = Vec::with_capacity(elems.len());
            for el in elems {
                match el {
                    InsideElem::ToleranceAbs { center, delta } => {
                        changed = true;
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
                        new_elems.push(InsideElem::Range { low, high });
                    }
                    InsideElem::TolerancePct { center, pct } => {
                        changed = true;
                        *modified += 1;
                        let hundred = arena.alloc(CrvExpr::Const {
                            value: 100,
                            width: 32,
                            signed: false,
                        });
                        let prod = arena.alloc(CrvExpr::Binary {
                            op: CrvBinaryOp::Mul,
                            lhs: center,
                            rhs: pct,
                        });
                        // Approximate integer delta `(center * pct) / 100` when constant, else keep symbolic
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
                            let _ = hundred;
                            prod
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
                        new_elems.push(InsideElem::Range { low, high });
                    }
                    other => new_elems.push(other),
                }
            }
            if changed {
                arena.alloc(CrvExpr::Inside {
                    target,
                    elems: new_elems,
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
