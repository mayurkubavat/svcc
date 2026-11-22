//! Lexer, syntax tree and parser for the C++ subset that `svcc` itself generates (and that a
//! `// svcc:c2rtl` golden function may use). The parser is strict: anything outside the
//! subset is an error, so a construct this reader does not model is never guessed at.

use svcc_ir::SvccError;

const STAGE: &str = "svcc-front-sc/cxx";

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T, SvccError> {
    Err(SvccError::new(STAGE, msg.into()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
    Id(String),
    Num(String),
    Str(String),
    P(String),
}

pub(crate) fn lex(src: &str) -> Result<Vec<Tok>, SvccError> {
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let mut line_start = true;
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '#' && line_start {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        line_start = false;
        if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            loop {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                    s.push(b[i]);
                    i += 1;
                }
                let next_is_id = b
                    .get(i + 2)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_');
                if b.get(i) == Some(&':') && b.get(i + 1) == Some(&':') && next_is_id {
                    s.push_str("::");
                    i += 2;
                } else {
                    break;
                }
            }
            out.push(Tok::Id(s));
            continue;
        }
        if c.is_ascii_digit() {
            let mut s = String::new();
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '\'') {
                if b[i] != '\'' {
                    s.push(b[i]);
                }
                i += 1;
            }
            out.push(Tok::Num(s));
            continue;
        }
        if c == '"' {
            let mut s = String::new();
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    return err("escape in a string literal");
                }
                s.push(b[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
            continue;
        }
        let two: String = b[i..(i + 2).min(b.len())].iter().collect();
        const TWO: &[&str] = &[
            "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=",
            "%=", "&=", "|=", "^=", "->",
        ];
        if TWO.contains(&two.as_str()) {
            if matches!(b.get(i + 2), Some('=')) && (two == "<<" || two == ">>") {
                return err("compound shift assignment");
            }
            out.push(Tok::P(two));
            i += 2;
            continue;
        }
        if "(){}[]<>;,.?:=+-*/%&|^~!".contains(c) {
            out.push(Tok::P(c.to_string()));
            i += 1;
            continue;
        }
        return err(format!("character '{c}' outside the supported C++ subset"));
    }
    Ok(out)
}

