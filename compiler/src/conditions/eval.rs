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
//! atom        := ident op literal
//! op          := '==' | '!='
//! literal     := '\'' … '\'' | '"' … '"'
//! ident       := [a-z][a-z0-9_]*
//! ```
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
    let literal = parse_quoted_literal(bytes, idx)
        .with_context(|| format!("Expected quoted string literal at position {}", *idx))?;

    let actual = tags
        .get(&tag)
        .with_context(|| format!("Missing tag '{}' referenced in condition", tag))?;

    Ok(match op {
        AtomOp::Eq => actual == &literal,
        AtomOp::NotEq => actual != &literal,
    })
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
    let _ = parse_quoted_literal(bytes, idx)
        .with_context(|| format!("Expected quoted string literal at position {}", *idx))?;
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

// ----------------------------------------------------------------------------
// Grammar coverage tests for the hand-rolled evaluator (ADR-0008).
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::eval_condition;
    use std::collections::HashMap;

    fn tags_with(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        let mut tags = HashMap::new();
        for (k, v) in pairs {
            tags.insert((*k).to_string(), (*v).to_string());
        }
        tags
    }

    #[test]
    fn eval_double_quoted_literals() {
        let tags = tags_with(&[("variant", "heavy")]);
        assert!(eval_condition("variant == \"heavy\"", &tags).unwrap());
        assert!(!eval_condition("variant != \"heavy\"", &tags).unwrap());
    }

    #[test]
    fn eval_mixed_quotes_in_same_expression() {
        let tags = tags_with(&[("variant", "heavy"), ("region", "us")]);
        assert!(eval_condition("variant == 'heavy' && region == \"us\"", &tags).unwrap());
    }

    #[test]
    fn eval_and_precedence_binds_tighter_than_or() {
        // `a && b || c && d` should parse as `(a && b) || (c && d)`.
        let tags = tags_with(&[("a", "1"), ("b", "0"), ("c", "1"), ("d", "1")]);
        let expr = "a == '1' && b == '1' || c == '1' && d == '1'";
        assert!(eval_condition(expr, &tags).unwrap());
    }

    #[test]
    fn eval_parentheses_override_precedence() {
        let tags = tags_with(&[("a", "1"), ("b", "0"), ("c", "1")]);
        assert!(eval_condition("a == '1' && (b == '1' || c == '1')", &tags).unwrap());
        assert!(eval_condition("(a == '1' && b == '1') || c == '1'", &tags).unwrap());
        let tags2 = tags_with(&[("a", "1"), ("b", "0"), ("c", "0")]);
        assert!(!eval_condition("a == '1' && (b == '1' || c == '1')", &tags2).unwrap());
    }

    #[test]
    fn eval_unary_negation() {
        let tags = tags_with(&[("variant", "heavy")]);
        assert!(!eval_condition("!(variant == 'heavy')", &tags).unwrap());
        assert!(eval_condition("!(variant == 'light')", &tags).unwrap());
        assert!(eval_condition("!!(variant == 'heavy')", &tags).unwrap());
    }

    #[test]
    fn eval_short_circuit_and_skips_missing_rhs() {
        let tags = tags_with(&[("variant", "light")]);
        let ok = eval_condition("variant == 'heavy' && region == 'us'", &tags).unwrap();
        assert!(!ok);
    }

    #[test]
    fn eval_short_circuit_or_skips_missing_rhs() {
        let tags = tags_with(&[("variant", "heavy")]);
        let ok = eval_condition("variant == 'heavy' || region == 'us'", &tags).unwrap();
        assert!(ok);
    }

    #[test]
    fn eval_rejects_trailing_garbage() {
        let tags = tags_with(&[("variant", "heavy")]);
        let err = eval_condition("variant == 'heavy' xyz", &tags).unwrap_err();
        assert!(
            format!("{err}").contains("Failed to evaluate condition"),
            "err: {err}"
        );
    }

    #[test]
    fn eval_rejects_unbalanced_parentheses() {
        let tags = tags_with(&[("variant", "heavy")]);
        let err = eval_condition("(variant == 'heavy'", &tags).unwrap_err();
        assert!(
            format!("{err}").contains("Missing closing parenthesis")
                || format!("{err}").contains("Failed to evaluate condition"),
            "err: {err}"
        );
    }

    #[test]
    fn eval_neq_operator_not_confused_with_bang() {
        let tags = tags_with(&[("a", "y")]);
        assert!(eval_condition("a != 'x'", &tags).unwrap());
        assert!(!eval_condition("a != 'y'", &tags).unwrap());
    }

    #[test]
    fn eval_empty_condition_errors() {
        let tags: HashMap<String, String> = HashMap::new();
        let err = eval_condition("", &tags).unwrap_err();
        assert!(
            format!("{err}").contains("Failed to evaluate condition"),
            "err: {err}"
        );
    }

    #[test]
    fn eval_bool_literal_true_and_false() {
        let tags: HashMap<String, String> = HashMap::new();
        assert!(eval_condition("true", &tags).unwrap());
        assert!(!eval_condition("false", &tags).unwrap());
        // With surrounding whitespace.
        assert!(eval_condition("  true  ", &tags).unwrap());
    }

    #[test]
    fn eval_bool_literal_combines_with_atoms() {
        let tags = tags_with(&[("variant", "heavy")]);
        assert!(eval_condition("true && variant == 'heavy'", &tags).unwrap());
        assert!(!eval_condition("false && variant == 'heavy'", &tags).unwrap());
        assert!(eval_condition("false || variant == 'heavy'", &tags).unwrap());
        // Short-circuit: `true ||` should skip missing-tag RHS.
        assert!(eval_condition("true || region == 'us'", &tags).unwrap());
    }

    #[test]
    fn eval_true_prefixed_identifier_is_tag_not_keyword() {
        // `true_region` is a valid identifier; must parse as an atom.
        let tags = tags_with(&[("true_region", "us")]);
        assert!(eval_condition("true_region == 'us'", &tags).unwrap());
    }
}
