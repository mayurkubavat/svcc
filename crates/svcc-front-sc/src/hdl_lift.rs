//! Reads generated SystemC back into the typed `svcc_ir::hdl` design IR, *with C++ meaning*.
//!
//! The input is the header `svcc rtl2sc` writes (`SC_MODULE`s made of `sc_in` / `sc_out` /
//! `sc_signal` / `sc_vector` members, `const` member functions, `SC_METHOD` processes and an
//! `SC_CTOR`). The output is a design whose expressions say what that C++ computes under the
//! rules of [`crate::cxx_sem`]: a sum of two `sc_uint<8>` values is a 64-bit sum here, and it
//! is 8 bits wide only if the C++ constructs an `sc_uint<8>` from it.
//!
//! What the reader checks beyond expressions:
//!
//! - **Signal writes.** In an edge-triggered method `.write` is a nonblocking assignment. In
//!   a level-sensitive method it becomes a blocking assignment, which is the same thing only
//!   if the method never reads that signal; a method that does is rejected.
//! - **Local copies.** `T v = s.read(); ...; s.write(v);` around a body that does not touch
//!   `s` otherwise is the signal `s` assigned with blocking assignments (the same for a copy
//!   of a whole `sc_vector`). Any other use of a copy is rejected.
//! - **Sensitivity.** A level-sensitive method must be sensitive to every signal it reads
//!   (directly or through the functions it calls) and must not be `dont_initialize()`d; an
//!   edge-triggered one must be.
//! - **Undefined behaviour.** Each module gets a 1-bit output [`UB_PORT`] that is 1 in a
//!   step where one of its processes, functions or child modules executes an operation whose
//!   result C++ or SystemC does not define. A proof must show it stays 0.
//!
//! Anything outside this subset is an error: the design is then reported as not checked.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::cxx::{err, lex, Ast, CTy, Parser, SAst, Tok};
use crate::cxx_sem::{conv, cty_ty, konst, or, reads, Body, FnSig, SigInfo, UB_LOCAL};
use svcc_ir::hdl::{
    Conn, Design, Dir, Edge, Expr, Function, Instance, LocalVar, LvBase, Lval, Module, Port,
    ProcKind, Process, Signal, Stmt, Ty,
};
use svcc_ir::SvccError;

/// Output port every lifted module gets: 1 when undefined behaviour is executed.
pub const UB_PORT: &str = "svcc__ub";

enum Sens {
    Sig(String),
    Elem(String, u64),
    Array(String),
    Edge(String, bool),
}

enum CtorItem {
    Method(String),
    Sens(Vec<Sens>),
    DontInit,
    InitWrite {
        sig: String,
        val: Ast,
    },
    Bind {
        inst: String,
        port: String,
        sig: String,
        elem: Option<u64>,
    },
}

struct SigSrc {
    c: String,
    name: String,
    ty: CTy,
    dim: Option<u32>,
    dir: Option<Dir>,
    init: Option<Ast>,
}

struct FuncSrc {
    name: String,
    ret: CTy,
    args: Vec<(CTy, String)>,
    body: Vec<SAst>,
}

struct ModSrc {
    name: String,
    sigs: Vec<SigSrc>,
    /// `(module type, C++ identifier, instance name)`.
    insts: Vec<(String, String, String)>,
    funcs: Vec<FuncSrc>,
    methods: Vec<(String, Vec<SAst>)>,
    ctor: Vec<CtorItem>,
}