/// A C++ type the reader models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CTy {
    Bool,
    /// Built-in integer: `w` is 8, 16, 32 or 64.
    Int {
        w: u32,
        s: bool,
    },
    /// `sc_uint<w>` / `sc_int<w>`.
    Sc {
        w: u32,
        s: bool,
    },
    /// `sc_biguint<w>` / `sc_bigint<w>`.
    Big {
        w: u32,
        s: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ast {
    /// Integer literal with the type its spelling gives it.
    Num {
        v: u64,
        ty: CTy,
    },
    Bool(bool),
    Id(String),
    Un(String, Box<Ast>),
    Bin(String, Box<Ast>, Box<Ast>),
    Cond(Box<Ast>, Box<Ast>, Box<Ast>),
    /// `f(args)`.
    Call(String, Vec<Ast>),
    /// `T(arg)`.
    Cast(CTy, Box<Ast>),
    /// `obj.name(args)`.
    Method(Box<Ast>, String, Vec<Ast>),
    Index(Box<Ast>, Box<Ast>),
    /// `(a, b, ...)`: SystemC concatenation, most significant part first.
    Comma(Vec<Ast>),
}

impl Ast {
    /// Calls `f` with every identifier used (variables, called functions, method receivers).
    pub(crate) fn idents(&self, f: &mut dyn FnMut(&str)) {
        match self {
            Ast::Num { .. } | Ast::Bool(_) => {}
            Ast::Id(n) => f(n),
            Ast::Un(_, a) | Ast::Cast(_, a) => a.idents(f),
            Ast::Bin(_, a, b) | Ast::Index(a, b) => {
                a.idents(f);
                b.idents(f);
            }
            Ast::Cond(a, b, c) => {
                a.idents(f);
                b.idents(f);
                c.idents(f);
            }
            Ast::Call(n, args) => {
                f(n);
                args.iter().for_each(|a| a.idents(f));
            }
            Ast::Method(o, _, args) => {
                o.idents(f);
                args.iter().for_each(|a| a.idents(f));
            }
            Ast::Comma(ps) => ps.iter().for_each(|a| a.idents(f)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SAst {
    Decl {
        ty: CTy,
        name: String,
        array: Option<u64>,
        init: Option<Ast>,
    },
    Assign {
        lhs: Ast,
        rhs: Ast,
    },
    /// `target.write(value)`.
    Write {
        target: Ast,
        value: Ast,
    },
    If {
        cond: Ast,
        then_s: Vec<SAst>,
        else_s: Vec<SAst>,
    },
    For {
        init: Vec<SAst>,
        cond: Ast,
        step: Vec<SAst>,
        body: Vec<SAst>,
    },
    Switch {
        sel: Ast,
        arms: Vec<(Vec<u64>, Vec<SAst>)>,
        default: Vec<SAst>,
    },
    Block(Vec<SAst>),
    Return(Ast),
}

impl SAst {
    pub(crate) fn idents(&self, f: &mut dyn FnMut(&str)) {
        match self {
            SAst::Decl { name, init, .. } => {
                f(name);
                if let Some(i) = init {
                    i.idents(f);
                }
            }
            SAst::Assign { lhs, rhs } => {
                lhs.idents(f);
                rhs.idents(f);
            }
            SAst::Write { target, value } => {
                target.idents(f);
                value.idents(f);
            }
            SAst::If {
                cond,
                then_s,
                else_s,
            } => {
                cond.idents(f);
                then_s.iter().for_each(|s| s.idents(f));
                else_s.iter().for_each(|s| s.idents(f));
            }
            SAst::For {
                init,
                cond,
                step,
                body,
            } => {
                init.iter().for_each(|s| s.idents(f));
                cond.idents(f);
                step.iter().for_each(|s| s.idents(f));
                body.iter().for_each(|s| s.idents(f));
            }
            SAst::Switch { sel, arms, default } => {
                sel.idents(f);
                arms.iter()
                    .for_each(|a| a.1.iter().for_each(|s| s.idents(f)));
                default.iter().for_each(|s| s.idents(f));
            }
            SAst::Block(b) => b.iter().for_each(|s| s.idents(f)),
            SAst::Return(e) => e.idents(f),
        }
    }
}

pub(crate) struct Parser {
    pub(crate) t: Vec<Tok>,
    pub(crate) i: usize,
}

impl Parser {
    pub(crate) fn new(t: Vec<Tok>) -> Self {
        Parser { t, i: 0 }
    }

    pub(crate) fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    pub(crate) fn at_end(&self) -> bool {
        self.i >= self.t.len()
    }

    pub(crate) fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(x)) if x == p)
    }

    pub(crate) fn is_id(&self, id: &str) -> bool {
        matches!(self.peek(), Some(Tok::Id(x)) if x == id)
    }

    pub(crate) fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    pub(crate) fn eat_id(&mut self, id: &str) -> bool {
        if self.is_id(id) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn context(&self) -> String {
        let lo = self.i.saturating_sub(4);
        let hi = (self.i + 4).min(self.t.len());
        self.t[lo..hi]
            .iter()
            .map(|t| match t {
                Tok::Id(s) | Tok::Num(s) | Tok::P(s) => s.clone(),
                Tok::Str(s) => format!("\"{s}\""),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(crate) fn fail<T>(&self, what: &str) -> Result<T, SvccError> {
        err(format!("{what} near `{}`", self.context()))
    }

    pub(crate) fn expect_p(&mut self, p: &str) -> Result<(), SvccError> {
        if self.eat_p(p) {
            Ok(())
        } else {
            self.fail(&format!("expected `{p}`"))
        }
    }

    /// `>` that may be the first half of a `>>` token (closing nested templates).
    pub(crate) fn expect_gt(&mut self) -> Result<(), SvccError> {
        if self.eat_p(">") {
            return Ok(());
        }
        if self.is_p(">>") {
            self.t[self.i] = Tok::P(">".to_string());
            return Ok(());
        }
        self.fail("expected `>`")
    }

    pub(crate) fn ident(&mut self) -> Result<String, SvccError> {
        match self.peek() {
            Some(Tok::Id(s)) => {
                let s = s.clone();
                self.i += 1;
                Ok(s)
            }
            _ => self.fail("expected an identifier"),
        }
    }

    pub(crate) fn string(&mut self) -> Result<String, SvccError> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                let s = s.clone();
                self.i += 1;
                Ok(s)
            }
            _ => self.fail("expected a string literal"),
        }
    }

    pub(crate) fn number(&mut self) -> Result<u64, SvccError> {
        match self.peek() {
            Some(Tok::Num(_)) => match self.primary()? {
                Ast::Num { v, .. } => Ok(v),
                _ => self.fail("expected a number"),
            },
            _ => self.fail("expected a number"),
        }
    }

    /// Parses a type if one starts here. Leaves the position unchanged otherwise.
    pub(crate) fn try_type(&mut self) -> Result<Option<CTy>, SvccError> {
        let save = self.i;
        while self.eat_id("const") {}
        let Some(Tok::Id(id)) = self.peek().cloned() else {
            self.i = save;
            return Ok(None);
        };
        let ty = match id.as_str() {
            "bool" => {
                self.i += 1;
                CTy::Bool
            }
            "uint8_t" | "int8_t" | "uint16_t" | "int16_t" => {
                self.i += 1;
                CTy::Int {
                    w: if id.ends_with("8_t") { 8 } else { 16 },
                    s: !id.starts_with('u'),
                }
            }
            "int" | "int32_t" => {
                self.i += 1;
                CTy::Int { w: 32, s: true }
            }
            "uint32_t" => {
                self.i += 1;
                CTy::Int { w: 32, s: false }
            }
            "uint64_t" | "sc_dt::uint64" => {
                self.i += 1;
                CTy::Int { w: 64, s: false }
            }
            "int64_t" | "sc_dt::int64" => {
                self.i += 1;
                CTy::Int { w: 64, s: true }
            }
            "unsigned" | "signed" | "long" => {
                let mut s = true;
                let mut longs = 0;
                loop {
                    if self.eat_id("unsigned") {
                        s = false;
                    } else if self.eat_id("signed") || self.eat_id("int") {
                    } else if self.eat_id("long") {
                        longs += 1;
                    } else {
                        break;
                    }
                }
                // `long` is 64 bits on the LP64 targets svcc supports.
                CTy::Int {
                    w: if longs > 0 { 64 } else { 32 },
                    s,
                }
            }
            "sc_dt::sc_uint" | "sc_dt::sc_int" | "sc_dt::sc_biguint" | "sc_dt::sc_bigint" => {
                self.i += 1;
                self.expect_p("<")?;
                let w = self.number()? as u32;
                self.expect_gt()?;
                if w == 0 {
                    return self.fail("zero-width vector");
                }
                match id.as_str() {
                    "sc_dt::sc_uint" | "sc_dt::sc_int" => {
                        if w > 64 {
                            return self.fail("sc_uint/sc_int wider than 64 bits");
                        }
                        CTy::Sc {
                            w,
                            s: id == "sc_dt::sc_int",
                        }
                    }
                    _ => CTy::Big {
                        w,
                        s: id == "sc_dt::sc_bigint",
                    },
                }
            }
            _ => {
                self.i = save;
                return Ok(None);
            }
        };
        Ok(Some(ty))
    }

    pub(crate) fn expr(&mut self) -> Result<Ast, SvccError> {
        let c = self.binary(0)?;
        if self.eat_p("?") {
            let a = self.expr()?;
            self.expect_p(":")?;
            let b = self.expr()?;
            return Ok(Ast::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn binary(&mut self, level: usize) -> Result<Ast, SvccError> {
        const LEVELS: &[&[&str]] = &[
            &["||"],
            &["&&"],
            &["|"],
            &["^"],
            &["&"],
            &["==", "!="],
            &["<", "<=", ">", ">="],
            &["<<", ">>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        loop {
            let op = match self.peek() {
                Some(Tok::P(p)) if LEVELS[level].contains(&p.as_str()) => p.clone(),
                _ => return Ok(lhs),
            };
            self.i += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn unary(&mut self) -> Result<Ast, SvccError> {
        for op in ["-", "~", "!"] {
            if self.eat_p(op) {
                let a = self.unary()?;
                return Ok(Ast::Un(op.to_string(), Box::new(a)));
            }
        }
        if self.is_p("++") || self.is_p("--") || self.is_p("*") || self.is_p("&") {
            return self.fail("unsupported unary operator");
        }
        self.postfix()
    }

    fn args(&mut self) -> Result<Vec<Ast>, SvccError> {
        let mut args = Vec::new();
        if self.eat_p(")") {
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            if self.eat_p(")") {
                return Ok(args);
            }
            self.expect_p(",")?;
        }
    }

    fn postfix(&mut self) -> Result<Ast, SvccError> {
        let mut e = self.primary()?;
        loop {
            if self.eat_p("[") {
                let i = self.expr()?;
                self.expect_p("]")?;
                e = Ast::Index(Box::new(e), Box::new(i));
            } else if self.eat_p(".") {
                let name = self.ident()?;
                self.expect_p("(")?;
                let args = self.args()?;
                e = Ast::Method(Box::new(e), name, args);
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Ast, SvccError> {
        if let Some(ty) = self.try_type()? {
            self.expect_p("(")?;
            let a = self.paren_body()?;
            return Ok(Ast::Cast(ty, Box::new(a)));
        }
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                number(&n)
                    .ok_or(())
                    .or_else(|_| self.fail("bad integer literal"))
            }
            Some(Tok::Id(id)) => {
                self.i += 1;
                match id.as_str() {
                    "true" => return Ok(Ast::Bool(true)),
                    "false" => return Ok(Ast::Bool(false)),
                    _ => {}
                }
                if self.eat_p("(") {
                    let args = self.args()?;
                    Ok(Ast::Call(id, args))
                } else {
                    Ok(Ast::Id(id))
                }
            }
            Some(Tok::P(p)) if p == "(" => {
                self.i += 1;
                self.paren_body()
            }
            _ => self.fail("expected an expression"),
        }
    }

    /// The inside of `( ... )` after the opening parenthesis, through the closing one.
    fn paren_body(&mut self) -> Result<Ast, SvccError> {
        let first = self.expr()?;
        if self.eat_p(")") {
            return Ok(first);
        }
        let mut parts = vec![first];
        while self.eat_p(",") {
            parts.push(self.expr()?);
        }
        self.expect_p(")")?;
        Ok(Ast::Comma(parts))
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    /// Statements up to (and consuming) the closing `}`.
    pub(crate) fn block_body(&mut self) -> Result<Vec<SAst>, SvccError> {
        let mut out = Vec::new();
        while !self.eat_p("}") {
            if self.at_end() {
                return self.fail("unterminated block");
            }
            self.stmt(&mut out)?;
        }
        Ok(out)
    }

    /// One statement, or a `{ ... }` block as a single nested entry.
    fn stmt_or_block(&mut self) -> Result<Vec<SAst>, SvccError> {
        if self.eat_p("{") {
            return self.block_body();
        }
        let mut out = Vec::new();
        self.stmt(&mut out)?;
        Ok(out)
    }

    /// A declaration, assignment, increment or `.write` call, without the trailing `;`.
    fn simple(&mut self, out: &mut Vec<SAst>) -> Result<(), SvccError> {
        if let Some(ty) = self.try_type()? {
            let name = self.ident()?;
            let mut array = None;
            if self.eat_p("[") {
                array = Some(self.number()?);
                self.expect_p("]")?;
            }
            let init = if self.eat_p("=") {
                Some(self.expr()?)
            } else {
                None
            };
            out.push(SAst::Decl {
                ty,
                name,
                array,
                init,
            });
            if self.is_p(",") {
                return self.fail("several declarators in one declaration");
            }
            return Ok(());
        }
        if self.eat_p("++") || self.is_p("--") {
            if self.is_p("--") {
                return self.fail("decrement");
            }
            let name = self.ident()?;
            out.push(incr(&name));
            return Ok(());
        }
        let lhs = self.postfix()?;
        if self.eat_p("++") {
            let Ast::Id(name) = lhs else {
                return self.fail("increment of something that is not a variable");
            };
            out.push(incr(&name));
            return Ok(());
        }
        if self.eat_p("=") {
            let rhs = self.expr()?;
            out.push(SAst::Assign { lhs, rhs });
            return Ok(());
        }
        // `x op= e` is `x = x op (e)`: the operation is carried out in the common type of
        // `x` and `e`, and the result converted back to the type of `x`.
        for op in ["+=", "-=", "*=", "/=", "%=", "&=", "|=", "^="] {
            if self.eat_p(op) {
                if !matches!(lhs, Ast::Id(_)) {
                    return self.fail("compound assignment to something that is not a variable");
                }
                let rhs = self.expr()?;
                out.push(SAst::Assign {
                    lhs: lhs.clone(),
                    rhs: Ast::Bin(op[..op.len() - 1].to_string(), Box::new(lhs), Box::new(rhs)),
                });
                return Ok(());
            }
        }
        if let Ast::Method(target, name, mut args) = lhs {
            if name == "write" && args.len() == 1 {
                out.push(SAst::Write {
                    target: *target,
                    value: args.remove(0),
                });
                return Ok(());
            }
        }
        self.fail("statement is not a declaration, assignment or write")
    }

    fn simple_list(&mut self, end: &str) -> Result<Vec<SAst>, SvccError> {
        let mut out = Vec::new();
        if self.eat_p(end) {
            return Ok(out);
        }
        loop {
            self.simple(&mut out)?;
            if self.eat_p(end) {
                return Ok(out);
            }
            self.expect_p(",")?;
        }
    }

    pub(crate) fn stmt(&mut self, out: &mut Vec<SAst>) -> Result<(), SvccError> {
        if self.eat_p(";") {
            return Ok(());
        }
        if self.eat_p("{") {
            let b = self.block_body()?;
            out.push(SAst::Block(b));
            return Ok(());
        }
        if self.eat_id("if") {
            self.expect_p("(")?;
            let cond = self.expr()?;
            self.expect_p(")")?;
            let then_s = self.stmt_or_block()?;
            let else_s = if self.eat_id("else") {
                self.stmt_or_block()?
            } else {
                Vec::new()
            };
            out.push(SAst::If {
                cond,
                then_s,
                else_s,
            });
            return Ok(());
        }
        if self.eat_id("for") {
            self.expect_p("(")?;
            let init = self.simple_list(";")?;
            let cond = self.expr()?;
            self.expect_p(";")?;
            let step = self.simple_list(")")?;
            let body = self.stmt_or_block()?;
            out.push(SAst::For {
                init,
                cond,
                step,
                body,
            });
            return Ok(());
        }
        if self.eat_id("switch") {
            self.expect_p("(")?;
            let sel = self.expr()?;
            self.expect_p(")")?;
            self.expect_p("{")?;
            let mut arms = Vec::new();
            let mut default = Vec::new();
            let mut seen_default = false;
            while !self.eat_p("}") {
                let mut labels = Vec::new();
                let mut is_default = false;
                loop {
                    if self.eat_id("case") {
                        labels.push(self.number()?);
                        self.expect_p(":")?;
                    } else if self.eat_id("default") {
                        self.expect_p(":")?;
                        is_default = true;
                    } else {
                        break;
                    }
                }
                if labels.is_empty() && !is_default {
                    return self.fail("expected `case` or `default`");
                }
                if is_default && (!labels.is_empty() || seen_default) {
                    return self.fail("`default` sharing a body with `case` labels");
                }
                // Every body ends in `break;`: no fall-through to model.
                let mut body = Vec::new();
                loop {
                    if self.eat_id("break") {
                        self.expect_p(";")?;
                        break;
                    }
                    if self.is_id("case") || self.is_id("default") || self.is_p("}") {
                        return self.fail("switch body without `break`");
                    }
                    self.stmt(&mut body)?;
                }
                if is_default {
                    seen_default = true;
                    default = body;
                } else {
                    arms.push((labels, body));
                }
            }
            out.push(SAst::Switch { sel, arms, default });
            return Ok(());
        }
        if self.eat_id("return") {
            let e = self.expr()?;
            self.expect_p(";")?;
            out.push(SAst::Return(e));
            return Ok(());
        }
        for kw in ["while", "do", "goto", "break", "continue", "throw", "try"] {
            if self.is_id(kw) {
                return self.fail(&format!("`{kw}` statement"));
            }
        }
        self.simple(out)?;
        self.expect_p(";")
    }
}

fn incr(name: &str) -> SAst {
    SAst::Assign {
        lhs: Ast::Id(name.to_string()),
        rhs: Ast::Bin(
            "+".to_string(),
            Box::new(Ast::Id(name.to_string())),
            Box::new(Ast::Num {
                v: 1,
                ty: CTy::Int { w: 32, s: true },
            }),
        ),
    }
}

/// Value and C++ type of an integer literal (LP64).
fn number(s: &str) -> Option<Ast> {
    let lower = s.to_ascii_lowercase();
    let (digits, radix) = match lower.strip_prefix("0x") {
        Some(d) => (d.to_string(), 16),
        None => (lower.clone(), 10),
    };
    let body: String = digits.chars().take_while(|c| c.is_digit(radix)).collect();
    let suffix = &digits[body.len()..];
    if body.is_empty() || (radix == 10 && body.len() > 1 && body.starts_with('0')) {
        return None;
    }
    let v = u64::from_str_radix(&body, radix).ok()?;
    let (unsigned, long) = match suffix {
        "" => (false, false),
        "u" => (true, false),
        "l" | "ll" => (false, true),
        "ul" | "ull" | "lu" | "llu" => (true, true),
        _ => return None,
    };
    let fits = |w: u32, s: bool| -> bool {
        match (w, s) {
            (32, true) => v <= i32::MAX as u64,
            (32, false) => v <= u32::MAX as u64,
            (64, true) => v <= i64::MAX as u64,
            _ => true,
        }
    };
    // The first type of the standard's list that can hold the value.
    let candidates: &[(u32, bool)] = match (unsigned, long, radix) {
        (false, false, 10) => &[(32, true), (64, true)],
        (false, false, _) => &[(32, true), (32, false), (64, true), (64, false)],
        (true, false, _) => &[(32, false), (64, false)],
        (false, true, 10) => &[(64, true)],
        (false, true, _) => &[(64, true), (64, false)],
        (true, true, _) => &[(64, false)],
    };
    let (w, sg) = candidates.iter().copied().find(|(w, s)| fits(*w, *s))?;
    Some(Ast::Num {
        v,
        ty: CTy::Int { w, s: sg },
    })
}
