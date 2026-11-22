//! Synthesizable SystemVerilog -> typed `svcc_ir::hdl` design lowering.
//!
//! Works on `svlang`'s elaborated design (parameters and generate constructs resolved, one
//! [`Instance`] per elaborated instance) and binds every expression with `svlang`'s
//! IEEE 1800-2023 11.6 / 11.8 [`ExprBinder`], so widths, signedness and all implicit
//! conversions come from the front end instead of being re-derived from text.
//!
//! The lowering is fail-closed: a construct it does not understand is an error with a
//! source location, never a silent omission. The few things skipped on purpose
//! (assertions, `initial` blocks, report tasks) are listed in [`hdl::Design::notes`].

use std::collections::{BTreeMap, HashMap, HashSet};

use svcc_ir::hdl::{
    self, BinOp, CaseArm, Conn, Dir, Edge, Expr, LocalVar, LvBase, LvSel, Lval, Pat, ProcKind,
    Process, Stmt, Ty, UnOp,
};
use svcc_ir::SvccError;
use svcc_svlang::elab::{
    extract_subroutine_formals, BinaryOp, CVal, Child, ElabItem, ElabItemKind, ElaboratedDesign,
    ExprArena, ExprBinder, ExprId, ExprKind, InsideItem, InstanceId, Loc, LocalBinding, LocalId,
    PortDir, RtlType, SymbolId, SymbolKind, SysFunc, UnaryOp,
};
use svcc_svlang::syntax::{Compilation, NodeId, Severity, SyntaxKind, Tree};

const STAGE: &str = "svcc-front-sv/hdl";

/// Parses, elaborates and lowers SystemVerilog RTL source into a typed [`hdl::Design`].
///
/// `tops` selects the top modules; when empty, `svlang` picks every uninstantiated module.
pub fn lower_hdl_design(
    filename: &str,
    source: &str,
    tops: &[String],
) -> Result<hdl::Design, SvccError> {
    let comp = if tops.is_empty() {
        Compilation::from_source(filename, source)
    } else {
        let trees = Compilation::from_source(filename, source).into_trees();
        Compilation::from_trees_with_tops(trees, tops.to_vec())
    };
    let (diags, elab) = comp.check();
    let errs: Vec<String> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}:{}:{}: {}", d.file, d.line, d.column, d.message))
        .collect();
    if !errs.is_empty() {
        return Err(SvccError::new(STAGE, join_errs(&errs)));
    }
    lower_elaborated(comp.trees(), &elab.design)
}

/// Lowers an already elaborated design.
pub fn lower_elaborated(
    trees: &[Tree],
    design: &ElaboratedDesign,
) -> Result<hdl::Design, SvccError> {
    let mut lw = Lowerer {
        trees,
        design,
        errs: Vec::new(),
        notes: Vec::new(),
        out: Vec::new(),
        spec_by_key: HashMap::new(),
        used_module_names: HashSet::new(),
        next_local: 0,
        m: ModState::default(),
    };
    let mut tops = Vec::new();
    for &t in &design.tops {
        let inst = design.instance(t);
        if inst.is_interface || inst.is_checker {
            continue;
        }
        if let Some(name) = lw.lower_instance(t) {
            tops.push(name);
        }
    }
    if !lw.errs.is_empty() {
        return Err(SvccError::new(STAGE, join_errs(&lw.errs)));
    }
    if tops.is_empty() {
        return Err(SvccError::new(STAGE, "no module found to convert"));
    }
    lw.notes.sort();
    lw.notes.dedup();
    Ok(hdl::Design {
        modules: lw.out,
        tops,
        notes: lw.notes,
    })
}

fn join_errs(errs: &[String]) -> String {
    let mut s = errs
        .iter()
        .take(25)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n  ");
    if errs.len() > 25 {
        s.push_str(&format!("\n  ... and {} more", errs.len() - 25));
    }
    s
}

#[derive(Clone)]
struct SigInfo {
    ty: Ty,
    scalar: bool,
    dims: Vec<u32>,
    is_input: bool,
}

#[derive(Default)]
struct ModState {
    sym_names: HashMap<SymbolId, String>,
    sigs: BTreeMap<String, SigInfo>,
    used: HashSet<String>,
    ports: Vec<hdl::Port>,
    signals: Vec<hdl::Signal>,
    functions: Vec<hdl::Function>,
    processes: Vec<Process>,
    instances: Vec<hdl::Instance>,
    func_names: HashMap<(usize, NodeId, String), String>,
    func_stack: Vec<(usize, NodeId, String)>,
}

impl ModState {
    fn unique(&mut self, base: &str) -> String {
        let base = sanitize(base);
        if self.used.insert(base.clone()) {
            return base;
        }
        let mut n = 1;
        loop {
            let cand = format!("{base}_{n}");
            if self.used.insert(cand.clone()) {
                return cand;
            }
            n += 1;
        }
    }
}

fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => out.push(ch),
            '.' => out.push_str("__"),
            '[' => out.push('_'),
            ']' => {}
            _ => out.push('_'),
        }
    }
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    out
}

/// Lowering context of one process or function body.
#[derive(Clone)]
struct Ctx {
    inst: InstanceId,
    prefix: String,
    gen_consts: Vec<(String, CVal)>,
    /// Visible local bindings; later entries shadow earlier ones.
    locals: Vec<(String, LocalBinding)>,
    local_names: HashMap<u32, (String, Ty)>,
    decls: Vec<LocalVar>,
    used: HashSet<String>,
    /// Return type when lowering a function body.
    func_ret: Option<Ty>,
    allow_nba: bool,
}

impl Ctx {
    fn new(inst: InstanceId, prefix: &str, gen_consts: &[(String, CVal)]) -> Self {
        Ctx {
            inst,
            prefix: prefix.to_string(),
            gen_consts: gen_consts.to_vec(),
            locals: Vec::new(),
            local_names: HashMap::new(),
            decls: Vec::new(),
            used: HashSet::new(),
            func_ret: None,
            allow_nba: false,
        }
    }
}

struct Lowerer<'a> {
    trees: &'a [Tree],
    design: &'a ElaboratedDesign,
    errs: Vec<String>,
    notes: Vec<String>,
    out: Vec<hdl::Module>,
    spec_by_key: HashMap<String, String>,
    used_module_names: HashSet<String>,
    next_local: u32,
    m: ModState,
}

fn ity(t: &RtlType) -> Option<Ty> {
    t.integral_shape().map(|(w, s, _)| Ty::new(w, s))
}

fn own_toks(t: &Tree, node: NodeId) -> Vec<&str> {
    t.node(node)
        .children
        .iter()
        .filter_map(|c| match *c {
            Child::Token(i) if !t.tokens[i as usize].kind.is_trivia() => {
                Some(&*t.tokens[i as usize].text)
            }
            _ => None,
        })
        .collect()
}

fn const_words(cv: &CVal, w: u32) -> Vec<u64> {
    let n = w.div_ceil(64).max(1);
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let v = cv.v.bits(64 * i, 64).low_i128() as u64;
        let u = cv.u.bits(64 * i, 64).low_i128() as u64;
        let mut word = v & !u;
        let top = w - 64 * i;
        if top < 64 {
            word &= (1u64 << top) - 1;
        }
        out.push(word);
    }
    out
}

fn unknown_words(cv: &CVal, w: u32) -> Vec<u64> {
    let n = w.div_ceil(64).max(1);
    (0..n)
        .map(|i| {
            let mut u = cv.u.bits(64 * i, 64).low_i128() as u64;
            let top = w - 64 * i;
            if top < 64 {
                u &= (1u64 << top) - 1;
            }
            u
        })
        .collect()
}

fn z_words(cv: &CVal, w: u32) -> Vec<u64> {
    // At unknown bits the value bit is 0 for x and 1 for z.
    let n = w.div_ceil(64).max(1);
    (0..n)
        .map(|i| {
            let v = cv.v.bits(64 * i, 64).low_i128() as u64;
            let u = cv.u.bits(64 * i, 64).low_i128() as u64;
            let mut z = v & u;
            let top = w - 64 * i;
            if top < 64 {
                z &= (1u64 << top) - 1;
            }
            z
        })
        .collect()
}

fn ones_words(w: u32) -> Vec<u64> {
    let n = w.div_ceil(64).max(1);
    (0..n)
        .map(|i| {
            let top = w - 64 * i;
            if top < 64 {
                (1u64 << top) - 1
            } else {
                u64::MAX
            }
        })
        .collect()
}

fn const_expr(cv: &CVal, ty: Ty) -> Expr {
    let cv2;
    let cv = if cv.w != ty.w {
        cv2 = cv.cast(ty.w, cv.s);
        &cv2
    } else {
        cv
    };
    Expr::Const {
        ty,
        words: const_words(cv, ty.w),
    }
}

fn get_bit(words: &[u64], i: u32) -> bool {
    words
        .get((i / 64) as usize)
        .is_some_and(|w| (w >> (i % 64)) & 1 == 1)
}

fn set_bit(words: &mut [u64], i: u32) {
    words[(i / 64) as usize] |= 1u64 << (i % 64);
}

/// SystemVerilog assignment-like conversion of `e` to type `to`: extend by the signedness of
/// the source, or truncate, then reinterpret the sign.
fn coerce(e: Expr, to: Ty) -> Expr {
    let from = e.ty();
    if from == to {
        return e;
    }
    if let Expr::Const { words, .. } = &e {
        let mut out = vec![0u64; to.w.div_ceil(64) as usize];
        let neg = from.s && get_bit(words, from.w - 1);
        for i in 0..to.w {
            let b = if i < from.w { get_bit(words, i) } else { neg };
            if b {
                set_bit(&mut out, i);
            }
        }
        return Expr::Const { ty: to, words: out };
    }
    let mut x = e;
    if from.w != to.w {
        x = Expr::Resize {
            arg: Box::new(x),
            ty: Ty::new(to.w, from.s),
        };
    }
    if x.ty().s != to.s {
        x = Expr::Cast {
            arg: Box::new(x),
            ty: to,
        };
    }
    x
}

fn truth(e: Expr) -> Expr {
    let t = e.ty();
    if t.w == 1 {
        coerce(e, Ty::BIT)
    } else {
        Expr::Unary {
            op: UnOp::RedOr,
            arg: Box::new(e),
            ty: Ty::BIT,
        }
    }
}

fn lnot(e: Expr) -> Expr {
    Expr::Unary {
        op: UnOp::LogNot,
        arg: Box::new(e),
        ty: Ty::BIT,
    }
}

fn bin(op: BinOp, l: Expr, r: Expr, ty: Ty) -> Expr {
    Expr::Binary {
        op,
        lhs: Box::new(l),
        rhs: Box::new(r),
        ty,
    }
}

/// Signed integer type wide enough for index arithmetic on `e`.
fn index_ty(e: &Expr) -> Ty {
    if e.ty().w < 32 {
        Ty::INT
    } else {
        Ty::new(64, true)
    }
}

fn add_dyn(a: Option<Expr>, b: Option<Expr>) -> Option<Expr> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => {
            let ty = if a.ty().w < 32 && b.ty().w < 32 {
                Ty::INT
            } else {
                Ty::new(64, true)
            };
            Some(bin(BinOp::Add, coerce(a, ty), coerce(b, ty), ty))
        }
    }
}

/// Position of a selected slice inside its base.
struct Pos {
    lo: u32,
    lo_dyn: Option<Expr>,
}

impl<'a> Lowerer<'a> {
    fn err(&mut self, loc: Loc, msg: impl AsRef<str>) {
        let file = self
            .design
            .files
            .get(loc.file as usize)
            .cloned()
            .unwrap_or_default();
        self.errs.push(format!(
            "{}:{}:{}: {}",
            file,
            loc.line,
            loc.column,
            msg.as_ref()
        ));
    }

    fn note(&mut self, loc: Loc, msg: impl AsRef<str>) {
        let file = self
            .design
            .files
            .get(loc.file as usize)
            .cloned()
            .unwrap_or_default();
        self.notes
            .push(format!("{}:{}: {}", file, loc.line, msg.as_ref()));
    }