fn parse_module(p: &mut Parser) -> Result<ModSrc, SvccError> {
    p.expect_p("(")?;
    let name = p.ident()?;
    p.expect_p(")")?;
    p.expect_p("{")?;
    let mut m = ModSrc {
        name,
        sigs: Vec::new(),
        insts: Vec::new(),
        funcs: Vec::new(),
        methods: Vec::new(),
        ctor: Vec::new(),
    };
    let value_type = |p: &mut Parser| -> Result<CTy, SvccError> {
        match p.try_type()? {
            Some(t @ (CTy::Bool | CTy::Int { .. } | CTy::Sc { .. } | CTy::Big { .. })) => Ok(t),
            _ => p.fail(
                "signal value type (bool, fixed-width int, sc_uint, sc_int, sc_biguint or sc_bigint)",
            ),
        }
    };
    loop {
        if p.eat_p("}") {
            p.expect_p(";")?;
            return Ok(m);
        }
        if p.at_end() {
            return p.fail("unterminated SC_MODULE");
        }
        if p.eat_id("SC_CTOR") {
            p.expect_p("(")?;
            p.ident()?;
            p.expect_p(")")?;
            p.expect_p("{")?;
            m.ctor = parse_ctor(p)?;
            continue;
        }
        if p.eat_id("void") {
            let name = p.ident()?;
            p.expect_p("(")?;
            p.expect_p(")")?;
            p.expect_p("{")?;
            let body = p.block_body()?;
            m.methods.push((name, body));
            continue;
        }
        let kind = match p.peek() {
            Some(Tok::Id(id)) => id.clone(),
            _ => return p.fail("module member"),
        };
        match kind.as_str() {
            "sc_core::sc_in" | "sc_core::sc_out" | "sc_core::sc_signal" | "sc_in" | "sc_out"
            | "sc_signal" => {
                p.i += 1;
                p.expect_p("<")?;
                let ty = value_type(p)?;
                p.expect_gt()?;
                let c = p.ident()?;
                p.expect_p("{")?;
                let name = p.string()?;
                let init = if p.eat_p(",") { Some(p.expr()?) } else { None };
                p.expect_p("}")?;
                p.expect_p(";")?;
                let dir = match kind.as_str() {
                    "sc_core::sc_in" | "sc_in" => Some(Dir::In),
                    "sc_core::sc_out" | "sc_out" => Some(Dir::Out),
                    _ => None,
                };
                if dir.is_some() && init.is_some() {
                    return p.fail("port with a start value");
                }
                m.sigs.push(SigSrc {
                    c,
                    name,
                    ty,
                    dim: None,
                    dir,
                    init,
                });
            }
            "sc_core::sc_vector" | "sc_vector" => {
                p.i += 1;
                p.expect_p("<")?;
                // A vector of signals is an unpacked array; a vector of ports an array port.
                let vdir = if p.eat_id("sc_core::sc_signal") || p.eat_id("sc_signal") {
                    None
                } else if p.eat_id("sc_core::sc_in") || p.eat_id("sc_in") {
                    Some(Dir::In)
                } else if p.eat_id("sc_core::sc_out") || p.eat_id("sc_out") {
                    Some(Dir::Out)
                } else {
                    return p.fail("sc_vector of sc_signal, sc_in or sc_out");
                };
                p.expect_p("<")?;
                let ty = value_type(p)?;
                p.expect_gt()?;
                p.expect_gt()?;
                let c = p.ident()?;
                p.expect_p("{")?;
                let name = p.string()?;
                p.expect_p(",")?;
                let n = p.number()?;
                p.expect_p("}")?;
                p.expect_p(";")?;
                if n == 0 || n > (1 << 24) {
                    return p.fail("sc_vector size");
                }
                m.sigs.push(SigSrc {
                    c,
                    name,
                    ty,
                    dim: Some(n as u32),
                    dir: vdir,
                    init: None,
                });
            }
            _ => {
                if let Some(ret) = p.try_type()? {
                    // Member function: `T name(T a, ...) const { ... }`.
                    let name = p.ident()?;
                    p.expect_p("(")?;
                    let mut args = Vec::new();
                    if !p.eat_p(")") {
                        loop {
                            let Some(t) = p.try_type()? else {
                                return p.fail("parameter type");
                            };
                            args.push((t, p.ident()?));
                            if p.eat_p(")") {
                                break;
                            }
                            p.expect_p(",")?;
                        }
                    }
                    if !p.eat_id("const") {
                        return p.fail("member function that is not const");
                    }
                    p.expect_p("{")?;
                    let body = p.block_body()?;
                    m.funcs.push(FuncSrc {
                        name,
                        ret,
                        args,
                        body,
                    });
                } else {
                    // Child module instance: `type name{"name"};`.
                    let ty = p.ident()?;
                    let c = p.ident()?;
                    p.expect_p("{")?;
                    let name = p.string()?;
                    p.expect_p("}")?;
                    p.expect_p(";")?;
                    m.insts.push((ty, c, name));
                }
            }
        }
    }
}

