// SPDX-License-Identifier: BUSL-1.1

//! Hand-rolled recursive-descent evaluator for ConfigFlux tag-condition
//! expressions (ADR-0008). The evaluator replaces the previous `evalexpr`
//! dependency and shares tokeniser primitives with the atom parser in the
//! `conditions` module so implication and evaluation both accept the exact
//! same grammar.
//!
//! Grammar:
//!
//! ```text
//! condition   := or_expr
//! or_expr     := and_expr ('||' and_expr)*
//! and_expr    := unary ('&&' unary)*
//! unary       := '!' unary | primary
//! primary     := '(' condition ')' | bool_lit | atom
//! bool_lit    := 'true' | 'false'      (as whole-token keywords)
//! atom        := ident op rhs
//! rhs         := literal | ident       (an ident names another facet)
//! op          := '==' | '!='
//! literal     := '\'' … '\'' | '"' … '"'
//! ident       := [a-z][a-z0-9_]*
//! ```
//!
//! An unquoted right-hand side compares two facets' bound values rather than a
//! facet against a constant (configflux-secb.2 / ADR-0057 §D5). This evaluator
//! and the typed-AST parser in `ast` accept the same grammar, so a condition is
//! never valid in a constraint and invalid in a selector.
//!
//! The `true` / `false` keywords are first-class boolean primaries of the
//! grammar (also represented as `ConditionExpr::Bool` in the typed AST). They
//! let a condition denote an unconditional value without synthesising an atom —
//! e.g. the typed implication entry uses a `true` antecedent to stand in for
//! "no precondition". They are parsed as whole-token keywords: `true_region` is
//! a tag identifier, not `true` followed by `_region`.
//!
//! Short-circuit semantics: `&&` stops at the first false and `||` stops at
//! the first true. A right-hand side that references a missing tag is not
//! evaluated — and does not surface an error — when the left-hand side has
//! already decided the Boolean value. This matches the prior `evalexpr`-
//! backed behaviour the resolver tests pin.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;

use super::{
    parse_ident, parse_operator, parse_quoted_literal, skip_ws, AtomOp,
};

pub(super) fn eval_condition_impl(
    condition: &str,
    tags: &HashMap<String, String>,
) -> Result<bool> {
    let bytes = condition.as_bytes();
    let mut idx = 0;
    let value = parse_or(bytes, &mut idx, tags)?;
    skip_ws(bytes, &mut idx);
    if idx != bytes.len() {
        bail!("Unexpected trailing input at position {}", idx);
    }
    Ok(value)
}

fn parse_or(bytes: &[u8], idx: &mut usize, tags: &HashMap<String, String>) -> Result<bool> {
    let mut acc = parse_and(bytes, idx, tags)?;
    loop {
        skip_ws(bytes, idx);
        if !consume_op2(bytes, idx, b'|', b'|') {
            return Ok(acc);
        }
        if acc {
            skip_expr_and(bytes, idx)?;
        } else {
            let rhs = parse_and(bytes, idx, tags)?;
            acc = acc || rhs;
        }
    }
}

fn parse_and(bytes: &[u8], idx: &mut usize, tags: &HashMap<String, String>) -> Result<bool> {
    let mut acc = parse_unary(bytes, idx, tags)?;
    loop {
        skip_ws(bytes, idx);
        if !consume_op2(bytes, idx, b'&', b'&') {
            return Ok(acc);
        }
        if !acc {
            skip_unary(bytes, idx)?;
        } else {
            let rhs = parse_unary(bytes, idx, tags)?;
            acc = acc && rhs;
        }
    }
}

fn parse_unary(bytes: &[u8], idx: &mut usize, tags: &HashMap<String, String>) -> Result<bool> {
    skip_ws(bytes, idx);
    if *idx < bytes.len() && bytes[*idx] == b'!' {
        // Guard against consuming `!=` here — that is the atom operator.
        if *idx + 1 < bytes.len() && bytes[*idx + 1] == b'=' {
            return parse_primary(bytes, idx, tags);
        }
        *idx += 1;
        let inner = parse_unary(bytes, idx, tags)?;
        return Ok(!inner);
    }
    parse_primary(bytes, idx, tags)
}

fn parse_primary(bytes: &[u8], idx: &mut usize, tags: &HashMap<String, String>) -> Result<bool> {
    skip_ws(bytes, idx);
    if *idx >= bytes.len() {
        bail!("Unexpected end of condition");
    }
    if bytes[*idx] == b'(' {
        *idx += 1;
        let inner = parse_or(bytes, idx, tags)?;
        skip_ws(bytes, idx);
        if *idx >= bytes.len() || bytes[*idx] != b')' {
            bail!("Missing closing parenthesis at position {}", *idx);
        }
        *idx += 1;
        return Ok(inner);
    }
    if let Some(literal) = try_consume_bool_literal(bytes, idx) {
        return Ok(literal);
    }
    eval_atom(bytes, idx, tags)
}

/// Accept the keywords `true` and `false` as standalone Boolean primaries so
/// that callers can pass plain `"true"` / `"false"` as a condition, matching
/// the prior `evalexpr`-backed behaviour the resolver tests pin.
///
/// A keyword must be followed by either end-of-input, whitespace, or a
/// character that cannot continue an identifier; otherwise it is an atom's
/// tag name and is left for the atom parser.
fn try_consume_bool_literal(bytes: &[u8], idx: &mut usize) -> Option<bool> {
    let value = if starts_with_keyword(bytes, *idx, b"true") {
        Some(true)
    } else if starts_with_keyword(bytes, *idx, b"false") {
        Some(false)
    } else {
        None
    };
    if let Some(v) = value {
        let kw_len = if v { 4 } else { 5 };
        *idx += kw_len;
        return Some(v);
    }
    None
}

