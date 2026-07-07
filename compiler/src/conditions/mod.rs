// SPDX-License-Identifier: BUSL-1.1

//! Tag-condition evaluation and implication for the compiler's resolver
//! and override-gating logic.
//!
//! The grammar is a Boolean conjunction/disjunction of tag-equality atoms
//! (e.g. `variant == 'heavy' && region != 'eu'`) evaluated against a
//! `HashMap<String, String>` of tag values. The string evaluator lives in the
//! `eval` submodule; the atom parser in this module supplies the shared
//! tokeniser consumed by `eval` and the typed-AST parser in `ast`.
//!
//! Component-dependency activation subsumption (`link_verify`) is decided by
//! [`condition_implies`], which parses both conditions into the typed
//! [`ConditionExpr`] AST and delegates to the exhaustive truth-table evaluator
//! in `selection_eval` (configflux-uiyo). It replaced the earlier
//! string-scanning matrix engine.
//!
//! See ADR-0008 for the decision to retire the previous `evalexpr`
//! dependency in favour of the hand-rolled evaluator that lives here.

use anyhow::{Context, Result};
use std::collections::HashMap;

mod ast;
mod eval;
mod implication;
mod rewrite;
mod selection_eval;

pub(crate) use ast::{parse_condition_expr, ConditionExpr};
// configflux-p2sz.4 / ADR-0034 D3: public AST-based condition identifier
// rewrite + enumeration for the model scrubber (`tools/model_scrubber`). These
// are the ONLY public entry points into the condition AST; the scrubber must
// parse-substitute-re-serialize (a regex rename is prohibited), and these reuse
// only the in-crate grammar — no compiler→solver coupling (ADR-0003 §2).
pub use rewrite::{condition_identifiers, rewrite_condition_identifiers, ConditionIdentifiers};
// configflux-uiyo: typed exhaustive-implication evaluator over `ConditionExpr`,
// backing `condition_implies_typed` below (the link_verify dependency-
// subsumption check). Replaced the string-scanning matrix engine.
pub(crate) use implication::condition_expr_implies;
// configflux-ccs.7: typed-AST selection-constraint evaluator. Replaces the
// string-scanning option-validity path in loader_api/shared_ops.rs.
pub(crate) use selection_eval::{
    for_each_eq_predicate, is_pure_conjunction, mentions_eq, not_contradicted,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConditionPredicate {
    pub tag: String,
    pub op: ConditionPredicateOp,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConditionPredicateOp {
    Eq,
    NotEq,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConditionAtom {
    tag: String,
    op: AtomOp,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum AtomOp {
    Eq,
    NotEq,
}

pub fn eval_condition(condition: &str, tags: &HashMap<String, String>) -> Result<bool> {
    eval::eval_condition_impl(condition, tags)
        .with_context(|| format!("Failed to evaluate condition: '{}'", condition))
}

/// Decide whether component-activation condition `a` implies condition `b`
/// (`a => b`): a component may only `depends_on` another when the dependent's
/// activation condition holds wherever the depender's does. `None` means "no
/// condition" — an always-active component. A missing consequent (`b == None`)
/// is implied by anything; a missing antecedent (`a == None`) is the
/// always-true precondition.
///
/// configflux-uiyo: this typed entry replaced the string-scanning matrix
/// engine (the former `condition_implies` / `eval_implication_by_matrix`
/// cluster). It parses both sides into the typed [`ConditionExpr`] AST and
/// delegates to the exhaustive truth-table evaluator
/// [`condition_expr_implies`]. Parsing reuses the validated grammar in `ast`,
/// so an unparseable condition surfaces a descriptive error rather than a
/// silent verdict. The truth-table search is logically sound, so it accepts
/// every dependency edge the old subset shortcut accepted (byte-stable) plus
/// the sound cross-operator implications that shortcut missed.
pub(crate) fn condition_implies_typed(a: Option<&str>, b: Option<&str>) -> Result<bool> {
    let Some(b_cond) = b else {
        return Ok(true);
    };
    let consequent = parse_condition_expr(b_cond)
        .with_context(|| format!("Failed to parse dependency condition: '{}'", b_cond))?;
    let antecedent = match a {
        None => ConditionExpr::Bool(true),
        Some(a_cond) => parse_condition_expr(a_cond)
            .with_context(|| format!("Failed to parse component condition: '{}'", a_cond))?,
    };
    Ok(condition_expr_implies(&antecedent, &consequent))
}

// Shared tokeniser primitives. `pub(crate)` so the `eval` submodule can
// consume them via `super::` while still keeping them off the crate's
// public API surface.

pub(crate) fn parse_ident(bytes: &[u8], idx: &mut usize) -> Option<String> {
    if *idx >= bytes.len() || !is_ident_start(bytes[*idx]) {
        return None;
    }
    let start = *idx;
    *idx += 1;
    while *idx < bytes.len() && is_ident_char(bytes[*idx]) {
        *idx += 1;
    }
    std::str::from_utf8(&bytes[start..*idx])
        .ok()
        .map(|s| s.to_string())
}

pub(crate) fn parse_operator(bytes: &[u8], idx: &mut usize) -> Option<AtomOp> {
    if *idx + 1 >= bytes.len() {
        return None;
    }
    let op = match (bytes[*idx], bytes[*idx + 1]) {
        (b'=', b'=') => AtomOp::Eq,
        (b'!', b'=') => AtomOp::NotEq,
        _ => return None,
    };
    *idx += 2;
    Some(op)
}

pub(crate) fn parse_quoted_literal(bytes: &[u8], idx: &mut usize) -> Option<String> {
    if *idx >= bytes.len() || (bytes[*idx] != b'\'' && bytes[*idx] != b'"') {
        return None;
    }
    let quote = bytes[*idx];
    *idx += 1;
    let start = *idx;
    while *idx < bytes.len() && bytes[*idx] != quote {
        *idx += 1;
    }
    if *idx >= bytes.len() {
        return None;
    }
    let value = std::str::from_utf8(&bytes[start..*idx]).ok()?.to_string();
    *idx += 1;
    Some(value)
}

pub(crate) fn skip_ws(bytes: &[u8], idx: &mut usize) {
    while *idx < bytes.len() && bytes[*idx].is_ascii_whitespace() {
        *idx += 1;
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_lowercase()
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
}

fn to_public_atom(atom: ConditionAtom) -> ConditionPredicate {
    ConditionPredicate {
        tag: atom.tag,
        op: match atom.op {
            AtomOp::Eq => ConditionPredicateOp::Eq,
            AtomOp::NotEq => ConditionPredicateOp::NotEq,
        },
        value: atom.value,
    }
}

// ----------------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_condition_accepts_single_quotes() {
        let mut tags = HashMap::new();
        tags.insert("variant".to_string(), "heavy".to_string());
        tags.insert("region".to_string(), "us".to_string());
        let result = eval_condition("variant == 'heavy' && region != 'eu'", &tags).unwrap();
        assert!(result);
    }

    #[test]
    fn eval_condition_errors_on_missing_tag() {
        let tags = HashMap::new();
        let err = eval_condition("variant == 'heavy'", &tags).unwrap_err();
        assert!(
            format!("{err}").contains("Failed to evaluate condition"),
            "err: {err}"
        );
    }

    #[test]
    fn condition_implies_subset_is_true() {
        let a = "variant == 'heavy' && region == 'us'";
        let b = "variant == 'heavy'";
        assert!(condition_implies_typed(Some(a), Some(b)).unwrap());
    }

    #[test]
    fn condition_implies_mismatch_is_false() {
        let a = "variant == 'heavy'";
        let b = "region == 'us'";
        assert!(!condition_implies_typed(Some(a), Some(b)).unwrap());
    }

    #[test]
    fn condition_implies_none_cases() {
        assert!(!condition_implies_typed(None, Some("variant == 'heavy'")).unwrap());
        assert!(condition_implies_typed(Some("variant == 'heavy'"), None).unwrap());
    }

    #[test]
    fn condition_implies_or_clause_is_false() {
        let a = "variant == 'heavy' || region == 'us'";
        let b = "variant == 'heavy'";
        assert!(!condition_implies_typed(Some(a), Some(b)).unwrap());
    }

    #[test]
    fn condition_implies_rejects_unparseable_condition() {
        // The typed entry parses both sides through the validated grammar, so a
        // malformed condition surfaces a descriptive parse error instead of a
        // silent verdict.
        let err = condition_implies_typed(Some("variant == heavy"), Some("variant == 'heavy'"))
            .unwrap_err();
        assert!(
            format!("{err}").contains("Failed to parse component condition"),
            "err: {err}"
        );
    }
}