fn parse_ctor(p: &mut Parser) -> Result<Vec<CtorItem>, SvccError> {
    let mut out = Vec::new();
    loop {
        if p.eat_p("}") {
            return Ok(out);
        }
        if p.eat_id("SC_METHOD") {
            p.expect_p("(")?;
            out.push(CtorItem::Method(p.ident()?));
            p.expect_p(")")?;
            p.expect_p(";")?;
        } else if p.eat_id("sensitive") {
            let mut terms = Vec::new();
            while p.eat_p("<<") {
                let s = p.ident()?;
                if p.eat_p("[") {
                    let i = p.number()?;
                    p.expect_p("]")?;
                    terms.push(Sens::Elem(s, i));
                } else if p.eat_p(".") {
                    let e = p.ident()?;
                    p.expect_p("(")?;
                    p.expect_p(")")?;
                    match e.as_str() {
                        // `posedge_event()` is the same event on a signal that is not a port.
                        "pos" | "posedge_event" => terms.push(Sens::Edge(s, true)),
                        "neg" | "negedge_event" => terms.push(Sens::Edge(s, false)),
                        _ => return p.fail("event finder other than pos() or neg()"),
                    }
                } else {
                    terms.push(Sens::Sig(s));
                }
            }
            p.expect_p(";")?;
            out.push(CtorItem::Sens(terms));
        } else if p.eat_id("for") {
            // for (auto& s : vec) sensitive << s;
            p.expect_p("(")?;
            if !p.eat_id("auto") {
                return p.fail("range-for over a signal vector");
            }
            p.expect_p("&")?;
            let v = p.ident()?;
            p.expect_p(":")?;
            let arr = p.ident()?;
            p.expect_p(")")?;
            if !p.eat_id("sensitive") {
                return p.fail("sensitive");
            }
            p.expect_p("<<")?;
            if p.ident()? != v {
                return p.fail("loop variable");
            }
            p.expect_p(";")?;
            out.push(CtorItem::Sens(vec![Sens::Array(arr)]));
        } else if p.eat_id("dont_initialize") {
            p.expect_p("(")?;
            p.expect_p(")")?;
            p.expect_p(";")?;
            out.push(CtorItem::DontInit);
        } else {
            let inst = p.ident()?;
            p.expect_p(".")?;
            let port = p.ident()?;
            p.expect_p("(")?;
            if port == "write" {
                let val = p.expr()?;
                p.expect_p(")")?;
                p.expect_p(";")?;
                out.push(CtorItem::InitWrite { sig: inst, val });
            } else {
                let sig = p.ident()?;
                let elem = if p.eat_p("[") {
                    let i = p.number()?;
                    p.expect_p("]")?;
                    Some(i)
                } else {
                    None
                };
                p.expect_p(")")?;
                p.expect_p(";")?;
                out.push(CtorItem::Bind {
                    inst,
                    port,
                    sig,
                    elem,
                });
            }
        }
    }
}

fn count_idents(ss: &[SAst]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for s in ss {
        s.idents(&mut |n| *m.entry(n.to_string()).or_insert(0) += 1);
    }
    m
}

/// `k`-indexed copy loop over a whole array: `for (unsigned k = 0; k < n; ++k) <stmt>`.
/// Returns the loop variable, the bound and the single body statement.
fn copy_loop(s: &SAst) -> Option<(&str, u64, &SAst)> {
    let SAst::For {
        init,
        cond,
        step,
        body,
    } = s
    else {
        return None;
    };
    let [SAst::Decl {
        name: k,
        init: Some(Ast::Num { v: 0, .. }),
        array: None,
        ..
    }] = init.as_slice()
    else {
        return None;
    };
    let Ast::Bin(op, l, r) = cond else {
        return None;
    };
    let (Ast::Id(ck), Ast::Num { v: n, .. }) = (l.as_ref(), r.as_ref()) else {
        return None;
    };
    let [SAst::Assign {
        lhs: Ast::Id(sk), ..
    }] = step.as_slice()
    else {
        return None;
    };
    if op != "<" || ck != k || sk != k || body.len() != 1 {
        return None;
    }
    Some((k, *n, &body[0]))
}

struct AliasCand {
    local: String,
    sig: String,
    array: bool,
}

/// Recognizes the copy-in statements at the top of a method and the matching copy-out
/// statements at its bottom. Returns the aliases and the body between them.
fn split_copies<'a>(
    body: &'a [SAst],
    sigs: &HashMap<String, SigInfo>,
) -> Result<(Vec<AliasCand>, &'a [SAst]), SvccError> {
    let mut cands: Vec<AliasCand> = Vec::new();
    let mut lo = 0;
    while lo < body.len() {
        match &body[lo] {
            SAst::Decl {
                ty,
                name,
                array: None,
                init: Some(Ast::Method(obj, m, args)),
            } if m == "read" && args.is_empty() => {
                let Ast::Id(s) = obj.as_ref() else { break };
                let Some(si) = sigs.get(s) else { break };
                if si.dim.is_some() || si.ty != *ty || !si.writable {
                    break;
                }
                cands.push(AliasCand {
                    local: name.clone(),
                    sig: s.clone(),
                    array: false,
                });
                lo += 1;
            }
            SAst::Decl {
                ty,
                name,
                array: Some(n),
                init: None,
            } => {
                // T sh[n]; for (unsigned k = 0; k < n; ++k) sh[k] = s[k].read();
                let Some((k, bound, SAst::Assign { lhs, rhs })) =
                    body.get(lo + 1).and_then(copy_loop)
                else {
                    break;
                };
                let kid = Ast::Id(k.to_string());
                let Ast::Method(src, m, args) = rhs else {
                    break;
                };
                let Ast::Index(arr, ix) = src.as_ref() else {
                    break;
                };
                let Ast::Id(s) = arr.as_ref() else { break };
                let ok_lhs =
                    *lhs == Ast::Index(Box::new(Ast::Id(name.clone())), Box::new(kid.clone()));
                let Some(si) = sigs.get(s) else { break };
                if !ok_lhs
                    || m != "read"
                    || !args.is_empty()
                    || **ix != kid
                    || bound != *n
                    || si.dim != Some(*n as u32)
                    || si.ty != *ty
                {
                    break;
                }
                cands.push(AliasCand {
                    local: name.clone(),
                    sig: s.clone(),
                    array: true,
                });
                lo += 2;
            }
            _ => break,
        }
    }
    let mut hi = body.len();
    let mut closed = BTreeSet::new();
    while hi > lo {
        match &body[hi - 1] {
            SAst::Write {
                target: Ast::Id(s),
                value: Ast::Id(l),
            } if cands
                .iter()
                .any(|c| !c.array && c.sig == *s && c.local == *l) =>
            {
                closed.insert(l.clone());
                hi -= 1;
            }
            other => {
                let Some((k, bound, SAst::Write { target, value })) = copy_loop(other) else {
                    break;
                };
                let kid = Ast::Id(k.to_string());
                let (Ast::Index(arr, ti), Ast::Index(loc, vi)) = (target, value) else {
                    break;
                };
                let (Ast::Id(s), Ast::Id(l)) = (arr.as_ref(), loc.as_ref()) else {
                    break;
                };
                let known = cands
                    .iter()
                    .any(|c| c.array && c.sig == *s && c.local == *l);
                let full = sigs.get(s).and_then(|x| x.dim) == Some(bound as u32);
                if !known || !full || **ti != kid || **vi != kid {
                    break;
                }
                closed.insert(l.clone());
                hi -= 1;
            }
        }
    }
    for c in &cands {
        if !closed.contains(&c.local) {
            return err(format!(
                "local '{}' copies signal '{}' but is not written back at the end of the method",
                c.local, c.sig
            ));
        }
    }
    Ok((cands, &body[lo..hi]))
}

