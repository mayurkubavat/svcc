//! Pratt / recursive-descent parser lowering SV constraint item clauses into [`CrvArena`].
//!
//! Supports IEEE 1800-2023 constraint expressions:
//! - `soft <expr>`
//! - `unique { a, b, c }`
//! - Implication `a -> b`, `if (a) b`, and equivalence `a <-> b`
//! - `x inside { v, [lo:hi], [center +/- delta], [center +%- pct] }`
//! - `x dist { v := w, [lo:hi] :/ w }`
//! - Enum enumerator resolution (`AXI_INCR` -> typed constant)
//! - Logical, bitwise, relational, shift, and arithmetic operators with IEEE 1800 precedence

use svcc_ir::{
    ConstraintClause, CrvArena, CrvBinaryOp, CrvExpr, CrvUnaryOp, DistItem, DistWeightKind, ExprId,
    InsideElem, UvirError,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Lit {
        value: u128,
        width: u32,
        signed: bool,
    },
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    DistPerVal, // :=
    DistSplit,  // :/
    PlusMinus,  // +/-
    PlusPct,    // +%-
    Arrow,      // ->
    Iff,        // <->
    OrOr,
    AndAnd,
    EqEq,
    NotEq,
    Le,
    Ge,
    Lt,
    Gt,
    Shl,
    Shr,
    BitOr,
    BitXor,
    BitAnd,
    Plus,
    Minus,
    Star,
    Percent,
    Bang,
    Tilde,
}

fn tokenize_sv_expr(src: &str) -> Result<Vec<Tok>, UvirError> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() || b == b';' {
            i += 1;
            continue;
        }
        if src[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if src[i..].starts_with("<->") {
            out.push(Tok::Iff);
            i += 3;
            continue;
        }
        if src[i..].starts_with("+/-") {
            out.push(Tok::PlusMinus);
            i += 3;
            continue;
        }
        if src[i..].starts_with("+%-") {
            out.push(Tok::PlusPct);
            i += 3;
            continue;
        }
        if src[i..].starts_with(":=") {
            out.push(Tok::DistPerVal);
            i += 2;
            continue;
        }
        if src[i..].starts_with(":/") {
            out.push(Tok::DistSplit);
            i += 2;
            continue;
        }
        if src[i..].starts_with("->") {
            out.push(Tok::Arrow);
            i += 2;
            continue;
        }
        if src[i..].starts_with("||") {
            out.push(Tok::OrOr);
            i += 2;
            continue;
        }
        if src[i..].starts_with("&&") {
            out.push(Tok::AndAnd);
            i += 2;
            continue;
        }
        if src[i..].starts_with("==") {
            out.push(Tok::EqEq);
            i += 2;
            continue;
        }
        if src[i..].starts_with("!=") {
            out.push(Tok::NotEq);
            i += 2;
            continue;
        }
        if src[i..].starts_with("<=") {
            out.push(Tok::Le);
            i += 2;
            continue;
        }
        if src[i..].starts_with(">=") {
            out.push(Tok::Ge);
            i += 2;
            continue;
        }
        if src[i..].starts_with("<<") {
            out.push(Tok::Shl);
            i += 2;
            continue;
        }
        if src[i..].starts_with(">>") {
            out.push(Tok::Shr);
            i += 2;
            continue;
        }

        match b {
            b'(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            b')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            b'{' => {
                out.push(Tok::LBrace);
                i += 1;
            }
            b'}' => {
                out.push(Tok::RBrace);
                i += 1;
            }
            b'[' => {
                out.push(Tok::LBracket);
                i += 1;
            }
            b']' => {
                out.push(Tok::RBracket);
                i += 1;
            }
            b',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            b':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            b'<' => {
                out.push(Tok::Lt);
                i += 1;
            }
            b'>' => {
                out.push(Tok::Gt);
                i += 1;
            }
            b'|' => {
                out.push(Tok::BitOr);
                i += 1;
            }
            b'^' => {
                out.push(Tok::BitXor);
                i += 1;
            }
            b'&' => {
                out.push(Tok::BitAnd);
                i += 1;
            }
            b'+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            b'-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            b'*' => {
                out.push(Tok::Star);
                i += 1;
            }
            b'%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            b'!' => {
                out.push(Tok::Bang);
                i += 1;
            }
            b'~' => {
                out.push(Tok::Tilde);
                i += 1;
            }
            b'\'' | b'0'..=b'9' => {
                let start = i;
                if bytes[i] == b'\'' {
                    i += 1;
                }
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'\'')
                {
                    i += 1;
                }
                let raw = &src[start..i];
                let (value, width, signed) = parse_sv_literal(raw)?;
                out.push(Tok::Lit {
                    value,
                    width,
                    signed,
                });
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'.'
                        || bytes[i] == b':')
                {
                    // Stop before `:=` or `:/` or single `:`
                    if bytes[i] == b':' && src.as_bytes().get(i + 1) != Some(&b':') {
                        break;
                    }
                    if bytes[i] == b':' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                out.push(Tok::Ident(src[start..i].to_string()));
            }
            other => {
                return Err(UvirError::new(
                    "svcc-front-sv/crv",
                    format!(
                        "unexpected character '{}' in constraint '{}'",
                        other as char, src
                    ),
                ));
            }
        }
    }
    Ok(out)
}