    /// Runs `f` with an expression binder scoped to `ctx`, collecting its diagnostics.
    fn with_binder<R>(
        &mut self,
        ctx: &Ctx,
        arena: &mut ExprArena,
        f: impl FnOnce(&mut ExprBinder) -> R,
    ) -> R {
        let (res, diags) = {
            let mut b = ExprBinder::new(
                self.trees,
                self.design,
                ctx.inst,
                &ctx.prefix,
                &ctx.gen_consts,
                arena,
            );
            for (n, l) in &ctx.locals {
                b.insert_local(n.clone(), l.clone());
            }
            let r = f(&mut b);
            (r, std::mem::take(&mut b.diags))
        };
        for d in diags {
            self.err(d.loc, d.message);
        }
        res
    }

    fn node_loc(&mut self, ctx: &Ctx, tree: usize, node: NodeId) -> Loc {
        let mut arena = ExprArena::new();
        self.with_binder(ctx, &mut arena, |b| b.loc_of(tree, node))
    }

    // ------------------------------------------------------------------
    // Modules
    // ------------------------------------------------------------------

    fn lower_instance(&mut self, id: InstanceId) -> Option<String> {
        let inst = self.design.instance(id);
        let key = format!(
            "{}|{:?}|{:?}",
            inst.definition, inst.params, inst.type_params
        );
        if let Some(n) = self.spec_by_key.get(&key) {
            return Some(n.clone());
        }
        if inst.is_interface || inst.is_checker || inst.is_blackbox_stub {
            self.err(
                inst.loc,
                format!(
                    "instance '{}' of interface, checker or black box '{}' is not supported",
                    inst.path, inst.definition
                ),
            );
            return None;
        }
        // Children first, so modules come out in dependency order.
        let mut child_modules = Vec::new();
        for &c in &inst.children {
            child_modules.push(self.lower_instance(c));
        }

        let mut name = sanitize(&inst.definition);
        let mut n = 1;
        while !self.used_module_names.insert(name.clone()) {
            name = format!("{}_p{}", sanitize(&inst.definition), n);
            n += 1;
        }
        self.spec_by_key.insert(key, name.clone());

        let saved = std::mem::take(&mut self.m);
        self.m.used.insert(name.clone());

        // Ports and signals.
        for &sid in &inst.symbols {
            let sym = self.design.symbol(sid);
            match &sym.kind {
                SymbolKind::Port { dir, .. } => {
                    let Some(ty) = ity(&sym.ty) else {
                        self.err(
                            sym.loc,
                            format!(
                                "port '{}' has a type that is not a packed integral type",
                                sym.name
                            ),
                        );
                        continue;
                    };
                    let d = match dir {
                        PortDir::Input => Dir::In,
                        PortDir::Output => Dir::Out,
                        _ => {
                            self.err(
                                sym.loc,
                                format!("inout or ref port '{}' is not supported", sym.name),
                            );
                            continue;
                        }
                    };
                    let nm = self.m.unique(&sym.rel_path);
                    let scalar = sym.packed_bounds.is_none() && ty == Ty::BIT;
                    self.m.sym_names.insert(sid, nm.clone());
                    self.m.sigs.insert(
                        nm.clone(),
                        SigInfo {
                            ty,
                            scalar,
                            dims: Vec::new(),
                            is_input: d == Dir::In,
                        },
                    );
                    self.m.ports.push(hdl::Port {
                        name: nm,
                        dir: d,
                        ty,
                        scalar,
                    });
                }
                SymbolKind::Var | SymbolKind::Net { .. } => {
                    let (ty, dims) = match &sym.ty {
                        RtlType::UnpackedArray { elem, dims } => {
                            let Some(ty) = ity(elem) else {
                                self.err(
                                    sym.loc,
                                    format!(
                                        "array '{}' has elements that are not packed integral values",
                                        sym.name
                                    ),
                                );
                                continue;
                            };
                            let sizes = dims
                                .iter()
                                .map(|(l, r)| (l.abs_diff(*r) + 1) as u32)
                                .collect();
                            (ty, sizes)
                        }
                        RtlType::Event | RtlType::Interface { .. } => continue,
                        other => {
                            let Some(ty) = ity(other) else {
                                self.err(
                                    sym.loc,
                                    format!("variable '{}' has an unsupported type", sym.name),
                                );
                                continue;
                            };
                            (ty, Vec::new())
                        }
                    };
                    if let SymbolKind::Net { net_type } = &sym.kind {
                        if !matches!(net_type.as_str(), "wire" | "tri" | "uwire") {
                            self.err(sym.loc, format!("net type '{net_type}' is not supported"));
                        }
                    }
                    let nm = self.m.unique(&sym.rel_path);
                    let scalar = sym.packed_bounds.is_none() && ty == Ty::BIT;
                    let mut init = None;
                    if matches!(sym.kind, SymbolKind::Var) && sym.init_node.is_some() {
                        match (&sym.cval, dims.is_empty()) {
                            (Some(cv), true) => init = Some(const_words(cv, ty.w)),
                            _ => self.err(
                                sym.loc,
                                format!(
                                    "variable '{}' has an initializer that is not a constant packed value",
                                    sym.name
                                ),
                            ),
                        }
                    }
                    self.m.sym_names.insert(sid, nm.clone());
                    self.m.sigs.insert(
                        nm.clone(),
                        SigInfo {
                            ty,
                            scalar,
                            dims: dims.clone(),
                            is_input: false,
                        },
                    );
                    self.m.signals.push(hdl::Signal {
                        name: nm,
                        ty,
                        scalar,
                        dims,
                        init,
                    });
                }
                SymbolKind::Param { .. } | SymbolKind::Genvar => {}
            }
        }

        // Net declaration assignments (`wire x = expr;`) are continuous assignments.
        for &sid in &inst.symbols {
            let sym = self.design.symbol(sid);
            if let (SymbolKind::Net { .. }, Some((tree, node))) = (&sym.kind, sym.init_node) {
                let Some(nm) = self.m.sym_names.get(&sid).cloned() else {
                    continue;
                };
                let Some(ty) = ity(&sym.ty) else { continue };
                let ctx = Ctx::new(id, &sym.init_scope_prefix, &[]);
                let mut arena = ExprArena::new();
                let lhs_ty = sym.ty.clone();
                let bound =
                    self.with_binder(&ctx, &mut arena, |b| b.bind_assigned(tree, node, &lhs_ty));
                let Some(rhs) = bound.and_then(|e| self.conv(&arena, e, &ctx)) else {
                    continue;
                };
                let pname = self.m.unique(&format!("assign_{nm}"));
                self.m.processes.push(Process {
                    name: pname,
                    kind: ProcKind::Comb,
                    locals: Vec::new(),
                    body: vec![Stmt::Assign {
                        lhs: Lval {
                            base: LvBase::Sig {
                                name: nm.clone(),
                                ty,
                            },
                            sel: None,
                        },
                        rhs: coerce(rhs, ty),
                        nonblocking: false,
                    }],
                    merged_from: 1,
                    line: sym.loc.line,
                });
            }
        }

        // Processes.
        for item in &inst.items {
            match item.kind {
                ElabItemKind::ProceduralBlock => self.lower_proc_block(id, item),
                ElabItemKind::ContinuousAssign => self.lower_cont_assign(id, item),
                ElabItemKind::FunctionDecl
                | ElabItemKind::LetDecl
                | ElabItemKind::SequenceDecl
                | ElabItemKind::PropertyDecl
                | ElabItemKind::ModportDecl => {}
                ElabItemKind::AssertStmt
                | ElabItemKind::ConcurrentAssert
                | ElabItemKind::ClockingDecl
                | ElabItemKind::DefaultDisable => {
                    self.note(item.loc, "assertion or clocking construct skipped");
                }
                ElabItemKind::TaskDecl => {}
                ElabItemKind::GateInst | ElabItemKind::UdpInst | ElabItemKind::NetAlias => {
                    self.err(
                        item.loc,
                        "gate primitives, UDPs and net aliases are not supported",
                    );
                }
            }
        }

        // Child instances.
        for (ci, &c) in inst.children.iter().enumerate() {
            if let Some(Some(child_mod)) = child_modules.get(ci) {
                let child_mod = child_mod.clone();
                self.lower_child(id, c, &child_mod);
            }
        }

        self.resolve_drivers(inst.loc);

        let m = std::mem::replace(&mut self.m, saved);
        let file = self
            .design
            .files
            .get(inst.loc.file as usize)
            .cloned()
            .unwrap_or_default();
        self.out.push(hdl::Module {
            name: name.clone(),
            definition: inst.definition.clone(),
            params: inst
                .params
                .iter()
                .map(|(n, v)| {
                    (
                        n.clone(),
                        v.as_ref().map(|c| c.show()).unwrap_or_else(|| "?".into()),
                    )
                })
                .collect(),
            ports: m.ports,
            signals: m.signals,
            functions: m.functions,
            processes: m.processes,
            instances: m.instances,
            file,
            line: inst.loc.line,
        });
        Some(name)
    }

    fn add_glue(&mut self, base: &str, ty: Ty, scalar: bool) -> String {
        let nm = self.m.unique(base);
        self.m.sigs.insert(
            nm.clone(),
            SigInfo {
                ty,
                scalar,
                dims: Vec::new(),
                is_input: false,
            },
        );
        self.m.signals.push(hdl::Signal {
            name: nm.clone(),
            ty,
            scalar,
            dims: Vec::new(),
            init: None,
        });
        nm
    }

    fn lower_child(&mut self, parent: InstanceId, child_id: InstanceId, child_mod: &str) {
        let child = self.design.instance(child_id);
        let inst_name = self.m.unique(&child.rel_path);
        let mut conns: Vec<Conn> = Vec::new();
        let mut done: HashSet<SymbolId> = HashSet::new();
        let child_ports: Vec<SymbolId> = child.ports.clone();

        for pc in &child.port_conns {
            let port_sid = match &pc.port_name {
                Some(n) => child_ports
                    .iter()
                    .copied()
                    .find(|s| self.design.symbol(*s).name == *n),
                None => child_ports.get(pc.arg_index).copied(),
            };
            let Some(port_sid) = port_sid else {
                self.err(pc.loc, "port connection does not match a port");
                continue;
            };
            let psym = self.design.symbol(port_sid);
            let Some(pty) = ity(&psym.ty) else { continue };
            let pscalar = psym.packed_bounds.is_none() && pty == Ty::BIT;
            let is_in = matches!(
                psym.kind,
                SymbolKind::Port {
                    dir: PortDir::Input,
                    ..
                }
            );
            done.insert(port_sid);
            let pname = sanitize(&psym.name);
            let ctx = Ctx::new(parent, &pc.scope_prefix, &pc.gen_consts);

            // Resolve the actual: an expression node, or the same-named parent signal for
            // `.name` and `.*` connections.
            let mut arena = ExprArena::new();
            let bound: Option<ExprId> = match pc.expr {
                Some((tree, node)) => {
                    if is_in {
                        let pt = psym.ty.clone();
                        self.with_binder(&ctx, &mut arena, |b| b.bind_assigned(tree, node, &pt))
                    } else {
                        self.with_binder(&ctx, &mut arena, |b| b.bind_self(tree, node))
                    }
                }
                None => {
                    if pc.implicit_named || pc.wildcard {
                        let pinst = self.design.instance(parent);
                        match pinst.resolve_lexical(&pc.scope_prefix, &psym.name) {
                            Some(sid) => {
                                let s = self.design.symbol(sid);
                                Some(arena.alloc(ExprKind::Symbol(sid), s.ty.clone(), None, pc.loc))
                            }
                            None => {
                                if pc.implicit_named {
                                    self.err(
                                        pc.loc,
                                        format!(
                                            "implicit connection '.{}' has no matching signal",
                                            psym.name
                                        ),
                                    );
                                }
                                None
                            }
                        }
                    } else {
                        None
                    }
                }
            };
            let Some(bound) = bound else {
                // Unconnected: inputs read 0, outputs are left open.
                let g = self.add_glue(&format!("{inst_name}_{pname}"), pty, pscalar);
                conns.push(Conn {
                    port: pname,
                    sig: g,
                    elem: Vec::new(),
                });
                continue;
            };

            if is_in {
                let Some(e) = self.conv(&arena, bound, &ctx) else {
                    continue;
                };
                let e = coerce(e, pty);
                if let Some(c) = self.direct_conn(&e, pty, pscalar, &pname) {
                    conns.push(c);
                    continue;
                }
                let g = self.add_glue(&format!("{inst_name}_{pname}"), pty, pscalar);
                let proc_name = self.m.unique(&format!("drive_{g}"));
                self.m.processes.push(Process {
                    name: proc_name,
                    kind: ProcKind::Comb,
                    locals: Vec::new(),
                    body: vec![Stmt::Assign {
                        lhs: Lval {
                            base: LvBase::Sig {
                                name: g.clone(),
                                ty: pty,
                            },
                            sel: None,
                        },
                        rhs: e,
                        nonblocking: false,
                    }],
                    merged_from: 1,
                    line: pc.loc.line,
                });
                conns.push(Conn {
                    port: pname,
                    sig: g,
                    elem: Vec::new(),
                });
            } else {
                let Some(lvs) = self.lvals(&arena, bound, &ctx) else {
                    continue;
                };
                if lvs.len() == 1 && lvs[0].sel.is_none() {
                    let as_expr = match &lvs[0].base {
                        LvBase::Sig { name, ty } => Some(Expr::Sig {
                            name: name.clone(),
                            ty: *ty,
                        }),
                        LvBase::Elem { name, idx, ty } => Some(Expr::Elem {
                            name: name.clone(),
                            idx: idx.clone(),
                            ty: *ty,
                        }),
                        LvBase::Local { .. } => None,
                    };
                    if let Some(c) =
                        as_expr.and_then(|e| self.direct_conn(&e, pty, pscalar, &pname))
                    {
                        conns.push(c);
                        continue;
                    }
                }
                let g = self.add_glue(&format!("{inst_name}_{pname}"), pty, pscalar);
                let mut body = Vec::new();
                self.assign_lvals(
                    lvs,
                    Expr::Sig {
                        name: g.clone(),
                        ty: pty,
                    },
                    false,
                    &mut body,
                    None,
                );
                let proc_name = self.m.unique(&format!("sink_{g}"));
                self.m.processes.push(Process {
                    name: proc_name,
                    kind: ProcKind::Comb,
                    locals: Vec::new(),
                    body,
                    merged_from: 1,
                    line: pc.loc.line,
                });
                conns.push(Conn {
                    port: pname,
                    sig: g,
                    elem: Vec::new(),
                });
            }
        }

        for &ps in &child_ports {
            if done.contains(&ps) {
                continue;
            }
            let psym = self.design.symbol(ps);
            let Some(pty) = ity(&psym.ty) else { continue };
            let pscalar = psym.packed_bounds.is_none() && pty == Ty::BIT;
            let pname = sanitize(&psym.name);
            let g = self.add_glue(&format!("{inst_name}_{pname}"), pty, pscalar);
            conns.push(Conn {
                port: pname,
                sig: g,
                elem: Vec::new(),
            });
        }

        self.m.instances.push(hdl::Instance {
            name: inst_name,
            module: child_mod.to_string(),
            conns,
        });
    }