fn local(name: &str, ty: Ty) -> Expr {
    Expr::Local {
        name: name.to_string(),
        ty,
    }
}

fn assign_sig(name: &str, rhs: Expr, nonblocking: bool) -> Stmt {
    Stmt::Assign {
        lhs: Lval {
            base: LvBase::Sig {
                name: name.to_string(),
                ty: Ty::BIT,
            },
            sel: None,
        },
        rhs,
        nonblocking,
    }
}

/// Port names of an already lifted module, by C++ identifier.
type PortMap = HashMap<String, (String, Option<u32>)>;

fn lift_module(src: &ModSrc, known: &HashMap<String, PortMap>) -> Result<Module, SvccError> {
    let ctx = |e: SvccError| {
        SvccError::new(
            "svcc-front-sc/hdl-lift",
            format!("module '{}': {e}", src.name),
        )
    };
    let mut sigs: HashMap<String, SigInfo> = HashMap::new();
    let mut names = BTreeSet::new();
    for s in &src.sigs {
        if !names.insert(s.name.clone()) || s.name.starts_with("svcc__") {
            return Err(ctx(SvccError::new("", format!("signal name '{}'", s.name))));
        }
        sigs.insert(
            s.c.clone(),
            SigInfo {
                name: s.name.clone(),
                ty: s.ty,
                dim: s.dim,
                writable: s.dir != Some(Dir::In),
            },
        );
    }
    let mut funcs: HashMap<String, FnSig> = HashMap::new();
    for f in &src.funcs {
        funcs.insert(
            f.name.clone(),
            FnSig {
                name: f.name.clone(),
                ret: f.ret,
                args: f.args.iter().map(|a| a.0).collect(),
            },
        );
    }

    let mut m = Module {
        name: src.name.clone(),
        definition: src.name.clone(),
        params: Vec::new(),
        ports: Vec::new(),
        signals: Vec::new(),
        functions: Vec::new(),
        processes: Vec::new(),
        instances: Vec::new(),
        file: String::new(),
        line: 0,
    };
    for s in &src.sigs {
        let ty = cty_ty(s.ty);
        let scalar = s.ty == CTy::Bool;
        match s.dir {
            Some(dir) => m.ports.push(Port {
                name: s.name.clone(),
                dir,
                ty,
                scalar,
                dims: s.dim.into_iter().collect(),
            }),
            None => {
                let ctor_init = src.ctor.iter().rev().find_map(|it| match it {
                    CtorItem::InitWrite { sig, val } if *sig == s.c => Some(val),
                    _ => None,
                });
                let init = match ctor_init.or(s.init.as_ref()) {
                    None => None,
                    Some(a) => {
                        let mut b = Body::new(&sigs, &funcs, false, None);
                        let v = b
                            .expr(a)
                            .and_then(|v| b.assign_conv(s.ty, v))
                            .map_err(ctx)?;
                        match v.e {
                            Expr::Const { words, .. } if v.ub.is_none() => Some(words),
                            _ => {
                                return Err(ctx(SvccError::new(
                                    "",
                                    format!("start value of '{}' is not a constant", s.name),
                                )))
                            }
                        }
                    }
                };
                m.signals.push(Signal {
                    name: s.name.clone(),
                    ty,
                    scalar,
                    dims: s.dim.into_iter().collect(),
                    init,
                });
            }
        }
    }
    for item in &src.ctor {
        if let CtorItem::InitWrite { sig, .. } = item {
            match src.sigs.iter().find(|s| s.c == *sig) {
                Some(s) if s.dir.is_none() && s.dim.is_none() => {}
                _ => {
                    return Err(ctx(SvccError::new(
                        "",
                        format!("constructor .write() on '{sig}', which is not a scalar sc_signal"),
                    )))
                }
            }
        }
    }

    // Identifiers each function uses, closed over calls: a method that calls a function
    // touches every signal that function touches.
    let mut fn_idents: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in &src.funcs {
        let mut ids = BTreeSet::new();
        f.body.iter().for_each(|s| {
            s.idents(&mut |n| {
                ids.insert(n.to_string());
            })
        });
        fn_idents.insert(f.name.clone(), ids);
    }
    loop {
        let mut changed = false;
        for f in &src.funcs {
            let callees: Vec<String> = fn_idents[&f.name]
                .iter()
                .filter(|n| fn_idents.contains_key(*n) && **n != f.name)
                .cloned()
                .collect();
            for c in callees {
                let add: Vec<String> = fn_idents[&c].iter().cloned().collect();
                let set = fn_idents.get_mut(&f.name).expect("function");
                for a in add {
                    changed |= set.insert(a);
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Functions.
    let mut fn_bodies: Vec<(String, Vec<Stmt>)> = Vec::new();
    for f in &src.funcs {
        let mut b = Body::new(&sigs, &funcs, false, Some(f.ret));
        for (t, n) in &f.args {
            b.declare(n, *t);
        }
        let args = std::mem::take(&mut b.locals);
        let body = b.stmts(&f.body).map_err(ctx)?;
        if !matches!(body.last(), Some(Stmt::Return(_))) {
            // Flowing off the end of a value-returning function is undefined.
            return Err(ctx(SvccError::new(
                "",
                format!("function '{}' does not end in a return", f.name),
            )));
        }
        if !b.written.is_empty() {
            return Err(ctx(SvccError::new(
                "",
                format!("function '{}' writes a signal", f.name),
            )));
        }
        let mut locals = std::mem::take(&mut b.locals);
        locals.push(LocalVar {
            name: UB_LOCAL.to_string(),
            ty: Ty::BIT,
        });
        fn_bodies.push((f.name.clone(), body.clone()));
        m.functions.push(Function {
            name: f.name.clone(),
            ret: Ty::unsigned(cty_ty(f.ret).w + 1),
            args,
            locals,
            body,
        });
    }
    let mut fn_reads: BTreeMap<String, BTreeSet<(String, Option<u64>)>> = BTreeMap::new();
    for _ in 0..=fn_bodies.len() {
        for (name, body) in &fn_bodies {
            let mut r = BTreeSet::new();
            reads(body, &fn_reads, &mut r);
            fn_reads.insert(name.clone(), r);
        }
    }

    // Processes, in the order the constructor registers them.
    let mut flags: Vec<String> = Vec::new();
    let mut registered = BTreeSet::new();
    let mut i = 0;
    while i < src.ctor.len() {
        let CtorItem::Method(pname) = &src.ctor[i] else {
            i += 1;
            continue;
        };
        i += 1;
        let mut sens: Vec<&Sens> = Vec::new();
        let mut dont_init = false;
        while i < src.ctor.len() {
            match &src.ctor[i] {
                CtorItem::Sens(t) => sens.extend(t.iter()),
                CtorItem::DontInit => dont_init = true,
                _ => break,
            }
            i += 1;
        }
        let perr = |msg: String| ctx(SvccError::new("", format!("method '{pname}': {msg}")));
        if !registered.insert(pname.clone()) {
            return Err(perr("registered twice".into()));
        }
        let Some((_, body)) = src.methods.iter().find(|x| x.0 == *pname) else {
            return Err(perr("no such member function".into()));
        };
        let hdl_name = |c: &str| -> Result<String, SvccError> {
            sigs.get(c)
                .map(|s| s.name.clone())
                .ok_or_else(|| perr(format!("'{c}' in the sensitivity list is not a signal")))
        };
        let mut edges = Vec::new();
        let mut level = false;
        let body_ids = count_idents(body);
        for s in &sens {
            match s {
                Sens::Edge(c, pos) => edges.push(Edge {
                    sig: hdl_name(c)?,
                    posedge: *pos,
                    is_async_ctrl: body_ids.contains_key(c),
                }),
                _ => level = true,
            }
        }
        if level && !edges.is_empty() {
            return Err(perr("mixes edge and level sensitivity".into()));
        }
        let seq = !edges.is_empty();
        if !seq && dont_init {
            return Err(perr("level-sensitive method with dont_initialize()".into()));
        }

        let (aliases, inner) = split_copies(body, &sigs).map_err(|e| perr(e.to_string()))?;
        let called: BTreeSet<&String> =
            body_ids.keys().filter(|n| funcs.contains_key(*n)).collect();
        for a in &aliases {
            let uses = body_ids.get(&a.sig).copied().unwrap_or(0);
            let decls = count_decls(body, &a.local);
            let via_fn = called.iter().any(|f| fn_idents[*f].contains(&a.sig));
            if uses != 2 || decls != 1 || via_fn {
                return Err(perr(format!(
                    "signal '{}' is used apart from its local copy '{}'",
                    a.sig, a.local
                )));
            }
        }
        let mut b = Body::new(&sigs, &funcs, seq, None);
        for a in &aliases {
            b.alias(&a.local, &a.sig, a.array)
                .map_err(|e| perr(e.to_string()))?;
        }
        let mut stmts = b.stmts(inner).map_err(|e| perr(e.to_string()))?;
        let mut rd = BTreeSet::new();
        reads(&stmts, &fn_reads, &mut rd);
        let alias_names: BTreeSet<String> = aliases
            .iter()
            .filter_map(|a| sigs.get(&a.sig).map(|s| s.name.clone()))
            .collect();
        if !seq {
            // A blocking assignment stands for `.write` only if the method never reads
            // the signal it writes.
            for w in &b.written {
                let n = &sigs[w].name;
                if rd.iter().any(|r| r.0 == *n) {
                    return Err(perr(format!(
                        "level-sensitive method reads signal '{n}' that it also writes"
                    )));
                }
            }
            let mut covered: BTreeSet<(String, Option<u64>)> = BTreeSet::new();
            for s in &sens {
                match s {
                    Sens::Sig(c) | Sens::Array(c) => {
                        covered.insert((hdl_name(c)?, None));
                    }
                    Sens::Elem(c, k) => {
                        covered.insert((hdl_name(c)?, Some(*k)));
                    }
                    Sens::Edge(..) => {}
                }
            }
            for (n, idx) in &rd {
                if alias_names.contains(n) {
                    continue;
                }
                let ok = covered.contains(&(n.clone(), None))
                    || (idx.is_some() && covered.contains(&(n.clone(), *idx)));
                if !ok {
                    return Err(perr(format!("not sensitive to '{n}', which it reads")));
                }
            }
        }
        let mut locals = std::mem::take(&mut b.locals);
        if b.ub_used {
            let flag = format!("{UB_PORT}_p_{pname}");
            locals.push(LocalVar {
                name: UB_LOCAL.to_string(),
                ty: Ty::BIT,
            });
            let raise = assign_sig(&flag, local(UB_LOCAL, Ty::BIT), seq);
            match stmts.as_mut_slice() {
                // Keep `if (ctrl) ... else ...` as the only statement of a method with an
                // asynchronous control.
                [Stmt::If { then_s, else_s, .. }] if edges.len() > 1 => {
                    then_s.push(raise.clone());
                    else_s.push(raise);
                }
                _ => stmts.push(raise),
            }
            m.signals.push(Signal {
                name: flag.clone(),
                ty: Ty::BIT,
                scalar: true,
                dims: Vec::new(),
                init: None,
            });
            flags.push(flag);
        }
        m.processes.push(Process {
            name: pname.clone(),
            kind: if seq {
                ProcKind::Seq { edges }
            } else {
                ProcKind::Comb
            },
            locals,
            body: stmts,
            merged_from: 1,
            line: 0,
        });
    }
    for (name, _) in &src.methods {
        if !registered.contains(name) {
            return Err(ctx(SvccError::new(
                "",
                format!("member function '{name}' is not registered as a process"),
            )));
        }
    }

    // Instances and their port bindings.
    for (ty, c, name) in &src.insts {
        let Some(child) = known.get(ty) else {
            return Err(ctx(SvccError::new(
                "",
                format!("instance '{name}' of unknown module '{ty}'"),
            )));
        };
        let mut conns = Vec::new();
        let mut bound = BTreeSet::new();
        for item in &src.ctor {
            let CtorItem::Bind {
                inst,
                port,
                sig,
                elem,
            } = item
            else {
                continue;
            };
            if inst != c {
                continue;
            }
            let (Some((pn, pdim)), Some(si)) = (child.get(port), sigs.get(sig)) else {
                return Err(ctx(SvccError::new(
                    "",
                    format!("binding {inst}.{port}({sig}) names an unknown port or signal"),
                )));
            };
            // A scalar port takes a scalar signal or one element of a vector; an array port
            // takes a whole vector of the same size.
            let shape_ok = match (pdim, elem, si.dim) {
                (None, None, None) | (None, Some(_), Some(_)) => true,
                (Some(n), None, Some(m)) => *n == m,
                _ => false,
            };
            if !bound.insert(pn.clone()) || !shape_ok {
                return Err(ctx(SvccError::new(
                    "",
                    format!("binding {inst}.{port}({sig})"),
                )));
            }
            if elem.is_some_and(|e| e >= si.dim.unwrap_or(0) as u64) {
                return Err(ctx(SvccError::new(
                    "",
                    format!("binding {inst}.{port}({sig}[..]) is outside the vector"),
                )));
            }
            conns.push(Conn {
                port: pn.clone(),
                sig: si.name.clone(),
                elem: elem.iter().map(|e| *e as u32).collect(),
            });
        }
        if bound.len() != child.len() {
            return Err(ctx(SvccError::new(
                "",
                format!("instance '{name}' has unbound ports"),
            )));
        }
        let flag = format!("{UB_PORT}_i_{name}");
        m.signals.push(Signal {
            name: flag.clone(),
            ty: Ty::BIT,
            scalar: true,
            dims: Vec::new(),
            init: None,
        });
        conns.push(Conn {
            port: UB_PORT.to_string(),
            sig: flag.clone(),
            elem: Vec::new(),
        });
        flags.push(flag);
        m.instances.push(Instance {
            name: name.clone(),
            module: ty.clone(),
            conns,
        });
    }
    for item in &src.ctor {
        if let CtorItem::Bind { inst, .. } = item {
            if !src.insts.iter().any(|x| x.1 == *inst) {
                return Err(ctx(SvccError::new(
                    "",
                    format!("binding on '{inst}', which is not an instance"),
                )));
            }
        }
    }

    // The module's flag: any of its processes or children.
    let mut any = konst(Ty::BIT, 0);
    for f in &flags {
        let s = Expr::Sig {
            name: f.clone(),
            ty: Ty::BIT,
        };
        any = if any.as_u64() == Some(0) {
            s
        } else {
            or(any, s)
        };
    }
    m.ports.push(Port {
        name: UB_PORT.to_string(),
        dir: Dir::Out,
        ty: Ty::BIT,
        scalar: true,
        dims: Vec::new(),
    });
    m.processes.push(Process {
        name: format!("{UB_PORT}_all"),
        kind: ProcKind::Comb,
        locals: Vec::new(),
        body: vec![assign_sig(UB_PORT, conv(any, Ty::BIT), false)],
        merged_from: 1,
        line: 0,
    });
    Ok(m)
}

fn count_decls(ss: &[SAst], name: &str) -> usize {
    ss.iter()
        .map(|s| match s {
            SAst::Decl { name: n, .. } => (n == name) as usize,
            SAst::If { then_s, else_s, .. } => {
                count_decls(then_s, name) + count_decls(else_s, name)
            }
            SAst::For {
                init, step, body, ..
            } => count_decls(init, name) + count_decls(step, name) + count_decls(body, name),
            SAst::Switch { arms, default, .. } => {
                arms.iter().map(|a| count_decls(&a.1, name)).sum::<usize>()
                    + count_decls(default, name)
            }
            SAst::Block(b) => count_decls(b, name),
            _ => 0,
        })
        .sum()
}

/// Lifts every `SC_MODULE` of a generated header. Modules must appear before the modules
/// that instantiate them (as `svcc rtl2sc` writes them).
pub fn lift_hdl_design(code: &str) -> Result<Design, SvccError> {
    lift(code, true)
}

/// As [`lift_hdl_design`], for a header that holds other declarations too (the UVM prelude
/// of a mixed file): everything at file scope that is not an `SC_MODULE` is skipped.
pub fn lift_hdl_modules(code: &str) -> Result<Design, SvccError> {
    lift(code, false)
}

fn lift(code: &str, only_modules: bool) -> Result<Design, SvccError> {
    let mut p = Parser::new(lex(code)?);
    let mut design = Design::default();
    let mut known: HashMap<String, PortMap> = HashMap::new();
    let mut instantiated = BTreeSet::new();
    while !p.at_end() {
        if !p.eat_id("SC_MODULE") {
            if only_modules {
                return p.fail("only SC_MODULE definitions are expected at file scope");
            }
            // Skip one token, or one whole braced block.
            if p.eat_p("{") {
                let mut depth = 1;
                while depth > 0 && !p.at_end() {
                    if p.eat_p("{") {
                        depth += 1;
                    } else if p.eat_p("}") {
                        depth -= 1;
                    } else {
                        p.i += 1;
                    }
                }
            } else {
                p.i += 1;
            }
            continue;
        }
        let src = parse_module(&mut p)?;
        if known.contains_key(&src.name) {
            return err(format!("module '{}' defined twice", src.name));
        }
        let m = lift_module(&src, &known)?;
        known.insert(
            src.name.clone(),
            src.sigs
                .iter()
                .filter(|s| s.dir.is_some())
                .map(|s| (s.c.clone(), (s.name.clone(), s.dim)))
                .collect(),
        );
        instantiated.extend(src.insts.iter().map(|x| x.0.clone()));
        design.modules.push(m);
    }
    if design.modules.is_empty() {
        return err("no SC_MODULE found");
    }
    design.tops = design
        .modules
        .iter()
        .map(|m| m.name.clone())
        .filter(|n| !instantiated.contains(n))
        .collect();
    Ok(design)
}

/// Lifts the first function marked `// svcc:c2rtl` (an untimed golden model) as a
/// combinational module: one input port per parameter, the result on output `out_port`,
/// and the undefined-behaviour flag on [`UB_PORT`].
pub fn lift_c2rtl_function(code: &str, out_port: &str) -> Result<Design, SvccError> {
    let Some(at) = code.find("// svcc:c2rtl") else {
        return err("no function marked `// svcc:c2rtl`");
    };
    let mut p = Parser::new(lex(&code[at..])?);
    let Some(ret) = p.try_type()? else {
        return p.fail("return type of the golden function");
    };
    let name = p.ident()?;
    p.expect_p("(")?;
    let mut params = Vec::new();
    if !p.eat_p(")") {
        loop {
            let Some(t) = p.try_type()? else {
                return p.fail("parameter type");
            };
            params.push((t, p.ident()?));
            if p.eat_p(")") {
                break;
            }
            p.expect_p(",")?;
        }
    }
    p.expect_p("{")?;
    let body = p.block_body()?;
    let ctx = |e: SvccError| {
        SvccError::new(
            "svcc-front-sc/hdl-lift",
            format!("golden function '{name}': {e}"),
        )
    };
    if params
        .iter()
        .any(|x| x.1 == out_port || x.1.starts_with("svcc__"))
    {
        return Err(ctx(SvccError::new(
            "",
            "parameter name clashes with a port",
        )));
    }

    // The function itself sees no signals.
    let no_sigs = HashMap::new();
    let mut funcs = HashMap::new();
    funcs.insert(
        name.clone(),
        FnSig {
            name: name.clone(),
            ret,
            args: params.iter().map(|x| x.0).collect(),
        },
    );
    let no_funcs = HashMap::new();
    let mut b = Body::new(&no_sigs, &no_funcs, false, Some(ret));
    for (t, n) in &params {
        b.declare(n, *t);
    }
    let args = std::mem::take(&mut b.locals);
    let fbody = b.stmts(&body).map_err(ctx)?;
    if !matches!(fbody.last(), Some(Stmt::Return(_))) {
        return Err(ctx(SvccError::new(
            "",
            "the function does not end in a return",
        )));
    }
    let mut locals = std::mem::take(&mut b.locals);
    locals.push(LocalVar {
        name: UB_LOCAL.to_string(),
        ty: Ty::BIT,
    });
    let rw = cty_ty(ret).w;
    let packed = Ty::unsigned(rw + 1);
    let function = Function {
        name: name.clone(),
        ret: packed,
        args,
        locals,
        body: fbody,
    };

    let mut ports: Vec<Port> = params
        .iter()
        .map(|(t, n)| Port {
            name: n.clone(),
            dir: Dir::In,
            ty: cty_ty(*t),
            scalar: *t == CTy::Bool,
            dims: Vec::new(),
        })
        .collect();
    ports.push(Port {
        name: out_port.to_string(),
        dir: Dir::Out,
        ty: Ty::unsigned(rw),
        scalar: ret == CTy::Bool,
        dims: Vec::new(),
    });
    ports.push(Port {
        name: UB_PORT.to_string(),
        dir: Dir::Out,
        ty: Ty::BIT,
        scalar: true,
        dims: Vec::new(),
    });
    let call = Expr::Call {
        func: name.clone(),
        args: params
            .iter()
            .map(|(t, n)| Expr::Sig {
                name: n.clone(),
                ty: cty_ty(*t),
            })
            .collect(),
        ty: packed,
    };
    let part = |lo: u32, w: u32| Expr::Slice {
        base: Box::new(local("svcc__r", packed)),
        lo,
        lo_dyn: None,
        ty: Ty::unsigned(w),
    };
    let out_sig = Stmt::Assign {
        lhs: Lval {
            base: LvBase::Sig {
                name: out_port.to_string(),
                ty: Ty::unsigned(rw),
            },
            sel: None,
        },
        rhs: part(0, rw),
        nonblocking: false,
    };
    let process = Process {
        name: "svcc__call".to_string(),
        kind: ProcKind::Comb,
        locals: vec![LocalVar {
            name: "svcc__r".to_string(),
            ty: packed,
        }],
        body: vec![
            Stmt::Assign {
                lhs: Lval {
                    base: LvBase::Local {
                        name: "svcc__r".to_string(),
                        ty: packed,
                    },
                    sel: None,
                },
                rhs: call,
                nonblocking: false,
            },
            out_sig,
            assign_sig(UB_PORT, part(rw, 1), false),
        ],
        merged_from: 1,
        line: 0,
    };
    let _ = funcs;
    Ok(Design {
        modules: vec![Module {
            name: name.clone(),
            definition: name.clone(),
            params: Vec::new(),
            ports,
            signals: Vec::new(),
            functions: vec![function],
            processes: vec![process],
            instances: Vec::new(),
            file: String::new(),
            line: 0,
        }],
        tops: vec![name],
        notes: Vec::new(),
    })
}
