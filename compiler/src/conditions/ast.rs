// SPDX-License-Identifier: BUSL-1.1

use super::{
    is_ident_char, parse_ident, parse_operator, parse_quoted_literal, skip_ws, to_public_atom,
    ConditionAtom, ConditionPredicate,
};
use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConditionExpr {
    Bool(bool),
    Predicate(ConditionPredicate),
    Not(Box<ConditionExpr>),
    And(Box<ConditionExpr>, Box<ConditionExpr>),
    Or(Box<ConditionExpr>, Box<ConditionExpr>),
    /// Cardinality operator `any_of(e1, …, eN)` — OR-reduction over its
    /// children (ADR-0006 §4). Children are held in source order; parsing is
    /// added by configflux-ccs.3 and BDD lowering by configflux-ccs.4-6.
    AnyOf(Vec<ConditionExpr>),
    /// Cardinality operator `all_of(e1, …, eN)` — AND-reduction over its
    /// children (ADR-0006 §4). Children are held in source order.
    AllOf(Vec<ConditionExpr>),
    /// Cardinality operator `exactly_one_of(e1, …, eN)` — at-least-one (OR)
    /// conjoined with pairwise at-most-one (ADR-0006 §4). Children are held in
    /// source order.
    ExactlyOneOf(Vec<ConditionExpr>),
}

pub(crate) fn parse_condition_expr(condition: &str) -> Result<ConditionExpr> {
    let bytes = condition.as_bytes();
    let mut parser = AstParser { bytes, idx: 0 };
    let expr = parser.parse_or()?;
    parser.skip_ws();
    if parser.idx != bytes.len() {
        bail!("Unexpected trailing input at position {}", parser.idx);
    }
    Ok(expr)
}

struct AstParser<'a> {
    bytes: &'a [u8],
    idx: usize,
}