pub fn parse_sv_literal(raw: &str) -> Result<(u128, u32, bool), UvirError> {
    let clean = raw.trim().replace('_', "");
    if clean == "'0" {
        return Ok((0, 32, false));
    }
    if clean == "'1" {
        return Ok((u32::MAX as u128, 32, false));
    }
    if let Some(hex) = clean
        .strip_prefix("0x")
        .or_else(|| clean.strip_prefix("0X"))
    {
        let v = u128::from_str_radix(hex, 16).map_err(|e| {
            UvirError::new("svcc-front-sv/crv", format!("bad hex literal {raw}: {e}"))
        })?;
        return Ok((v, 32, false));
    }
    if let Some((w_str, rhs)) = clean.split_once('\'') {
        let width = if w_str.is_empty() {
            32
        } else {
            w_str.parse::<u32>().unwrap_or(32)
        };
        let signed = rhs.starts_with('s') || rhs.starts_with('S');
        let rhs = rhs.trim_start_matches(['s', 'S']);
        let (radix, digits) = match rhs.as_bytes().first().copied().unwrap_or(b'd') {
            b'h' | b'H' => (16, &rhs[1..]),
            b'd' | b'D' => (10, &rhs[1..]),
            b'o' | b'O' => (8, &rhs[1..]),
            b'b' | b'B' => (2, &rhs[1..]),
            _ => (10, rhs),
        };
        let v = u128::from_str_radix(digits, radix).map_err(|e| {
            UvirError::new("svcc-front-sv/crv", format!("bad SV literal {raw}: {e}"))
        })?;
        return Ok((v, width, signed));
    }
    let v = clean.parse::<u128>().map_err(|e| {
        UvirError::new(
            "svcc-front-sv/crv",
            format!("bad integer literal {raw}: {e}"),
        )
    })?;
    Ok((v, 32, false))
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    arena: &'a mut CrvArena,
    enum_consts: &'a [(String, u64, u32)],
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, expected: &Tok) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_inside_elem(&mut self) -> Result<InsideElem, UvirError> {
        if self.eat(&Tok::LBracket) {
            let a = self.parse_add()?;
            let elem = if self.eat(&Tok::Colon) {
                let b = self.parse_add()?;
                InsideElem::Range { low: a, high: b }
            } else if self.eat(&Tok::PlusMinus) {
                let d = self.parse_add()?;
                InsideElem::ToleranceAbs {
                    center: a,
                    delta: d,
                }
            } else if self.eat(&Tok::PlusPct) {
                let p = self.parse_add()?;
                InsideElem::TolerancePct { center: a, pct: p }
            } else {
                return Err(UvirError::new(
                    "svcc-front-sv/crv",
                    "expected ':', '+/-', or '+%-' in [...] range",
                ));
            };
            let _ = self.eat(&Tok::RBracket);
            Ok(elem)
        } else {
            let single = self.parse_add()?;
            Ok(InsideElem::Single(single))
        }
    }

    fn parse_clause(&mut self) -> Result<ConstraintClause, UvirError> {
        let mut is_soft = false;
        if matches!(self.peek(), Some(Tok::Ident(s)) if s == "soft") {
            self.bump();
            is_soft = true;
        }
        if matches!(self.peek(), Some(Tok::Ident(s)) if s == "unique") {
            self.bump();
            if !self.eat(&Tok::LBrace) {
                return Err(UvirError::new(
                    "svcc-front-sv/crv",
                    "expected '{' after unique",
                ));
            }
            let mut vars = Vec::new();
            while let Some(tok) = self.peek() {
                if *tok == Tok::RBrace {
                    break;
                }
                match self.bump() {
                    Some(Tok::Ident(name)) => vars.push(name.clone()),
                    other => {
                        return Err(UvirError::new(
                            "svcc-front-sv/crv",
                            format!("expected identifier in unique list, got {other:?}"),
                        ))
                    }
                }
                let _ = self.eat(&Tok::Comma);
            }
            let _ = self.eat(&Tok::RBrace);
            let expr = self.arena.alloc(CrvExpr::Unique { vars });
            return Ok(ConstraintClause {
                expr,
                is_soft,
                soft_weight: None,
            });
        }
        if matches!(self.peek(), Some(Tok::Ident(s)) if s == "if") {
            self.bump();
            let _ = self.eat(&Tok::LParen);
            let cond = self.parse_iff()?;
            let _ = self.eat(&Tok::RParen);
            let _ = self.eat(&Tok::LBrace);
            let body = self.parse_iff()?;
            let _ = self.eat(&Tok::RBrace);
            let expr = self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Implies,
                lhs: cond,
                rhs: body,
            });
            return Ok(ConstraintClause {
                expr,
                is_soft,
                soft_weight: None,
            });
        }

        let expr = self.parse_iff()?;
        Ok(ConstraintClause {
            expr,
            is_soft,
            soft_weight: is_soft.then_some(1),
        })
    }

    fn parse_iff(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_implies()?;
        while self.eat(&Tok::Iff) {
            let rhs = self.parse_implies()?;
            lhs = self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Iff,
                lhs,
                rhs,
            });
        }
        Ok(lhs)
    }

    fn parse_implies(&mut self) -> Result<ExprId, UvirError> {
        let lhs = self.parse_log_or()?;
        if self.eat(&Tok::Arrow) {
            let open_brace = self.eat(&Tok::LBrace);
            let rhs = self.parse_implies()?;
            if open_brace {
                let _ = self.eat(&Tok::RBrace);
            }
            return Ok(self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::Implies,
                lhs,
                rhs,
            }));
        }
        Ok(lhs)
    }

    fn parse_log_or(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_log_and()?;
        while self.eat(&Tok::OrOr) {
            let rhs = self.parse_log_and()?;
            lhs = self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::LogOr,
                lhs,
                rhs,
            });
        }
        Ok(lhs)
    }

    fn parse_log_and(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_bit_or()?;
        while self.eat(&Tok::AndAnd) {
            let rhs = self.parse_bit_or()?;
            lhs = self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::LogAnd,
                lhs,
                rhs,
            });
        }
        Ok(lhs)
    }

    fn parse_bit_or(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_bit_xor()?;
        while self.eat(&Tok::BitOr) {
            let rhs = self.parse_bit_xor()?;
            lhs = self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::BitOr,
                lhs,
                rhs,
            });
        }
        Ok(lhs)
    }

    fn parse_bit_xor(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_bit_and()?;
        while self.eat(&Tok::BitXor) {
            let rhs = self.parse_bit_and()?;
            lhs = self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::BitXor,
                lhs,
                rhs,
            });
        }
        Ok(lhs)
    }

    fn parse_bit_and(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_equality()?;
        while self.eat(&Tok::BitAnd) {
            let rhs = self.parse_equality()?;
            lhs = self.arena.alloc(CrvExpr::Binary {
                op: CrvBinaryOp::BitAnd,
                lhs,
                rhs,
            });
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_relational()?;
        loop {
            if self.eat(&Tok::EqEq) {
                let rhs = self.parse_relational()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Eq,
                    lhs,
                    rhs,
                });
            } else if self.eat(&Tok::NotEq) {
                let rhs = self.parse_relational()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Ne,
                    lhs,
                    rhs,
                });
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_relational(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_shift()?;
        loop {
            if matches!(self.peek(), Some(Tok::Ident(s)) if s == "inside") {
                self.bump();
                if !self.eat(&Tok::LBrace) {
                    return Err(UvirError::new(
                        "svcc-front-sv/crv",
                        "expected '{' after inside",
                    ));
                }
                let mut elems = Vec::new();
                while let Some(t) = self.peek() {
                    if *t == Tok::RBrace {
                        break;
                    }
                    elems.push(self.parse_inside_elem()?);
                    let _ = self.eat(&Tok::Comma);
                }
                let _ = self.eat(&Tok::RBrace);
                lhs = self.arena.alloc(CrvExpr::Inside { target: lhs, elems });
            } else if matches!(self.peek(), Some(Tok::Ident(s)) if s == "dist") {
                self.bump();
                if !self.eat(&Tok::LBrace) {
                    return Err(UvirError::new(
                        "svcc-front-sv/crv",
                        "expected '{' after dist",
                    ));
                }
                let mut items = Vec::new();
                while let Some(t) = self.peek() {
                    if *t == Tok::RBrace {
                        break;
                    }
                    let elem = self.parse_inside_elem()?;
                    let kind = if self.eat(&Tok::DistPerVal) {
                        DistWeightKind::PerValue
                    } else if self.eat(&Tok::DistSplit) {
                        DistWeightKind::SplitRange
                    } else {
                        DistWeightKind::PerValue
                    };
                    let weight = match self.bump() {
                        Some(Tok::Lit { value, .. }) => *value as u32,
                        _ => 1,
                    };
                    items.push(DistItem { elem, kind, weight });
                    let _ = self.eat(&Tok::Comma);
                }
                let _ = self.eat(&Tok::RBrace);
                lhs = self.arena.alloc(CrvExpr::Dist { target: lhs, items });
            } else if self.eat(&Tok::Le) {
                let rhs = self.parse_shift()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Le,
                    lhs,
                    rhs,
                });
            } else if self.eat(&Tok::Ge) {
                let rhs = self.parse_shift()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Ge,
                    lhs,
                    rhs,
                });
            } else if self.eat(&Tok::Lt) {
                let rhs = self.parse_shift()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Lt,
                    lhs,
                    rhs,
                });
            } else if self.eat(&Tok::Gt) {
                let rhs = self.parse_shift()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Gt,
                    lhs,
                    rhs,
                });
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_shift(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_add()?;
        loop {
            if self.eat(&Tok::Shl) {
                let rhs = self.parse_add()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Shl,
                    lhs,
                    rhs,
                });
            } else if self.eat(&Tok::Shr) {
                let rhs = self.parse_add()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Shr,
                    lhs,
                    rhs,
                });
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_mul()?;
        loop {
            if self.eat(&Tok::Plus) {
                let rhs = self.parse_mul()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Add,
                    lhs,
                    rhs,
                });
            } else if self.eat(&Tok::Minus) {
                let rhs = self.parse_mul()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Sub,
                    lhs,
                    rhs,
                });
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<ExprId, UvirError> {
        let mut lhs = self.parse_unary()?;
        loop {
            if self.eat(&Tok::Star) {
                let rhs = self.parse_unary()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Mul,
                    lhs,
                    rhs,
                });
            } else if self.eat(&Tok::Percent) {
                let rhs = self.parse_unary()?;
                lhs = self.arena.alloc(CrvExpr::Binary {
                    op: CrvBinaryOp::Rem,
                    lhs,
                    rhs,
                });
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<ExprId, UvirError> {
        if self.eat(&Tok::Bang) {
            let arg = self.parse_unary()?;
            return Ok(self.arena.alloc(CrvExpr::Unary {
                op: CrvUnaryOp::Not,
                arg,
            }));
        }
        if self.eat(&Tok::Tilde) {
            let arg = self.parse_unary()?;
            return Ok(self.arena.alloc(CrvExpr::Unary {
                op: CrvUnaryOp::BitNot,
                arg,
            }));
        }
        if self.eat(&Tok::Minus) {
            let arg = self.parse_unary()?;
            return Ok(self.arena.alloc(CrvExpr::Unary {
                op: CrvUnaryOp::Neg,
                arg,
            }));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<ExprId, UvirError> {
        match self.bump() {
            Some(Tok::LParen) => {
                let inner = self.parse_iff()?;
                if !self.eat(&Tok::RParen) {
                    return Err(UvirError::new("svcc-front-sv/crv", "missing closing ')'"));
                }
                Ok(inner)
            }
            Some(Tok::Lit {
                value,
                width,
                signed,
            }) => Ok(self.arena.alloc(CrvExpr::Const {
                value: *value,
                width: *width,
                signed: *signed,
            })),
            Some(Tok::Ident(s)) if s == "true" => Ok(self.arena.alloc(CrvExpr::BoolConst(true))),
            Some(Tok::Ident(s)) if s == "false" => Ok(self.arena.alloc(CrvExpr::BoolConst(false))),
            Some(Tok::Ident(name)) => {
                let short = name.rsplit("::").next().unwrap_or(name);
                if let Some(&(_, val, w)) = self.enum_consts.iter().find(|(n, _, _)| n == short) {
                    return Ok(self.arena.alloc(CrvExpr::Const {
                        value: val as u128,
                        width: w,
                        signed: false,
                    }));
                }
                Ok(self.arena.alloc(CrvExpr::Var(name.clone())))
            }
            other => Err(UvirError::new(
                "svcc-front-sv/crv",
                format!("unexpected token in SV constraint primary: {other:?}"),
            )),
        }
    }
}

/// Parses a single SV `ConstraintItem` text into a [`ConstraintClause`].
pub fn lower_sv_constraint_item(
    raw_item: &str,
    arena: &mut CrvArena,
    enum_consts: &[(String, u64, u32)],
) -> Result<Option<ConstraintClause>, UvirError> {
    let trimmed = raw_item.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() || trimmed.starts_with("solve ") || trimmed.starts_with("disable ") {
        return Ok(None);
    }
    let toks = tokenize_sv_expr(trimmed)?;
    if toks.is_empty() {
        return Ok(None);
    }
    let mut parser = Parser {
        toks: &toks,
        pos: 0,
        arena,
        enum_consts,
    };
    parser.parse_clause().map(Some)
}