fn starts_with_keyword(bytes: &[u8], idx: usize, kw: &[u8]) -> bool {
    if idx + kw.len() > bytes.len() {
        return false;
    }
    if &bytes[idx..idx + kw.len()] != kw {
        return false;
    }
    // Must not be followed by an identifier-continuation byte, or it is a
    // tag name that starts with the keyword, not the keyword itself.
    let next = idx + kw.len();
    if next >= bytes.len() {
        return true;
    }
    let b = bytes[next];
    !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn eval_atom(bytes: &[u8], idx: &mut usize, tags: &HashMap<String, String>) -> Result<bool> {
    skip_ws(bytes, idx);
    let start = *idx;
    let tag = parse_ident(bytes, idx)
        .with_context(|| format!("Expected identifier at position {}", start))?;
    skip_ws(bytes, idx);
    let op = parse_operator(bytes, idx)
        .with_context(|| format!("Expected '==' or '!=' at position {}", *idx))?;
    skip_ws(bytes, idx);
    let rhs = parse_rhs(bytes, idx)?;

    let actual = tags
        .get(&tag)
        .with_context(|| format!("Missing tag '{}' referenced in condition", tag))?;
    // Operands are resolved left to right, so a condition that names two
    // unbound facets reports the left one first.
    let expected: &String = match &rhs {
        Rhs::Literal(value) => value,
        Rhs::Facet(name) => tags
            .get(name)
            .with_context(|| format!("Missing tag '{}' referenced in condition", name))?,
    };

    Ok(match op {
        AtomOp::Eq => actual == expected,
        AtomOp::NotEq => actual != expected,
    })
}

/// A predicate's right-hand side: a quoted literal, or — since
/// configflux-secb.2 / ADR-0057 §D5 — an unquoted identifier naming another
/// facet, whose bound value is the comparand.
enum Rhs {
    Literal(String),
    Facet(String),
}

/// Parse a right-hand side, accepting either form. Shared by the evaluating
/// and the short-circuit-skipping walks so both halves of this parser agree
/// on the grammar.
fn parse_rhs(bytes: &[u8], idx: &mut usize) -> Result<Rhs> {
    let start = *idx;
    if let Some(literal) = parse_quoted_literal(bytes, idx) {
        return Ok(Rhs::Literal(literal));
    }
    let name = parse_ident(bytes, idx).with_context(|| {
        format!(
            "Expected quoted string literal or identifier at position {}",
            start
        )
    })?;
    Ok(Rhs::Facet(name))
}

/// Advance the cursor past a short-circuited `and_expr` chain without
/// evaluating atoms against the tag environment. We still must parse the
/// structure to report syntax errors and to find the next `||` / end-of-
/// input.
fn skip_expr_and(bytes: &[u8], idx: &mut usize) -> Result<()> {
    skip_unary(bytes, idx)?;
    loop {
        skip_ws(bytes, idx);
        if !consume_op2(bytes, idx, b'&', b'&') {
            return Ok(());
        }
        skip_unary(bytes, idx)?;
    }
}

/// Advance the cursor past a single `unary` production without evaluating
/// atoms against the tag environment. Parentheses are honoured so that a
/// nested `||` inside a short-circuited conjunct does not leak into the
/// outer parser.
fn skip_unary(bytes: &[u8], idx: &mut usize) -> Result<()> {
    skip_ws(bytes, idx);
    if *idx >= bytes.len() {
        bail!("Unexpected end of condition");
    }
    if bytes[*idx] == b'!' {
        if *idx + 1 < bytes.len() && bytes[*idx + 1] == b'=' {
            return skip_atom(bytes, idx);
        }
        *idx += 1;
        return skip_unary(bytes, idx);
    }
    if bytes[*idx] == b'(' {
        *idx += 1;
        skip_condition(bytes, idx)?;
        skip_ws(bytes, idx);
        if *idx >= bytes.len() || bytes[*idx] != b')' {
            bail!("Missing closing parenthesis at position {}", *idx);
        }
        *idx += 1;
        return Ok(());
    }
    if try_consume_bool_literal(bytes, idx).is_some() {
        return Ok(());
    }
    skip_atom(bytes, idx)
}

fn skip_condition(bytes: &[u8], idx: &mut usize) -> Result<()> {
    skip_expr_and(bytes, idx)?;
    loop {
        skip_ws(bytes, idx);
        if !consume_op2(bytes, idx, b'|', b'|') {
            return Ok(());
        }
        skip_expr_and(bytes, idx)?;
    }
}

fn skip_atom(bytes: &[u8], idx: &mut usize) -> Result<()> {
    skip_ws(bytes, idx);
    let start = *idx;
    let _ = parse_ident(bytes, idx)
        .with_context(|| format!("Expected identifier at position {}", start))?;
    skip_ws(bytes, idx);
    let _ = parse_operator(bytes, idx)
        .with_context(|| format!("Expected '==' or '!=' at position {}", *idx))?;
    skip_ws(bytes, idx);
    // Both right-hand-side forms must be skippable, or a short-circuited
    // branch containing a facet comparison would fail to parse
    // (configflux-secb.2).
    let _ = parse_rhs(bytes, idx)?;
    Ok(())
}

fn consume_op2(bytes: &[u8], idx: &mut usize, a: u8, b: u8) -> bool {
    if *idx + 1 < bytes.len() && bytes[*idx] == a && bytes[*idx + 1] == b {
        *idx += 2;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests;