impl AstParser<'_> {
    fn parse_or(&mut self) -> Result<ConditionExpr> {
        let mut expr = self.parse_and()?;
        loop {
            self.skip_ws();
            if !self.consume_op2(b'|', b'|') {
                return Ok(expr);
            }
            let rhs = self.parse_and()?;
            expr = ConditionExpr::Or(Box::new(expr), Box::new(rhs));
        }
    }

    fn parse_and(&mut self) -> Result<ConditionExpr> {
        let mut expr = self.parse_unary()?;
        loop {
            self.skip_ws();
            if !self.consume_op2(b'&', b'&') {
                return Ok(expr);
            }
            let rhs = self.parse_unary()?;
            expr = ConditionExpr::And(Box::new(expr), Box::new(rhs));
        }
    }

    fn parse_unary(&mut self) -> Result<ConditionExpr> {
        self.skip_ws();
        if self.idx < self.bytes.len() && self.bytes[self.idx] == b'!' {
            if self.idx + 1 < self.bytes.len() && self.bytes[self.idx + 1] == b'=' {
                return self.parse_primary();
            }
            self.idx += 1;
            return Ok(ConditionExpr::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<ConditionExpr> {
        self.skip_ws();
        if self.idx >= self.bytes.len() {
            bail!("Unexpected end of condition");
        }
        if self.bytes[self.idx] == b'(' {
            self.idx += 1;
            let expr = self.parse_or()?;
            self.skip_ws();
            if self.idx >= self.bytes.len() || self.bytes[self.idx] != b')' {
                bail!("Missing closing parenthesis at position {}", self.idx);
            }
            self.idx += 1;
            return Ok(expr);
        }
        if let Some(value) = self.try_consume_bool_literal() {
            return Ok(ConditionExpr::Bool(value));
        }
        if let Some(expr) = self.try_parse_cardinality()? {
            return Ok(expr);
        }
        self.parse_predicate()
    }

    /// Recognise the grammar-v2 cardinality operators `any_of(...)`,
    /// `all_of(...)`, and `exactly_one_of(...)` (ADR-0006 §3). A keyword is
    /// only treated as a cardinality operator when it is immediately followed
    /// by `(` (after optional whitespace); otherwise it is left for
    /// `parse_predicate`, so a tag literally named `any_of` in `any_of == 'x'`
    /// still parses as a predicate. Returns `Ok(None)` when the cursor is not
    /// positioned on a cardinality call, leaving `self.idx` unchanged.
    fn try_parse_cardinality(&mut self) -> Result<Option<ConditionExpr>> {
        const KEYWORDS: [&[u8]; 3] = [b"any_of", b"all_of", b"exactly_one_of"];
        for kw in KEYWORDS {
            if !self.starts_with_keyword(kw) {
                continue;
            }
            // Peek past the keyword and any whitespace; only commit when an
            // opening paren follows. Otherwise this is a predicate whose tag
            // happens to share the keyword's spelling.
            let mut peek = self.idx + kw.len();
            skip_ws(self.bytes, &mut peek);
            if peek >= self.bytes.len() || self.bytes[peek] != b'(' {
                return Ok(None);
            }
            self.idx = peek + 1; // consume keyword + ws + '('
            let args = self.parse_cardinality_args(kw)?;
            let expr = match kw {
                b"any_of" => ConditionExpr::AnyOf(args),
                b"all_of" => ConditionExpr::AllOf(args),
                _ => ConditionExpr::ExactlyOneOf(args),
            };
            return Ok(Some(expr));
        }
        Ok(None)
    }

    /// Parse the comma-separated argument list of a cardinality call, having
    /// already consumed the opening `(`. Each argument is a full expression
    /// (`parse_or`), so arguments may be predicates, boolean literals, negated
    /// or parenthesised sub-expressions, and nested cardinality calls. Requires
    /// at least two arguments and rejects a trailing comma with a descriptive
    /// error (ADR-0006 §3).
    fn parse_cardinality_args(&mut self, kw: &[u8]) -> Result<Vec<ConditionExpr>> {
        let name = std::str::from_utf8(kw).unwrap_or("cardinality operator");
        let mut args = Vec::new();
        loop {
            args.push(self.parse_or()?);
            self.skip_ws();
            if self.idx >= self.bytes.len() {
                bail!(
                    "Unterminated `{}(...)`: missing closing parenthesis at position {}",
                    name,
                    self.idx
                );
            }
            match self.bytes[self.idx] {
                b',' => {
                    self.idx += 1;
                    self.skip_ws();
                    // A `)` here means the comma was trailing.
                    if self.idx < self.bytes.len() && self.bytes[self.idx] == b')' {
                        bail!(
                            "Trailing comma in `{}(...)` at position {}",
                            name,
                            self.idx
                        );
                    }
                }
                b')' => {
                    self.idx += 1;
                    break;
                }
                other => bail!(
                    "Expected ',' or ')' in `{}(...)` at position {} but found '{}'",
                    name,
                    self.idx,
                    other as char
                ),
            }
        }
        if args.len() < 2 {
            bail!(
                "`{}(...)` requires at least 2 arguments, found {}",
                name,
                args.len()
            );
        }
        Ok(args)
    }

    fn parse_predicate(&mut self) -> Result<ConditionExpr> {
        self.skip_ws();
        let start = self.idx;
        let tag = parse_ident(self.bytes, &mut self.idx)
            .with_context(|| format!("Expected identifier at position {}", start))?;
        self.skip_ws();
        let op = parse_operator(self.bytes, &mut self.idx)
            .with_context(|| format!("Expected '==' or '!=' at position {}", self.idx))?;
        self.skip_ws();
        let value = parse_quoted_literal(self.bytes, &mut self.idx)
            .with_context(|| format!("Expected quoted string literal at position {}", self.idx))?;
        Ok(ConditionExpr::Predicate(to_public_atom(ConditionAtom {
            tag,
            op,
            value,
        })))
    }

    fn try_consume_bool_literal(&mut self) -> Option<bool> {
        let value = if self.starts_with_keyword(b"true") {
            Some(true)
        } else if self.starts_with_keyword(b"false") {
            Some(false)
        } else {
            None
        };
        if let Some(v) = value {
            self.idx += if v { 4 } else { 5 };
            return Some(v);
        }
        None
    }

    fn starts_with_keyword(&self, kw: &[u8]) -> bool {
        if self.idx + kw.len() > self.bytes.len() {
            return false;
        }
        if &self.bytes[self.idx..self.idx + kw.len()] != kw {
            return false;
        }
        let next = self.idx + kw.len();
        next >= self.bytes.len() || !is_ident_char(self.bytes[next])
    }

    fn consume_op2(&mut self, a: u8, b: u8) -> bool {
        if self.idx + 1 < self.bytes.len()
            && self.bytes[self.idx] == a
            && self.bytes[self.idx + 1] == b
        {
            self.idx += 2;
            return true;
        }
        false
    }

    fn skip_ws(&mut self) {
        skip_ws(self.bytes, &mut self.idx);
    }
}

#[cfg(test)]
mod parser_tests;