    /// A port can bind straight to a parent signal when the actual is a whole signal (or one
    /// constant array element) of exactly the port's type.
    fn direct_conn(&self, e: &Expr, pty: Ty, pscalar: bool, port: &str) -> Option<Conn> {
        match e {
            Expr::Sig { name, ty } => {
                let info = self.m.sigs.get(name)?;
                (*ty == pty && info.scalar == pscalar && info.dims.is_empty()).then(|| Conn {
                    port: port.to_string(),
                    sig: name.clone(),
                    elem: Vec::new(),
                })
            }
            Expr::Elem { name, idx, ty } => {
                let info = self.m.sigs.get(name)?;
                if *ty != pty || info.scalar != pscalar {
                    return None;
                }
                let mut elem = Vec::new();
                for (i, e) in idx.iter().enumerate() {
                    let v = e.as_u64()?;
                    if v >= *info.dims.get(i)? as u64 {
                        return None;
                    }
                    elem.push(v as u32);
                }
                Some(Conn {
                    port: port.to_string(),
                    sig: name.clone(),
                    elem,
                })
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------------
    // Driver analysis: several blocks driving parts of one signal are merged
    // into one process, because a SystemC signal has a single writer.
    // ------------------------------------------------------------------

    fn resolve_drivers(&mut self, loc: Loc) {
        // Turn directly bound instance outputs into glue when the signal has other drivers.
        loop {
            let proc_keys: Vec<HashSet<String>> = self
                .m
                .processes
                .iter()
                .map(|p| driver_keys(&p.body))
                .collect();
            let mut changed = false;
            let child_out: Vec<(usize, usize, String)> = self
                .m
                .instances
                .iter()
                .enumerate()
                .flat_map(|(ii, inst)| {
                    let out = &self.out;
                    inst.conns
                        .iter()
                        .enumerate()
                        .filter(move |(_, c)| {
                            out.iter()
                                .find(|m| m.name == inst.module)
                                .and_then(|m| m.ports.iter().find(|p| p.name == c.port))
                                .is_some_and(|p| p.dir == Dir::Out)
                        })
                        .map(move |(ci, c)| (ii, ci, conn_key(c)))
                })
                .collect();
            for (n, (ii, ci, key)) in child_out.iter().enumerate() {
                let base = key.split('[').next().unwrap_or(key).to_string();
                let clash_proc = proc_keys
                    .iter()
                    .any(|ks| ks.iter().any(|k| keys_overlap(k, key)));
                let clash_inst = child_out
                    .iter()
                    .enumerate()
                    .any(|(m2, (_, _, k2))| m2 != n && keys_overlap(k2, key));
                if !(clash_proc || clash_inst) {
                    continue;
                }
                let Some(info) = self.m.sigs.get(&base).cloned() else {
                    continue;
                };
                let conn = self.m.instances[*ii].conns[*ci].clone();
                let iname = self.m.instances[*ii].name.clone();
                let g = self.add_glue(&format!("{iname}_{}", conn.port), info.ty, info.scalar);
                let lhs_base = if conn.elem.is_empty() {
                    LvBase::Sig {
                        name: conn.sig.clone(),
                        ty: info.ty,
                    }
                } else {
                    LvBase::Elem {
                        name: conn.sig.clone(),
                        idx: conn
                            .elem
                            .iter()
                            .map(|i| Expr::konst(Ty::INT, *i as u64))
                            .collect(),
                        ty: info.ty,
                    }
                };
                let pname = self.m.unique(&format!("sink_{g}"));
                self.m.processes.push(Process {
                    name: pname,
                    kind: ProcKind::Comb,
                    locals: Vec::new(),
                    body: vec![Stmt::Assign {
                        lhs: Lval {
                            base: lhs_base,
                            sel: None,
                        },
                        rhs: Expr::Sig {
                            name: g.clone(),
                            ty: info.ty,
                        },
                        nonblocking: false,
                    }],
                    merged_from: 1,
                    line: 0,
                });
                self.m.instances[*ii].conns[*ci] = Conn {
                    port: conn.port,
                    sig: g,
                    elem: Vec::new(),
                };
                changed = true;
                break;
            }
            if !changed {
                break;
            }
        }

        // Union processes that share a driven signal.
        let n = self.m.processes.len();
        let keys: Vec<HashSet<String>> = self
            .m
            .processes
            .iter()
            .map(|p| driver_keys(&p.body))
            .collect();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while p[r] != r {
                r = p[r];
            }
            let mut c = i;
            while p[c] != r {
                let nx = p[c];
                p[c] = r;
                c = nx;
            }
            r
        }
        for i in 0..n {
            for j in (i + 1)..n {
                let overlap = keys[i]
                    .iter()
                    .any(|a| keys[j].iter().any(|b| keys_overlap(a, b)));
                if overlap {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[rj.max(ri)] = ri.min(rj);
                    }
                }
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..n {
            let r = find(&mut parent, i);
            groups.entry(r).or_default().push(i);
        }
        if groups.len() == n {
            return;
        }
        let old = std::mem::take(&mut self.m.processes);
        let mut slots: Vec<Option<Process>> = old.into_iter().map(Some).collect();
        let mut merged: Vec<Process> = Vec::new();
        for (_, members) in groups {
            if members.len() == 1 {
                merged.push(slots[members[0]].take().expect("process"));
                continue;
            }
            let procs: Vec<Process> = members
                .iter()
                .map(|&i| slots[i].take().expect("process"))
                .collect();
            let kind = procs[0].kind.clone();
            if procs.iter().any(|p| p.kind != kind) {
                let shared: Vec<String> = procs.iter().map(|p| p.name.clone()).collect();
                self.err(
                    loc,
                    format!(
                        "a signal is driven by processes with different triggers ({}); this needs a single process in SystemC",
                        shared.join(", ")
                    ),
                );
                merged.extend(procs);
                continue;
            }
            let mut targets: Vec<String> = procs
                .iter()
                .flat_map(|p| driver_keys(&p.body))
                .map(|k| k.split('[').next().unwrap_or("").to_string())
                .collect();
            targets.sort();
            targets.dedup();
            let base = match kind {
                ProcKind::Comb => format!("comb_{}", targets.join("_")),
                ProcKind::Seq { .. } => format!("ff_{}", targets.join("_")),
            };
            let base: String = base.chars().take(48).collect();
            let name = self.m.unique(&base);
            let line = procs.iter().map(|p| p.line).find(|l| *l != 0).unwrap_or(0);
            let count = procs.iter().map(|p| p.merged_from).sum();
            let body = procs
                .into_iter()
                .map(|p| Stmt::Block {
                    locals: p.locals,
                    body: p.body,
                })
                .collect();
            merged.push(Process {
                name,
                kind,
                locals: Vec::new(),
                body,
                merged_from: count,
                line,
            });
        }
        self.m.processes = merged;
    }

    // ------------------------------------------------------------------
    // Processes
    // ------------------------------------------------------------------

    fn lower_cont_assign(&mut self, inst: InstanceId, item: &ElabItem) {
        let t = &self.trees[item.tree];
        let mut ctx = Ctx::new(inst, &item.scope_prefix, &item.gen_consts);
        let mut body = Vec::new();
        for ch in t.children(item.node) {
            if t.kind(ch) == SyntaxKind::BinaryExpr {
                let kids: Vec<NodeId> = t
                    .children(ch)
                    .filter(|&c| t.kind(c) != SyntaxKind::Attribute)
                    .collect();
                if kids.len() == 2 && own_toks(t, ch).first() == Some(&"=") {
                    self.assignment(item.tree, kids[0], "=", Some(kids[1]), &mut ctx, &mut body);
                    continue;
                }
            }
            if !matches!(t.kind(ch), SyntaxKind::Attribute) {
                self.err(
                    item.loc,
                    "continuous assignment with drive strength or delay is not supported",
                );
            }
        }
        if body.is_empty() {
            return;
        }
        let target = first_target(&body).unwrap_or_else(|| "net".into());
        let name = self.m.unique(&format!("assign_{target}"));
        self.m.processes.push(Process {
            name,
            kind: ProcKind::Comb,
            locals: ctx.decls,
            body,
            merged_from: 1,
            line: item.loc.line,
        });
    }

    fn lower_proc_block(&mut self, inst: InstanceId, item: &ElabItem) {
        let t = &self.trees[item.tree];
        let kw = own_toks(t, item.node).first().copied().unwrap_or("");
        let first = t
            .children(item.node)
            .find(|&c| t.kind(c) != SyntaxKind::Attribute);
        let Some(first) = first else { return };
        let mut ctx = Ctx::new(inst, &item.scope_prefix, &item.gen_consts);
        match kw {
            "initial" | "final" => {
                self.note(
                    item.loc,
                    format!("'{kw}' block skipped (not part of the synthesizable design)"),
                );
            }
            "always_comb" | "always_latch" => {
                let mut body = Vec::new();
                self.stmt(item.tree, first, &mut ctx, &mut body);
                self.push_proc(item, ProcKind::Comb, ctx, body, "comb");
            }
            "always_ff" | "always" => {
                if t.kind(first) != SyntaxKind::TimedStmt {
                    self.err(
                        item.loc,
                        "always block without an event control is not synthesizable",
                    );
                    return;
                }
                let Some(tc) = t.child(first, SyntaxKind::TimingControl) else {
                    self.err(item.loc, "always block needs an @(...) event control");
                    return;
                };
                let events: Vec<NodeId> = t
                    .children(tc)
                    .filter(|&c| t.kind(c) == SyntaxKind::EventExpr)
                    .collect();
                let mut flat = Vec::new();
                for ev in events {
                    flatten_events(t, ev, &mut flat);
                }
                let has_edge = flat.iter().any(|&ev| {
                    own_toks(t, ev)
                        .iter()
                        .any(|k| matches!(*k, "posedge" | "negedge" | "edge"))
                });
                let Some(body_node) = t.children(first).find(|&c| {
                    !matches!(t.kind(c), SyntaxKind::Attribute | SyntaxKind::TimingControl)
                }) else {
                    return;
                };
                if !has_edge {
                    if kw == "always_ff" {
                        self.err(item.loc, "always_ff needs an edge event control");
                        return;
                    }
                    // `always @*` or a level-sensitive list: combinational. The complete
                    // read set is used as sensitivity, as synthesis does.
                    let mut body = Vec::new();
                    self.stmt(item.tree, body_node, &mut ctx, &mut body);
                    self.push_proc(item, ProcKind::Comb, ctx, body, "comb");
                    return;
                }
                let mut edges = Vec::new();
                for ev in flat {
                    let toks = own_toks(t, ev);
                    let pos = if toks.contains(&"posedge") {
                        true
                    } else if toks.contains(&"negedge") {
                        false
                    } else {
                        self.err(
                            item.loc,
                            "event control mixes edge and level terms, or uses 'edge'",
                        );
                        return;
                    };
                    let Some(sig) = t.children(ev).find(|&c| t.kind(c) != SyntaxKind::Attribute)
                    else {
                        continue;
                    };
                    let mut arena = ExprArena::new();
                    let b = self.with_binder(&ctx, &mut arena, |b| b.bind_self(item.tree, sig));
                    let Some(e) = b.and_then(|e| self.conv(&arena, e, &ctx)) else {
                        continue;
                    };
                    match e {
                        Expr::Sig { name, ty } if ty.w == 1 => edges.push(Edge {
                            sig: name,
                            posedge: pos,
                            is_async_ctrl: false,
                        }),
                        _ => {
                            self.err(item.loc, "edge event must be a plain 1-bit signal");
                            return;
                        }
                    }
                }
                ctx.allow_nba = true;
                let mut body = Vec::new();
                self.stmt(item.tree, body_node, &mut ctx, &mut body);
                let reads = read_sigs(&body);
                for e in &mut edges {
                    e.is_async_ctrl = reads.contains(&e.sig);
                }
                self.push_proc(item, ProcKind::Seq { edges }, ctx, body, "ff");
            }
            other => self.err(item.loc, format!("unsupported procedural block '{other}'")),
        }
    }

    fn push_proc(
        &mut self,
        item: &ElabItem,
        kind: ProcKind,
        ctx: Ctx,
        body: Vec<Stmt>,
        prefix: &str,
    ) {
        if body.is_empty() {
            return;
        }
        let base = match first_target(&body) {
            Some(tg) => format!("{prefix}_{tg}"),
            None => format!("{prefix}_L{}", item.loc.line),
        };
        let name = self.m.unique(&base);
        self.m.processes.push(Process {
            name,
            kind,
            locals: ctx.decls,
            body,
            merged_from: 1,
            line: item.loc.line,
        });
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn stmt(&mut self, tree: usize, node: NodeId, ctx: &mut Ctx, out: &mut Vec<Stmt>) {
        let t = &self.trees[tree];
        match t.kind(node) {
            SyntaxKind::EmptyStmt
            | SyntaxKind::ImportDecl
            | SyntaxKind::EndLabel
            | SyntaxKind::Attribute
            | SyntaxKind::StmtLabel
            | SyntaxKind::Name
            | SyntaxKind::TypedefDecl => {}
            SyntaxKind::AssertStmt | SyntaxKind::ConcurrentAssert => {
                let loc = self.node_loc(ctx, tree, node);
                self.note(loc, "assertion skipped");
            }
            SyntaxKind::SeqBlock => {
                let saved = ctx.locals.len();
                for ch in t.children(node) {
                    self.stmt(tree, ch, ctx, out);
                }
                ctx.locals.truncate(saved);
            }
            SyntaxKind::DataDecl | SyntaxKind::ParamDecl => self.local_decl(tree, node, ctx, out),
            SyntaxKind::IfStmt => {
                let kids: Vec<NodeId> = t
                    .children(node)
                    .filter(|&c| {
                        !matches!(t.kind(c), SyntaxKind::Attribute | SyntaxKind::StmtLabel)
                    })
                    .collect();
                let (Some(&cond_n), Some(&then_n)) = (kids.first(), kids.get(1)) else {
                    return;
                };
                let else_n = match t.child(node, SyntaxKind::ElseClause) {
                    Some(ec) => t.children(ec).find(|&c| t.kind(c) != SyntaxKind::Attribute),
                    None => kids
                        .get(2)
                        .copied()
                        .filter(|&c| t.kind(c) != SyntaxKind::ElseClause),
                };
                let Some(cond) = self.cond(tree, cond_n, ctx) else {
                    return;
                };
                let mut then_s = Vec::new();
                let saved = ctx.locals.len();
                self.stmt(tree, then_n, ctx, &mut then_s);
                ctx.locals.truncate(saved);
                let mut else_s = Vec::new();
                if let Some(e) = else_n {
                    self.stmt(tree, e, ctx, &mut else_s);
                    ctx.locals.truncate(saved);
                }
                match cond.as_u64() {
                    Some(0) => out.extend(else_s),
                    Some(_) => out.extend(then_s),
                    None => out.push(Stmt::If {
                        cond,
                        then_s,
                        else_s,
                    }),
                }
            }
            SyntaxKind::CaseStmt => self.case_stmt(tree, node, ctx, out),
            SyntaxKind::LoopStmt | SyntaxKind::DoWhileStmt => self.loop_stmt(tree, node, ctx, out),
            SyntaxKind::ExprStmt => {
                if let Some(e) = t
                    .children(node)
                    .find(|&c| !matches!(t.kind(c), SyntaxKind::Attribute | SyntaxKind::StmtLabel))
                {
                    self.expr_stmt(tree, e, ctx, out);
                }
            }
            SyntaxKind::ReturnStmt => {
                let Some(ret) = ctx.func_ret else {
                    let loc = self.node_loc(ctx, tree, node);
                    self.err(loc, "'return' outside a function");
                    return;
                };
                let val = t
                    .children(node)
                    .find(|&c| t.kind(c) != SyntaxKind::Attribute);
                let e = match val {
                    Some(v) => {
                        let rt = RtlType::Integral {
                            width: ret.w,
                            signed: ret.s,
                            four_state: true,
                        };
                        let mut arena = ExprArena::new();
                        let b =
                            self.with_binder(ctx, &mut arena, |b| b.bind_assigned(tree, v, &rt));
                        let Some(e) = b.and_then(|e| self.conv(&arena, e, ctx)) else {
                            return;
                        };
                        coerce(e, ret)
                    }
                    None => Expr::Local {
                        name: "_ret".into(),
                        ty: ret,
                    },
                };
                out.push(Stmt::Return(e));
            }
            other => {
                let loc = self.node_loc(ctx, tree, node);
                self.err(
                    loc,
                    format!("statement kind {other:?} is not in the synthesizable subset"),
                );
            }
        }
    }

    fn cond(&mut self, tree: usize, node: NodeId, ctx: &Ctx) -> Option<Expr> {
        let mut arena = ExprArena::new();
        let b = self.with_binder(ctx, &mut arena, |b| b.bind_condition(tree, node))?;
        Some(truth(self.conv(&arena, b, ctx)?))
    }

    fn new_local(&mut self, ctx: &mut Ctx, name: &str, rty: RtlType, ty: Ty) -> String {
        let id = LocalId(self.next_local);
        self.next_local += 1;
        let mut cname = sanitize(name);
        let mut n = 1;
        while !ctx.used.insert(cname.clone()) || self.m.used.contains(&cname) {
            cname = format!("{}_{}", sanitize(name), n);
            n += 1;
        }
        ctx.locals.push((
            name.to_string(),
            LocalBinding {
                id,
                ty: rty,
                packed_bounds: None,
                cval: None,
            },
        ));
        ctx.local_names.insert(id.0, (cname.clone(), ty));
        ctx.decls.push(LocalVar {
            name: cname.clone(),
            ty,
        });
        cname
    }

    fn local_decl(&mut self, tree: usize, node: NodeId, ctx: &mut Ctx, out: &mut Vec<Stmt>) {
        let t = &self.trees[tree];
        let own = own_toks(t, node);
        if own
            .first()
            .is_some_and(|k| matches!(*k, "input" | "output" | "inout" | "ref"))
        {
            return;
        }
        let dt = t.child(node, SyntaxKind::DataType);
        let decls: Vec<NodeId> = t
            .children(node)
            .filter(|&c| t.kind(c) == SyntaxKind::Declarator)
            .collect();
        for d in decls {
            let Some(name) = t
                .child(d, SyntaxKind::Name)
                .map(|n| t.bare_text(n).trim().to_string())
            else {
                continue;
            };
            let loc = self.node_loc(ctx, tree, d);
            let (inst, prefix) = (ctx.inst, ctx.prefix.clone());
            let mut arena = ExprArena::new();
            let rty = self.with_binder(ctx, &mut arena, |b| {
                b.resolve_subroutine_dtype(tree, dt, Some(d), Some(inst), &prefix)
            });
            let Some(rty) = rty else { continue };
            let Some(ty) = ity(&rty) else {
                self.err(
                    loc,
                    format!("local variable '{name}' must have a packed integral type"),
                );
                continue;
            };
            let init = t.children(d).find(|&c| {
                !matches!(
                    t.kind(c),
                    SyntaxKind::Name | SyntaxKind::Dim | SyntaxKind::Attribute
                )
            });
            // The initializer is bound before the new name becomes visible.
            let init_e = match init {
                Some(n) => {
                    let mut a2 = ExprArena::new();
                    let b = self.with_binder(ctx, &mut a2, |b| b.bind_assigned(tree, n, &rty));
                    b.and_then(|e| self.conv(&a2, e, ctx))
                }
                None => None,
            };
            let cname = self.new_local(ctx, &name, rty, ty);
            if let Some(e) = init_e {
                out.push(Stmt::Assign {
                    lhs: Lval {
                        base: LvBase::Local { name: cname, ty },
                        sel: None,
                    },
                    rhs: coerce(e, ty),
                    nonblocking: false,
                });
            }
        }
    }

    fn case_stmt(&mut self, tree: usize, node: NodeId, ctx: &mut Ctx, out: &mut Vec<Stmt>) {
        let t = &self.trees[tree];
        let own = own_toks(t, node);
        let loc = self.node_loc(ctx, tree, node);
        if own.contains(&"randcase") || own.contains(&"matches") || own.contains(&"inside") {
            self.err(
                loc,
                "'case inside', pattern-matching case and randcase are not supported",
            );
            return;
        }
        let is_z = own.contains(&"casez");
        let is_x = own.contains(&"casex");

        // Split children into selector, item patterns and bodies.
        let mut sel_node = None;
        let mut branches: Vec<(Vec<NodeId>, NodeId)> = Vec::new();
        let mut default_stmt = None;
        let mut cur: Vec<NodeId> = Vec::new();
        let (mut in_default, mut after_colon) = (false, false);
        for ch in &t.node(node).children {
            match *ch {
                Child::Token(ti) => {
                    let tok = &t.tokens[ti as usize];
                    if tok.kind.is_trivia() {
                        continue;
                    }
                    match &*tok.text {
                        "default" => {
                            in_default = true;
                            after_colon = true;
                        }
                        ":" => after_colon = true,
                        _ => {}
                    }
                }
                Child::Node(n) => {
                    if matches!(t.kind(n), SyntaxKind::Attribute | SyntaxKind::StmtLabel) {
                        continue;
                    }
                    if sel_node.is_none() {
                        sel_node = Some(n);
                    } else if after_colon {
                        if in_default {
                            default_stmt = Some(n);
                        } else {
                            branches.push((std::mem::take(&mut cur), n));
                        }
                        in_default = false;
                        after_colon = false;
                    } else {
                        cur.push(n);
                    }
                }
            }
        }
        let Some(sel_node) = sel_node else { return };

        // IEEE 1800-2023 12.5: selector and all item expressions are sized to the widest,
        // and are signed only when all of them are.
        let mut arena = ExprArena::new();
        let shape = self.with_binder(ctx, &mut arena, |b| {
            let (mut w, mut s, _) = b.peek_self_shape(tree, sel_node)?;
            for (pats, _) in &branches {
                for &p in pats {
                    if let Some((pw, ps, _)) = b.peek_self_shape(tree, p) {
                        w = w.max(pw);
                        s = s && ps;
                    }
                }
            }
            Some((w, s))
        });
        let Some((cw, cs)) = shape else {
            self.err(loc, "case selector must be a packed integral expression");
            return;
        };
        let cty = Ty::new(cw, cs);
        let sel_b = self.with_binder(ctx, &mut arena, |b| {
            b.bind_with_context(tree, sel_node, cw, cs)
        });
        let Some(sel) = sel_b.and_then(|e| self.conv(&arena, e, ctx)) else {
            return;
        };
        let sel = coerce(sel, cty);

        let mut arms = Vec::new();
        for (pats, body_n) in branches {
            let mut ps = Vec::new();
            for p in pats {
                let mut a2 = ExprArena::new();
                let pb = self.with_binder(ctx, &mut a2, |b| b.bind_with_context(tree, p, cw, cs));
                let Some(pb) = pb else { continue };
                if let Some(cv) = a2.get(pb).cval.clone() {
                    let cv = if cv.w != cw { cv.cast(cw, cv.s) } else { cv };
                    let unk = unknown_words(&cv, cw);
                    let z = z_words(&cv, cw);
                    let wild: Vec<u64> = if is_x {
                        unk.clone()
                    } else if is_z {
                        z.clone()
                    } else {
                        vec![0; unk.len()]
                    };
                    // An unknown bit that is not a wildcard can never match a two-state value.
                    if unk.iter().zip(&wild).any(|(u, w)| u & !w != 0) {
                        continue;
                    }
                    let care: Vec<u64> = ones_words(cw)
                        .iter()
                        .zip(&wild)
                        .map(|(o, w)| o & !w)
                        .collect();
                    let value: Vec<u64> = const_words(&cv, cw)
                        .iter()
                        .zip(&care)
                        .map(|(v, c)| v & c)
                        .collect();
                    ps.push(Pat::Const { value, care });
                } else if let Some(e) = self.conv(&a2, pb, ctx) {
                    ps.push(Pat::Expr(coerce(e, cty)));
                }
            }
            let mut body = Vec::new();
            let saved = ctx.locals.len();
            self.stmt(tree, body_n, ctx, &mut body);
            ctx.locals.truncate(saved);
            if !ps.is_empty() {
                arms.push(CaseArm { pats: ps, body });
            }
        }
        let default = default_stmt.map(|d| {
            let mut body = Vec::new();
            let saved = ctx.locals.len();
            self.stmt(tree, d, ctx, &mut body);
            ctx.locals.truncate(saved);
            body
        });
        out.push(Stmt::Case { sel, arms, default });
    }

    fn loop_stmt(&mut self, tree: usize, node: NodeId, ctx: &mut Ctx, out: &mut Vec<Stmt>) {
        let t = &self.trees[tree];
        let own = own_toks(t, node);
        let kw = own.first().copied().unwrap_or("");
        let loc = self.node_loc(ctx, tree, node);
        let saved = ctx.locals.len();
        match kw {
            "for" => {
                let mut inits = Vec::new();
                let mut cond = None;
                let mut steps = Vec::new();
                let mut body = None;
                let (mut semis, mut depth, mut past) = (0usize, 0i32, false);
                for ch in &t.node(node).children {
                    match *ch {
                        Child::Token(ti) => {
                            let tok = &t.tokens[ti as usize];
                            if tok.kind.is_trivia() {
                                continue;
                            }
                            match &*tok.text {
                                "(" => depth += 1,
                                ")" => {
                                    depth -= 1;
                                    if depth <= 0 {
                                        past = true;
                                    }
                                }
                                ";" if !past => semis += 1,
                                _ => {}
                            }
                        }
                        Child::Node(n) => {
                            if matches!(t.kind(n), SyntaxKind::Attribute | SyntaxKind::StmtLabel) {
                                continue;
                            }
                            if past {
                                body = Some(n);
                            } else if semis == 0 {
                                inits.push(n);
                            } else if semis == 1 {
                                cond = Some(n);
                            } else {
                                steps.push(n);
                            }
                        }
                    }
                }
                let mut init_s = Vec::new();
                for i in inits {
                    if matches!(t.kind(i), SyntaxKind::DataDecl | SyntaxKind::ParamDecl) {
                        self.local_decl(tree, i, ctx, &mut init_s);
                    } else {
                        self.expr_stmt(tree, i, ctx, &mut init_s);
                    }
                }
                let cond_e = match cond {
                    Some(c) => self.cond(tree, c, ctx),
                    None => Some(Expr::konst(Ty::BIT, 1)),
                };
                let mut step_s = Vec::new();
                for s in steps {
                    self.expr_stmt(tree, s, ctx, &mut step_s);
                }
                let mut body_s = Vec::new();
                if let Some(b) = body {
                    self.stmt(tree, b, ctx, &mut body_s);
                }
                if let Some(cond) = cond_e {
                    out.push(Stmt::For {
                        init: init_s,
                        cond,
                        step: step_s,
                        body: body_s,
                    });
                }
            }
            "while" => {
                let kids: Vec<NodeId> = t
                    .children(node)
                    .filter(|&c| {
                        !matches!(t.kind(c), SyntaxKind::Attribute | SyntaxKind::StmtLabel)
                    })
                    .collect();
                let (Some(&c), Some(&b)) = (kids.first(), kids.get(1)) else {
                    return;
                };
                let Some(cond) = self.cond(tree, c, ctx) else {
                    return;
                };
                let mut body_s = Vec::new();
                self.stmt(tree, b, ctx, &mut body_s);
                out.push(Stmt::For {
                    init: Vec::new(),
                    cond,
                    step: Vec::new(),
                    body: body_s,
                });
            }
            _ => self.err(
                loc,
                format!(
                    "loop kind '{kw}' is not supported (use a 'for' loop with constant bounds)"
                ),
            ),
        }
        ctx.locals.truncate(saved);
    }

    fn expr_stmt(&mut self, tree: usize, e: NodeId, ctx: &mut Ctx, out: &mut Vec<Stmt>) {
        let t = &self.trees[tree];
        let op = own_toks(t, e).first().copied().unwrap_or("");
        match t.kind(e) {
            SyntaxKind::BinaryExpr => {
                let kids: Vec<NodeId> = t
                    .children(e)
                    .filter(|&c| t.kind(c) != SyntaxKind::Attribute)
                    .collect();
                if kids.len() == 2 && is_assign_op(op) {
                    self.assignment(tree, kids[0], op, Some(kids[1]), ctx, out);
                    return;
                }
            }
            SyntaxKind::PrefixExpr | SyntaxKind::PostfixExpr if matches!(op, "++" | "--") => {
                if let Some(target) = t.children(e).find(|&c| t.kind(c) != SyntaxKind::Attribute) {
                    let aop = if op == "++" { "+=" } else { "-=" };
                    self.assignment(tree, target, aop, None, ctx, out);
                    return;
                }
            }
            SyntaxKind::CallExpr => {
                let txt = t.bare_text(e);
                if txt.trim_start().starts_with('$') {
                    let loc = self.node_loc(ctx, tree, e);
                    self.note(loc, "system task call skipped");
                    return;
                }
            }
            _ => {}
        }
        let loc = self.node_loc(ctx, tree, e);
        self.err(
            loc,
            "expression statement is not an assignment (task calls are not supported)",
        );
    }

    fn assignment(
        &mut self,
        tree: usize,
        lhs_n: NodeId,
        op: &str,
        rhs_n: Option<NodeId>,
        ctx: &mut Ctx,
        out: &mut Vec<Stmt>,
    ) {
        let loc = self.node_loc(ctx, tree, lhs_n);
        let nb = op == "<=";
        if nb && !ctx.allow_nba {
            self.err(
                loc,
                "nonblocking assignment outside an edge-triggered block",
            );
            return;
        }
        let mut arena = ExprArena::new();
        let Some(lhs_b) = self.with_binder(ctx, &mut arena, |b| b.bind_self(tree, lhs_n)) else {
            return;
        };
        let lhs_rty = arena.get(lhs_b).ty.clone();
        let Some(lty) = ity(&lhs_rty) else {
            self.err(
                loc,
                "assignment target must be a packed integral value (whole-array and unpacked struct assignments are not supported)",
            );
            return;
        };
        let Some(lvs) = self.lvals(&arena, lhs_b, ctx) else {
            return;
        };

        let rhs = if op == "=" || op == "<=" {
            let Some(rn) = rhs_n else { return };
            let b = self.with_binder(ctx, &mut arena, |b| b.bind_assigned(tree, rn, &lhs_rty));
            let Some(e) = b.and_then(|e| self.conv(&arena, e, ctx)) else {
                return;
            };
            coerce(e, lty)
        } else {
            // Compound assignment `a op= b` is `a = a op b`.
            if lvs.len() != 1 {
                self.err(loc, "compound assignment to a concatenation");
                return;
            }
            let Some(cur) = self.conv(&arena, lhs_b, ctx) else {
                return;
            };
            let bop = match op {
                "+=" => BinOp::Add,
                "-=" => BinOp::Sub,
                "*=" => BinOp::Mul,
                "/=" => BinOp::Div,
                "%=" => BinOp::Mod,
                "&=" => BinOp::BitAnd,
                "|=" => BinOp::BitOr,
                "^=" => BinOp::BitXor,
                "<<=" | "<<<=" => BinOp::Shl,
                ">>=" => BinOp::LShr,
                ">>>=" => BinOp::AShr,
                _ => {
                    self.err(loc, format!("assignment operator '{op}' is not supported"));
                    return;
                }
            };
            match rhs_n {
                None => {
                    let one = Expr::konst(lty, 1);
                    bin(bop, cur, one, lty)
                }
                Some(rn) if matches!(bop, BinOp::Shl | BinOp::LShr | BinOp::AShr) => {
                    let b = self.with_binder(ctx, &mut arena, |b| b.bind_self(tree, rn));
                    let Some(r) = b.and_then(|e| self.conv(&arena, e, ctx)) else {
                        return;
                    };
                    let rt = r.ty();
                    bin(bop, cur, coerce(r, Ty::unsigned(rt.w)), lty)
                }
                Some(rn) => {
                    let shape = self.with_binder(ctx, &mut arena, |b| b.peek_self_shape(tree, rn));
                    let Some((rw, rs, _)) = shape else { return };
                    let cty = Ty::new(lty.w.max(rw), lty.s && rs);
                    let b = self.with_binder(ctx, &mut arena, |b| {
                        b.bind_with_context(tree, rn, cty.w, cty.s)
                    });
                    let Some(r) = b.and_then(|e| self.conv(&arena, e, ctx)) else {
                        return;
                    };
                    coerce(bin(bop, coerce(cur, cty), coerce(r, cty), cty), lty)
                }
            }
        };
        self.assign_lvals(lvs, rhs, nb, out, Some(ctx));
    }

    /// Emits the assignment of `rhs` to one target, or to the parts of a concatenation.
    fn assign_lvals(
        &mut self,
        lvs: Vec<Lval>,
        rhs: Expr,
        nb: bool,
        out: &mut Vec<Stmt>,
        ctx: Option<&mut Ctx>,
    ) {
        let width = |lv: &Lval| lv.sel.as_ref().map(|s| s.w).unwrap_or(lv.base.ty().w);
        let target_ty = |lv: &Lval| match &lv.sel {
            Some(s) => Ty::unsigned(s.w),
            None => lv.base.ty(),
        };
        if lvs.len() == 1 {
            let lv = lvs.into_iter().next().expect("one lvalue");
            let tt = target_ty(&lv);
            let nonblocking = nb && !matches!(lv.base, LvBase::Local { .. });
            out.push(Stmt::Assign {
                rhs: coerce(rhs, tt),
                lhs: lv,
                nonblocking,
            });
            return;
        }
        let total: u32 = lvs.iter().map(width).sum();
        let tty = Ty::unsigned(total);
        let rhs = coerce(rhs, tty);
        // Evaluate the right-hand side once, then distribute its slices.
        let src = match (&rhs, ctx) {
            (Expr::Sig { .. } | Expr::Local { .. } | Expr::Const { .. }, _) | (_, None) => rhs,
            (_, Some(ctx)) => {
                let rty = RtlType::Integral {
                    width: total,
                    signed: false,
                    four_state: true,
                };
                let name = self.new_local(ctx, "_cat", rty, tty);
                // The temporary is not a source-level name.
                ctx.locals.pop();
                out.push(Stmt::Assign {
                    lhs: Lval {
                        base: LvBase::Local {
                            name: name.clone(),
                            ty: tty,
                        },
                        sel: None,
                    },
                    rhs,
                    nonblocking: false,
                });
                Expr::Local { name, ty: tty }
            }
        };
        let mut hi = total;
        for lv in lvs {
            let w = width(&lv);
            let lo = hi - w;
            let part = Expr::Slice {
                base: Box::new(src.clone()),
                lo,
                lo_dyn: None,
                ty: Ty::unsigned(w),
            };
            let tt = target_ty(&lv);
            let nonblocking = nb && !matches!(lv.base, LvBase::Local { .. });
            out.push(Stmt::Assign {
                rhs: coerce(part, tt),
                lhs: lv,
                nonblocking,
            });
            hi = lo;
        }
    }

    // ------------------------------------------------------------------
    // Lvalues
    // ------------------------------------------------------------------

    fn lvals(&mut self, a: &ExprArena, id: ExprId, ctx: &Ctx) -> Option<Vec<Lval>> {
        if let ExprKind::Concat(parts) = &a.get(id).kind {
            let mut out = Vec::new();
            for p in parts {
                out.extend(self.lvals(a, *p, ctx)?);
            }
            return Some(out);
        }
        Some(vec![self.lval(a, id, ctx)?])
    }

    fn lval(&mut self, a: &ExprArena, id: ExprId, ctx: &Ctx) -> Option<Lval> {
        let node = a.get(id);
        let loc = node.loc;
        match &node.kind {
            ExprKind::Symbol(sid) => {
                let name = self.sym_name(*sid, ctx, loc)?;
                let info = self.m.sigs.get(&name)?.clone();
                if info.is_input {
                    self.err(loc, format!("assignment to input port '{name}'"));
                    return None;
                }
                if !info.dims.is_empty() {
                    self.err(
                        loc,
                        format!("assignment to the whole array '{name}' is not supported"),
                    );
                    return None;
                }
                Some(Lval {
                    base: LvBase::Sig { name, ty: info.ty },
                    sel: None,
                })
            }
            ExprKind::Local(lid) => {
                let (name, ty) = ctx.local_names.get(&lid.0).cloned().or_else(|| {
                    self.err(loc, "assignment to an unknown local variable");
                    None
                })?;
                Some(Lval {
                    base: LvBase::Local { name, ty },
                    sel: None,
                })
            }
            ExprKind::ArrayIndex { .. } => {
                let (name, idx, ty) = self.array_elem(a, id, ctx)?;
                Some(Lval {
                    base: LvBase::Elem { name, idx, ty },
                    sel: None,
                })
            }
            ExprKind::BitSelect {
                base,
                index,
                bounds,
                elem_width,
            } => {
                let p = self.lval(a, *base, ctx)?;
                let pw = a.get(*base).ty.bit_width().unwrap_or(1);
                let pos = self.norm_index(a, *index, *bounds, *elem_width, 0, pw, ctx)?;
                match pos {
                    Some(pos) => Some(compose(p, pos, *elem_width)),
                    None => {
                        self.err(loc, "constant index is out of range");
                        None
                    }
                }
            }
            ExprKind::PartSelect {
                base,
                lsb,
                bounds,
                elem_width,
                ..
            } => {
                let p = self.lval(a, *base, ctx)?;
                let w = node.ty.bit_width().unwrap_or(1);
                let pw = a.get(*base).ty.bit_width().unwrap_or(1);
                let Some(lo) = const_offset(*lsb, *bounds, *elem_width).filter(|lo| lo + w <= pw)
                else {
                    self.err(loc, "part select is out of range");
                    return None;
                };
                Some(compose(p, Pos { lo, lo_dyn: None }, w))
            }
            ExprKind::IndexedPartSelect {
                base,
                base_idx,
                width,
                ascending,
                bounds,
                elem_width,
            } => {
                let p = self.lval(a, *base, ctx)?;
                let pw = a.get(*base).ty.bit_width().unwrap_or(1);
                let adj = indexed_adjust(*bounds, *width, *ascending);
                let w = width * elem_width;
                let pos = self.norm_index(a, *base_idx, *bounds, *elem_width, adj, pw, ctx)?;
                match pos {
                    Some(pos) => Some(compose(p, pos, w)),
                    None => {
                        self.err(loc, "constant part select is out of range");
                        None
                    }
                }
            }
            ExprKind::Member {
                base,
                packed_slice: Some((lo, w)),
                ..
            } => {
                let p = self.lval(a, *base, ctx)?;
                Some(compose(
                    p,
                    Pos {
                        lo: *lo,
                        lo_dyn: None,
                    },
                    *w,
                ))
            }
            ExprKind::CastFourState { arg, .. } => self.lval(a, *arg, ctx),
            _ => {
                self.err(loc, "unsupported assignment target");
                None
            }
        }
    }

    fn sym_name(&mut self, sid: SymbolId, ctx: &Ctx, loc: Loc) -> Option<String> {
        let sym = self.design.symbol(sid);
        if sym.instance != ctx.inst {
            self.err(
                loc,
                format!(
                    "hierarchical reference to '{}' is not supported",
                    sym.full_path
                ),
            );
            return None;
        }
        match self.m.sym_names.get(&sid) {
            Some(n) => Some(n.clone()),
            None => {
                self.err(
                    loc,
                    format!("'{}' is not a signal that can be used here", sym.name),
                );
                None
            }
        }
    }

    /// Resolves an unpacked array element reference to `(array name, element offsets, element type)`.
    fn array_elem(
        &mut self,
        a: &ExprArena,
        id: ExprId,
        ctx: &Ctx,
    ) -> Option<(String, Vec<Expr>, Ty)> {
        let loc = a.get(id).loc;
        let mut chain = Vec::new();
        let mut cur = id;
        let sid = loop {
            match &a.get(cur).kind {
                ExprKind::ArrayIndex { base, index, dim } => {
                    chain.push((*index, *dim));
                    cur = *base;
                }
                ExprKind::Symbol(s) => break *s,
                _ => {
                    self.err(
                        loc,
                        "indexing of local or nested unpacked arrays is not supported",
                    );
                    return None;
                }
            }
        };
        chain.reverse();
        let name = self.sym_name(sid, ctx, loc)?;
        let info = self.m.sigs.get(&name)?.clone();
        if chain.len() != info.dims.len() {
            self.err(
                loc,
                format!("array '{name}' must be indexed down to one element"),
            );
            return None;
        }
        let mut idx = Vec::new();
        for (ix, (l, r)) in chain {
            let low = l.min(r);
            let e = self.conv(a, ix, ctx)?;
            let off = if let Some(cv) = &a.get(ix).cval {
                let v = cv.i() - low;
                Expr::konst(Ty::INT, v.clamp(-1, i32::MAX as i128) as u64)
            } else if low == 0 {
                e
            } else {
                let it = index_ty(&e);
                bin(BinOp::Sub, coerce(e, it), Expr::konst(it, low as u64), it)
            };
            idx.push(off);
        }
        Some((name, idx, info.ty))
    }

    /// Normalizes a bit or element index to a zero-based bit position inside the base.
    /// Outer `None` is an error; inner `None` means a constant index outside the base.
    #[allow(clippy::too_many_arguments)]
    fn norm_index(
        &mut self,
        a: &ExprArena,
        index: ExprId,
        bounds: (i128, i128),
        elem_w: u32,
        adj: i128,
        base_w: u32,
        ctx: &Ctx,
    ) -> Option<Option<Pos>> {
        let (left, right) = bounds;
        let desc = left >= right;
        if let Some(cv) = &a.get(index).cval {
            if !cv.known() {
                return Some(None);
            }
            let i = cv.i() + adj;
            let rel = if desc { i - right } else { right - i };
            if rel < 0 {
                return Some(None);
            }
            let lo = rel * elem_w as i128;
            if lo + elem_w as i128 > base_w as i128 {
                return Some(None);
            }
            return Some(Some(Pos {
                lo: lo as u32,
                lo_dyn: None,
            }));
        }
        let e = self.conv(a, index, ctx)?;
        let c = if desc { adj - right } else { right - adj };
        let dynamic = if desc && c == 0 && elem_w == 1 {
            e
        } else {
            let it = index_ty(&e);
            let i = coerce(e, it);
            let k = |v: i128| Expr::konst(it, v as u64);
            let rel = if desc {
                if c == 0 {
                    i
                } else {
                    bin(BinOp::Add, i, k(c), it)
                }
            } else {
                bin(BinOp::Sub, k(c), i, it)
            };
            if elem_w == 1 {
                rel
            } else {
                bin(BinOp::Mul, rel, k(elem_w as i128), it)
            }
        };
        Some(Some(Pos {
            lo: 0,
            lo_dyn: Some(dynamic),
        }))
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn conv(&mut self, a: &ExprArena, id: ExprId, ctx: &Ctx) -> Option<Expr> {
        let node = a.get(id);
        let loc = node.loc;
        if matches!(node.ty, RtlType::UnpackedArray { .. }) {
            self.err(loc, "whole unpacked arrays cannot be used in an expression");
            return None;
        }
        let Some(ty) = ity(&node.ty) else {
            self.err(loc, "expression is not a packed integral value");
            return None;
        };
        if let Some(cv) = &node.cval {
            if !matches!(node.kind, ExprKind::UnbasedUnsized(_)) {
                return Some(const_expr(cv, ty));
            }
        }
        match &node.kind {
            ExprKind::Const(cv) => Some(const_expr(cv, ty)),
            ExprKind::UnbasedUnsized(_) => {
                self.err(loc, "unsized literal in a context without a width");
                None
            }
            ExprKind::Symbol(sid) => {
                let sym = self.design.symbol(*sid);
                if matches!(sym.kind, SymbolKind::Param { .. } | SymbolKind::Genvar) {
                    self.err(
                        loc,
                        format!("parameter '{}' has no constant value here", sym.name),
                    );
                    return None;
                }
                let name = self.sym_name(*sid, ctx, loc)?;
                Some(Expr::Sig { name, ty })
            }
            ExprKind::Local(lid) => match ctx.local_names.get(&lid.0) {
                Some((name, lty)) => Some(coerce(
                    Expr::Local {
                        name: name.clone(),
                        ty: *lty,
                    },
                    ty,
                )),
                None => {
                    self.err(loc, "reference to an unknown local variable");
                    None
                }
            },
            ExprKind::Unary { op, arg } => {
                let x = self.conv(a, *arg, ctx)?;
                let un = |op, x: Expr, ty| Expr::Unary {
                    op,
                    arg: Box::new(x),
                    ty,
                };
                Some(match op {
                    UnaryOp::Plus => coerce(x, ty),
                    UnaryOp::Neg => un(UnOp::Neg, coerce(x, ty), ty),
                    UnaryOp::BitNot => un(UnOp::BitNot, coerce(x, ty), ty),
                    UnaryOp::LogNot => coerce(lnot(truth(x)), ty),
                    UnaryOp::RedAnd => coerce(un(UnOp::RedAnd, x, Ty::BIT), ty),
                    UnaryOp::RedOr => coerce(un(UnOp::RedOr, x, Ty::BIT), ty),
                    UnaryOp::RedXor => coerce(un(UnOp::RedXor, x, Ty::BIT), ty),
                    UnaryOp::RedNand => coerce(lnot(un(UnOp::RedAnd, x, Ty::BIT)), ty),
                    UnaryOp::RedNor => coerce(lnot(un(UnOp::RedOr, x, Ty::BIT)), ty),
                    UnaryOp::RedXnor => coerce(lnot(un(UnOp::RedXor, x, Ty::BIT)), ty),
                })
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.conv(a, *lhs, ctx)?;
                let r = self.conv(a, *rhs, ctx)?;
                use BinaryOp as B;
                Some(match op {
                    B::Add
                    | B::Sub
                    | B::Mul
                    | B::Div
                    | B::Mod
                    | B::BitAnd
                    | B::BitOr
                    | B::BitXor
                    | B::BitXnor => {
                        let o = match op {
                            B::Add => BinOp::Add,
                            B::Sub => BinOp::Sub,
                            B::Mul => BinOp::Mul,
                            B::Div => BinOp::Div,
                            B::Mod => BinOp::Mod,
                            B::BitAnd => BinOp::BitAnd,
                            B::BitOr => BinOp::BitOr,
                            B::BitXor => BinOp::BitXor,
                            _ => BinOp::BitXnor,
                        };
                        bin(o, coerce(l, ty), coerce(r, ty), ty)
                    }
                    B::Shl | B::LShr | B::AShr => {
                        let o = match op {
                            B::Shl => BinOp::Shl,
                            B::LShr => BinOp::LShr,
                            _ => BinOp::AShr,
                        };
                        let rt = r.ty();
                        bin(o, coerce(l, ty), coerce(r, Ty::unsigned(rt.w)), ty)
                    }
                    B::Eq
                    | B::Neq
                    | B::CaseEq
                    | B::CaseNeq
                    | B::WildEq
                    | B::WildNeq
                    | B::Lt
                    | B::Le
                    | B::Gt
                    | B::Ge => {
                        let (lt, rt) = (l.ty(), r.ty());
                        let ct = Ty::new(lt.w.max(rt.w), lt.s && rt.s);
                        let negate = matches!(op, B::WildNeq);
                        let o = match op {
                            B::Eq | B::CaseEq | B::WildEq | B::WildNeq => BinOp::Eq,
                            B::Neq | B::CaseNeq => BinOp::Ne,
                            B::Lt => BinOp::Lt,
                            B::Le => BinOp::Le,
                            B::Gt => BinOp::Gt,
                            _ => BinOp::Ge,
                        };
                        let mut lhs_e = coerce(l, ct);
                        let mut rhs_e = coerce(r, ct);
                        if matches!(op, B::WildEq | B::WildNeq) {
                            if let Some(cv) = a.get(*rhs).cval.as_ref().filter(|c| !c.known()) {
                                let cv = cv.cast(ct.w, cv.s);
                                let care: Vec<u64> = ones_words(ct.w)
                                    .iter()
                                    .zip(unknown_words(&cv, ct.w))
                                    .map(|(o, u)| o & !u)
                                    .collect();
                                let mask = Expr::Const {
                                    ty: ct,
                                    words: care,
                                };
                                lhs_e = bin(BinOp::BitAnd, lhs_e, mask, ct);
                                rhs_e = Expr::Const {
                                    ty: ct,
                                    words: const_words(&cv, ct.w),
                                };
                            }
                        }
                        let cmp = bin(o, lhs_e, rhs_e, Ty::BIT);
                        coerce(if negate { lnot(cmp) } else { cmp }, ty)
                    }
                    B::LogAnd => coerce(bin(BinOp::LogAnd, truth(l), truth(r), Ty::BIT), ty),
                    B::LogOr => coerce(bin(BinOp::LogOr, truth(l), truth(r), Ty::BIT), ty),
                    B::LogImp => coerce(bin(BinOp::LogOr, lnot(truth(l)), truth(r), Ty::BIT), ty),
                    B::LogEqv => coerce(bin(BinOp::Eq, truth(l), truth(r), Ty::BIT), ty),
                    B::Pow => {
                        self.err(loc, "'**' with a non-constant operand is not supported");
                        return None;
                    }
                })
            }
            ExprKind::Cond {
                cond,
                then_val,
                else_val,
            } => {
                let c = truth(self.conv(a, *cond, ctx)?);
                let t = coerce(self.conv(a, *then_val, ctx)?, ty);
                let e = coerce(self.conv(a, *else_val, ctx)?, ty);
                Some(Expr::Cond {
                    cond: Box::new(c),
                    then_e: Box::new(t),
                    else_e: Box::new(e),
                    ty,
                })
            }
            ExprKind::Inside { expr, items } => {
                let x = self.conv(a, *expr, ctx)?;
                let mut acc: Option<Expr> = None;
                for it in items {
                    let term = match it {
                        InsideItem::Single(i) => {
                            let v = self.conv(a, *i, ctx)?;
                            let ct = Ty::new(x.ty().w.max(v.ty().w), x.ty().s && v.ty().s);
                            bin(BinOp::Eq, coerce(x.clone(), ct), coerce(v, ct), Ty::BIT)
                        }
                        InsideItem::Range(lo, hi) => {
                            let l = self.conv(a, *lo, ctx)?;
                            let h = self.conv(a, *hi, ctx)?;
                            let ct = Ty::new(
                                x.ty().w.max(l.ty().w).max(h.ty().w),
                                x.ty().s && l.ty().s && h.ty().s,
                            );
                            bin(
                                BinOp::LogAnd,
                                bin(BinOp::Ge, coerce(x.clone(), ct), coerce(l, ct), Ty::BIT),
                                bin(BinOp::Le, coerce(x.clone(), ct), coerce(h, ct), Ty::BIT),
                                Ty::BIT,
                            )
                        }
                    };
                    acc = Some(match acc {
                        None => term,
                        Some(p) => bin(BinOp::LogOr, p, term, Ty::BIT),
                    });
                }
                Some(coerce(acc.unwrap_or(Expr::konst(Ty::BIT, 0)), ty))
            }
            ExprKind::Concat(parts) => {
                let mut ps = Vec::new();
                for p in parts {
                    ps.push(self.conv(a, *p, ctx)?);
                }
                let total: u32 = ps.iter().map(|p| p.ty().w).sum();
                if ps.len() == 1 {
                    let only = ps.pop().expect("one part");
                    return Some(coerce(coerce(only, Ty::unsigned(total)), ty));
                }
                Some(coerce(
                    Expr::Concat {
                        parts: ps,
                        ty: Ty::unsigned(total),
                    },
                    ty,
                ))
            }
            ExprKind::Replicate { count, expr } => {
                let x = self.conv(a, *expr, ctx)?;
                let total = x.ty().w * count;
                if *count == 0 {
                    self.err(loc, "zero replication is not supported here");
                    return None;
                }
                Some(coerce(
                    Expr::Repl {
                        count: *count,
                        arg: Box::new(x),
                        ty: Ty::unsigned(total),
                    },
                    ty,
                ))
            }
            ExprKind::Member {
                base,
                packed_slice: Some((lo, w)),
                ..
            } => {
                let b = self.conv(a, *base, ctx)?;
                Some(coerce(
                    Expr::Slice {
                        base: Box::new(b),
                        lo: *lo,
                        lo_dyn: None,
                        ty: Ty::new(*w, ty.s && *w == ty.w),
                    },
                    ty,
                ))
            }
            ExprKind::Member { .. } => {
                self.err(loc, "unpacked struct members are not supported");
                None
            }
            ExprKind::BitSelect {
                base,
                index,
                bounds,
                elem_width,
            } => {
                let b = self.conv(a, *base, ctx)?;
                let pos = self.norm_index(a, *index, *bounds, *elem_width, 0, b.ty().w, ctx)?;
                Some(slice_of(b, pos, *elem_width, ty))
            }
            ExprKind::PartSelect {
                base,
                lsb,
                bounds,
                elem_width,
                ..
            } => {
                let b = self.conv(a, *base, ctx)?;
                let lo =
                    const_offset(*lsb, *bounds, *elem_width).filter(|lo| lo + ty.w <= b.ty().w);
                Some(slice_of(b, lo.map(|lo| Pos { lo, lo_dyn: None }), ty.w, ty))
            }
            ExprKind::IndexedPartSelect {
                base,
                base_idx,
                width,
                ascending,
                bounds,
                elem_width,
            } => {
                let b = self.conv(a, *base, ctx)?;
                let adj = indexed_adjust(*bounds, *width, *ascending);
                let pos =
                    self.norm_index(a, *base_idx, *bounds, *elem_width, adj, b.ty().w, ctx)?;
                Some(slice_of(b, pos, width * elem_width, ty))
            }
            ExprKind::ArrayIndex { base, index, dim } => {
                if let Some(items) = self.const_table(a, *base, ctx) {
                    // Lookup in a constant table (unpacked parameter): a selection chain.
                    let (arena2, elems) = items;
                    let ix = self.conv(a, *index, ctx)?;
                    let it = index_ty(&ix);
                    let ix = coerce(ix, it);
                    let left = dim.0;
                    let step: i128 = if dim.0 <= dim.1 { 1 } else { -1 };
                    let mut acc = Expr::konst(ty, 0);
                    for (k, el) in elems.iter().enumerate().rev() {
                        let v = coerce(self.conv(&arena2, *el, ctx)?, ty);
                        let at = left + step * k as i128;
                        let test = bin(BinOp::Eq, ix.clone(), Expr::konst(it, at as u64), Ty::BIT);
                        acc = Expr::Cond {
                            cond: Box::new(test),
                            then_e: Box::new(v),
                            else_e: Box::new(acc),
                            ty,
                        };
                    }
                    return Some(acc);
                }
                let (name, idx, ety) = self.array_elem(a, id, ctx)?;
                Some(coerce(Expr::Elem { name, idx, ty: ety }, ty))
            }
            ExprKind::Resize { arg, width, signed } => {
                let mut x = self.conv(a, *arg, ctx)?;
                if x.ty().s != *signed {
                    let w = x.ty().w;
                    x = coerce(x, Ty::new(w, *signed));
                }
                Some(coerce(coerce(x, Ty::new(*width, *signed)), ty))
            }
            ExprKind::CastSign { arg, .. } | ExprKind::CastFourState { arg, .. } => {
                let x = self.conv(a, *arg, ctx)?;
                let w = x.ty().w;
                // Same bits, new signedness; a width change here would be a binder bug.
                let x = if w == ty.w {
                    x
                } else {
                    coerce(x, Ty::new(ty.w, false))
                };
                Some(match x {
                    Expr::Const { words, .. } => Expr::Const { ty, words },
                    x if x.ty() == ty => x,
                    x => Expr::Cast {
                        arg: Box::new(x),
                        ty,
                    },
                })
            }
            ExprKind::SysCall { func, args } => {
                let mut xs = Vec::new();
                for g in args {
                    xs.push(self.conv(a, *g, ctx)?);
                }
                let first = xs.into_iter().next();
                let count = |x: Expr| Expr::CountOnes {
                    arg: Box::new(x),
                    ty: Ty::INT,
                };
                match (func, first) {
                    (SysFunc::CountOnes, Some(x)) => Some(coerce(count(x), ty)),
                    (SysFunc::OneHot, Some(x)) => Some(coerce(
                        bin(BinOp::Eq, count(x), Expr::konst(Ty::INT, 1), Ty::BIT),
                        ty,
                    )),
                    (SysFunc::OneHot0, Some(x)) => Some(coerce(
                        bin(BinOp::Le, count(x), Expr::konst(Ty::INT, 1), Ty::BIT),
                        ty,
                    )),
                    (SysFunc::IsUnknown, _) => Some(Expr::konst(ty, 0)),
                    (SysFunc::Signed | SysFunc::Unsigned, Some(x)) => {
                        let w = x.ty().w;
                        Some(coerce(coerce(x, Ty::new(w, ty.s)), ty))
                    }
                    _ => {
                        self.err(
                            loc,
                            format!("system function {func:?} needs constant arguments"),
                        );
                        None
                    }
                }
            }
            ExprKind::FuncCall {
                callee,
                callee_inst,
                callee_scope_prefix,
                name,
                args,
            } => {
                let mut xs = Vec::new();
                for g in args {
                    xs.push(self.conv(a, *g, ctx)?);
                }
                let f =
                    self.function(*callee, *callee_inst, callee_scope_prefix, name, ctx, loc)?;
                let func = self.m.functions.iter().find(|x| x.name == f)?;
                let ret = func.ret;
                if xs.len() != func.args.len() {
                    self.err(
                        loc,
                        format!(
                            "call of '{name}' needs default arguments, which are not supported"
                        ),
                    );
                    return None;
                }
                let args = xs
                    .into_iter()
                    .zip(func.args.iter())
                    .map(|(x, f)| coerce(x, f.ty))
                    .collect();
                Some(coerce(
                    Expr::Call {
                        func: f,
                        args,
                        ty: ret,
                    },
                    ty,
                ))
            }
            ExprKind::Sampled { .. } => {
                self.err(
                    loc,
                    "sampled value functions are not part of the synthesizable design",
                );
                None
            }
            ExprKind::AssignPattern(_) => {
                self.err(
                    loc,
                    "assignment patterns with non-constant members are not supported",
                );
                None
            }
        }
    }

    /// Elements of a one-dimensional constant table when `base` is an unpacked parameter or
    /// an assignment pattern, in declaration order, with the arena they are bound in.
    fn const_table(
        &mut self,
        a: &ExprArena,
        base: ExprId,
        ctx: &Ctx,
    ) -> Option<(ExprArena, Vec<ExprId>)> {
        let positional = |items: &[(Option<String>, ExprId)]| {
            items
                .iter()
                .all(|(k, _)| k.is_none())
                .then(|| items.iter().map(|(_, e)| *e).collect::<Vec<_>>())
        };
        match &a.get(base).kind {
            ExprKind::AssignPattern(items) => Some((a.clone(), positional(items)?)),
            ExprKind::Symbol(sid) => {
                let sym = self.design.symbol(*sid);
                if !matches!(sym.kind, SymbolKind::Param { .. }) {
                    return None;
                }
                let (tree, node) = sym.init_node?;
                let ictx = Ctx::new(sym.instance, &sym.init_scope_prefix, &ctx.gen_consts);
                let mut arena = ExprArena::new();
                let ty = sym.ty.clone();
                let b =
                    self.with_binder(&ictx, &mut arena, |b| b.bind_assigned(tree, node, &ty))?;
                let ExprKind::AssignPattern(items) = &arena.get(b).kind else {
                    return None;
                };
                let elems = positional(items)?;
                Some((arena, elems))
            }
            _ => None,
        }
    }

    /// Lowers a function on first use and returns its emitted name.
    fn function(
        &mut self,
        callee: (usize, NodeId),
        callee_inst: Option<InstanceId>,
        prefix: &str,
        name: &str,
        caller: &Ctx,
        loc: Loc,
    ) -> Option<String> {
        let key = (callee.0, callee.1, prefix.to_string());
        if let Some(n) = self.m.func_names.get(&key) {
            if self.m.func_stack.contains(&key) {
                self.err(
                    loc,
                    format!("recursive function '{name}' is not synthesizable"),
                );
                return None;
            }
            return Some(n.clone());
        }
        if callee_inst.is_some_and(|i| i != caller.inst) {
            self.err(
                loc,
                format!("call of function '{name}' in another instance is not supported"),
            );
            return None;
        }
        let (tree, node) = callee;
        let t = &self.trees[tree];
        let owner = callee_inst.unwrap_or(caller.inst);
        let gen_consts: Vec<(String, CVal)> = self
            .design
            .instance(owner)
            .items
            .iter()
            .find(|it| it.tree == tree && it.node == node && it.scope_prefix == prefix)
            .map(|it| it.gen_consts.clone())
            .unwrap_or_default();
        let mut ctx = Ctx::new(owner, prefix, &gen_consts);
        let mut arena = ExprArena::new();
        let rty = self.with_binder(&ctx, &mut arena, |b| {
            b.resolve_function_return_type(tree, node, callee_inst, prefix)
        })?;
        let Some(ret) = ity(&rty) else {
            self.err(
                loc,
                format!("function '{name}' must return a packed integral value"),
            );
            return None;
        };
        let fname = self.m.unique(name);
        self.m.func_names.insert(key.clone(), fname.clone());
        self.m.func_stack.push(key);
        ctx.func_ret = Some(ret);
        ctx.used.insert("_ret".into());

        // The function name is an implicit variable holding the return value.
        let ret_id = LocalId(self.next_local);
        self.next_local += 1;
        ctx.locals.push((
            name.to_string(),
            LocalBinding {
                id: ret_id,
                ty: rty.clone(),
                packed_bounds: None,
                cval: None,
            },
        ));
        ctx.local_names.insert(ret_id.0, ("_ret".into(), ret));

        let mut args = Vec::new();
        let mut ok = true;
        for f in extract_subroutine_formals(t, node) {
            if !matches!(f.dir, PortDir::Input) {
                self.err(
                    loc,
                    format!(
                        "function '{name}' has an output, inout or ref argument, which is not supported"
                    ),
                );
                ok = false;
                continue;
            }
            let mut a2 = ExprArena::new();
            let fty = self.with_binder(&ctx, &mut a2, |b| {
                b.resolve_subroutine_dtype(tree, f.dtype, f.decl, callee_inst, prefix)
            });
            let Some(fty) = fty else {
                ok = false;
                continue;
            };
            let Some(ty) = ity(&fty) else {
                self.err(
                    loc,
                    format!(
                        "argument '{}' of function '{name}' must be a packed integral value",
                        f.name
                    ),
                );
                ok = false;
                continue;
            };
            let cname = self.new_local(&mut ctx, &f.name, fty, ty);
            args.push(LocalVar { name: cname, ty });
        }
        ctx.decls.clear();

        let mut body = Vec::new();
        if let Some(b) = t.child(node, SyntaxKind::Body) {
            for ch in t.children(b) {
                self.stmt(tree, ch, &mut ctx, &mut body);
            }
        }
        self.m.func_stack.pop();
        if !ok {
            return None;
        }
        if writes_signal(&body) {
            self.err(
                loc,
                format!("function '{name}' assigns a module signal, which is not supported"),
            );
            return None;
        }
        self.m.functions.push(hdl::Function {
            name: fname.clone(),
            ret,
            args,
            locals: ctx.decls,
            body,
        });
        Some(fname)
    }
}

fn is_assign_op(op: &str) -> bool {
    matches!(
        op,
        "=" | "<="
            | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "&="
            | "|="
            | "^="
            | "<<="
            | ">>="
            | "<<<="
            | ">>>="
    )
}

fn flatten_events(t: &Tree, ev: NodeId, out: &mut Vec<NodeId>) {
    let subs: Vec<NodeId> = t
        .children(ev)
        .filter(|&c| t.kind(c) == SyntaxKind::EventExpr)
        .collect();
    if subs.is_empty() {
        out.push(ev);
    } else {
        for s in subs {
            flatten_events(t, s, out);
        }
    }
}

/// Bit offset of constant index `idx` inside a vector declared with `bounds`.
fn const_offset(idx: i128, bounds: (i128, i128), elem_w: u32) -> Option<u32> {
    let (left, right) = bounds;
    let rel = if left >= right {
        idx - right
    } else {
        right - idx
    };
    if rel < 0 {
        return None;
    }
    u32::try_from(rel * elem_w as i128).ok()
}

/// Offset from the base index of `[b +: w]` / `[b -: w]` to the index of its least significant element.
fn indexed_adjust(bounds: (i128, i128), width: u32, ascending: bool) -> i128 {
    let desc = bounds.0 >= bounds.1;
    let w = width as i128;
    match (desc, ascending) {
        (true, true) => 0,
        (true, false) => -(w - 1),
        (false, true) => w - 1,
        (false, false) => 0,
    }
}

fn slice_of(base: Expr, pos: Option<Pos>, w: u32, ty: Ty) -> Expr {
    let Some(pos) = pos else {
        // A constant select outside the vector reads x, which maps to 0.
        return Expr::konst(ty, 0);
    };
    if let (Expr::Const { words, .. }, None) = (&base, &pos.lo_dyn) {
        let mut out = vec![0u64; w.div_ceil(64) as usize];
        for i in 0..w {
            if get_bit(words, pos.lo + i) {
                set_bit(&mut out, i);
            }
        }
        return coerce(
            Expr::Const {
                ty: Ty::unsigned(w),
                words: out,
            },
            ty,
        );
    }
    if pos.lo == 0 && pos.lo_dyn.is_none() && w == base.ty().w {
        return coerce(coerce(base, Ty::unsigned(w)), ty);
    }
    coerce(
        Expr::Slice {
            base: Box::new(base),
            lo: pos.lo,
            lo_dyn: pos.lo_dyn.map(Box::new),
            ty: Ty::unsigned(w),
        },
        ty,
    )
}

fn compose(mut p: Lval, pos: Pos, w: u32) -> Lval {
    p.sel = Some(match p.sel.take() {
        None => LvSel {
            lo: pos.lo,
            lo_dyn: pos.lo_dyn,
            w,
        },
        Some(s) => LvSel {
            lo: s.lo + pos.lo,
            lo_dyn: add_dyn(s.lo_dyn, pos.lo_dyn),
            w,
        },
    });
    p
}

fn walk_stmts(stmts: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
    for s in stmts {
        f(s);
        match s {
            Stmt::If { then_s, else_s, .. } => {
                walk_stmts(then_s, f);
                walk_stmts(else_s, f);
            }
            Stmt::Case { arms, default, .. } => {
                for a in arms {
                    walk_stmts(&a.body, f);
                }
                if let Some(d) = default {
                    walk_stmts(d, f);
                }
            }
            Stmt::For {
                init, step, body, ..
            } => {
                walk_stmts(init, f);
                walk_stmts(step, f);
                walk_stmts(body, f);
            }
            Stmt::Block { body, .. } => walk_stmts(body, f),
            Stmt::Assign { .. } | Stmt::Return(_) => {}
        }
    }
}

fn first_target(stmts: &[Stmt]) -> Option<String> {
    let mut found = None;
    walk_stmts(stmts, &mut |s| {
        if let Stmt::Assign { lhs, .. } = s {
            if found.is_none() && !matches!(lhs.base, LvBase::Local { .. }) {
                found = Some(lhs.base.name().to_string());
            }
        }
    });
    found
}

fn writes_signal(stmts: &[Stmt]) -> bool {
    let mut w = false;
    walk_stmts(stmts, &mut |s| {
        if let Stmt::Assign { lhs, .. } = s {
            if !matches!(lhs.base, LvBase::Local { .. }) {
                w = true;
            }
        }
    });
    w
}

/// Driver keys of a process: `name` for a plain signal, `name[i][j]` for a constant array
/// element, `name[*]` when the element is selected dynamically.
fn driver_keys(stmts: &[Stmt]) -> HashSet<String> {
    let mut keys = HashSet::new();
    walk_stmts(stmts, &mut |s| {
        if let Stmt::Assign { lhs, .. } = s {
            match &lhs.base {
                LvBase::Sig { name, .. } => {
                    keys.insert(name.clone());
                }
                LvBase::Elem { name, idx, .. } => {
                    let consts: Option<Vec<u64>> = idx.iter().map(|e| e.as_u64()).collect();
                    keys.insert(match consts {
                        Some(c) => format!(
                            "{name}{}",
                            c.iter().map(|i| format!("[{i}]")).collect::<String>()
                        ),
                        None => format!("{name}[*]"),
                    });
                }
                LvBase::Local { .. } => {}
            }
        }
    });
    keys
}

fn conn_key(c: &Conn) -> String {
    format!(
        "{}{}",
        c.sig,
        c.elem.iter().map(|i| format!("[{i}]")).collect::<String>()
    )
}

fn keys_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (an, bn) = (
        a.split('[').next().unwrap_or(a),
        b.split('[').next().unwrap_or(b),
    );
    an == bn && (a.ends_with("[*]") || b.ends_with("[*]"))
}

/// Names of the plain signals a statement list reads.
fn read_sigs(stmts: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    let on_expr = |e: &Expr, out: &mut HashSet<String>| {
        e.walk(&mut |x| {
            if let Expr::Sig { name, .. } = x {
                out.insert(name.clone());
            }
        });
    };
    walk_stmts(stmts, &mut |s| match s {
        Stmt::Assign { lhs, rhs, .. } => {
            on_expr(rhs, &mut out);
            if let Some(LvSel {
                lo_dyn: Some(d), ..
            }) = &lhs.sel
            {
                on_expr(d, &mut out);
            }
            if let LvBase::Elem { idx, .. } = &lhs.base {
                idx.iter().for_each(|e| on_expr(e, &mut out));
            }
        }
        Stmt::If { cond, .. } => on_expr(cond, &mut out),
        Stmt::Case { sel, arms, .. } => {
            on_expr(sel, &mut out);
            for a in arms {
                for p in &a.pats {
                    if let Pat::Expr(e) = p {
                        on_expr(e, &mut out);
                    }
                }
            }
        }
        Stmt::For { cond, .. } => on_expr(cond, &mut out),
        Stmt::Return(e) => on_expr(e, &mut out),
        Stmt::Block { .. } => {}
    });
    out
}
