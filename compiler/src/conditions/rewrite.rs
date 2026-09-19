// SPDX-License-Identifier: BUSL-1.1

//! Identifier-rewrite and identifier-enumeration over the typed condition AST.
//!
//! These two entry points exist for the model scrubber (`tools/model_scrubber`,
//! configflux-p2sz.4 / ADR-0034). The scrubber must pseudonymize every
//! identifier embedded in a `condition` string while preserving the constraint
//! structure *exactly*. A regex/string rename cannot reliably tell an
//! identifier from surrounding syntax, so ADR-0034 D3 mandates: parse the
//! condition through the existing grammar into [`ConditionExpr`], substitute
//! identifiers on the typed AST, and re-serialize from the AST.
//!
//! Two capabilities are exported, both pure and free of any
//! compiler→solver coupling (ADR-0003 §2): they reuse only the in-crate
//! condition grammar.
//!
//! - [`condition_identifiers`] enumerates the facet tags and the quoted option
//!   literals a condition references, so the scrubber can *derive* (not guess)
//!   the option-identifier set that drives its `Value::String`
//!   option-vs-free-text discrimination rule.
//! - [`rewrite_condition_identifiers`] parses, substitutes each tag and each
//!   `(tag, literal)` via caller-supplied closures, and re-serializes a
//!   canonical condition string that re-parses to an isomorphic AST.

use super::{parse_condition_expr, ConditionExpr, ConditionPredicate, ConditionPredicateOp};
use anyhow::Result;
use std::collections::BTreeSet;

/// The identifiers a condition references, partitioned by role.
///
/// `tags` are facet identifiers (the left-hand side of every predicate, e.g.
/// `variant` in `variant == 'heavy'`). `options` are `(tag, literal)` pairs:
/// the quoted option identifier together with the facet it qualifies (e.g.
/// `("variant", "heavy")`). Both are sorted and de-duplicated so the caller
/// gets a stable, order-independent view regardless of where in the expression
/// a reference appears.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConditionIdentifiers {
    /// Facet identifiers, sorted and de-duplicated.
    pub tags: BTreeSet<String>,
    /// `(facet_tag, option_literal)` pairs, sorted and de-duplicated.
    pub options: BTreeSet<(String, String)>,
}

/// Enumerate every facet tag and quoted option literal a condition references.
///
/// The condition is parsed through the validated grammar; a malformed condition
/// surfaces the parser's descriptive error rather than a partial result. The
/// walk visits every predicate in every Boolean/cardinality sub-expression, so
/// the returned sets are complete.
pub fn condition_identifiers(condition: &str) -> Result<ConditionIdentifiers> {
    let expr = parse_condition_expr(condition)?;
    let mut acc = ConditionIdentifiers::default();
    collect(&expr, &mut acc);
    Ok(acc)
}

fn collect(expr: &ConditionExpr, acc: &mut ConditionIdentifiers) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(p) => {
            acc.tags.insert(p.tag.clone());
            acc.options.insert((p.tag.clone(), p.value.clone()));
        }
        ConditionExpr::Not(inner) => collect(inner, acc),
        ConditionExpr::And(lhs, rhs) | ConditionExpr::Or(lhs, rhs) => {
            collect(lhs, acc);
            collect(rhs, acc);
        }
        ConditionExpr::AnyOf(children)
        | ConditionExpr::AllOf(children)
        | ConditionExpr::ExactlyOneOf(children) => {
            for child in children {
                collect(child, acc);
            }
        }
        // configflux-secb.2 / ADR-0057 §D5: BOTH operands are facet
        // identifiers, so both belong in `tags`. Missing the right-hand one
        // would leave a real facet name un-pseudonymized in scrubbed output
        // (ADR-0034 D3). Neither is an option literal, so `options` is
        // untouched.
        ConditionExpr::FacetCompare { left, right, .. } => {
            acc.tags.insert(left.clone());
            acc.tags.insert(right.clone());
        }
    }
}

/// Parse `condition`, substitute every facet tag and every quoted option
/// literal via the supplied closures, and re-serialize a canonical condition
/// string from the rewritten AST.
///
/// `rename_tag(tag)` returns the replacement facet identifier. `rename_literal(
/// tag, literal)` returns the replacement option identifier, given both the
/// literal and the *original* facet it qualifies (so the same literal under two
/// different facets can map independently if the caller wants). Both closures
/// must return strings that are valid grammar identifiers (ASCII lowercase
/// start; lowercase/digit/underscore tail) — the scrubber's category-prefixed
/// pseudonyms (`opt_001`, `tag_003`, …) satisfy this by construction.
///
/// The re-serialized string is canonical: a single space around binary
/// operators, no redundant parentheses beyond those needed to preserve
/// precedence and associativity, single-quoted literals. It is guaranteed to
/// re-parse to an AST isomorphic to the substituted one, which is what lets the
/// scrubber's output pass `link_and_verify` (whose dependency-implication check
/// re-parses these conditions).
pub fn rewrite_condition_identifiers(
    condition: &str,
    rename_tag: &dyn Fn(&str) -> String,
    rename_literal: &dyn Fn(&str, &str) -> String,
) -> Result<String> {
    let expr = parse_condition_expr(condition)?;
    let rewritten = substitute(&expr, rename_tag, rename_literal);
    Ok(serialize(&rewritten, Prec::Or))
}

fn substitute(
    expr: &ConditionExpr,
    rename_tag: &dyn Fn(&str) -> String,
    rename_literal: &dyn Fn(&str, &str) -> String,
) -> ConditionExpr {
    match expr {
        ConditionExpr::Bool(b) => ConditionExpr::Bool(*b),
        ConditionExpr::Predicate(p) => ConditionExpr::Predicate(ConditionPredicate {
            tag: rename_tag(&p.tag),
            op: p.op.clone(),
            value: rename_literal(&p.tag, &p.value),
        }),
        ConditionExpr::Not(inner) => {
            ConditionExpr::Not(Box::new(substitute(inner, rename_tag, rename_literal)))
        }
        ConditionExpr::And(lhs, rhs) => ConditionExpr::And(
            Box::new(substitute(lhs, rename_tag, rename_literal)),
            Box::new(substitute(rhs, rename_tag, rename_literal)),
        ),
        ConditionExpr::Or(lhs, rhs) => ConditionExpr::Or(
            Box::new(substitute(lhs, rename_tag, rename_literal)),
            Box::new(substitute(rhs, rename_tag, rename_literal)),
        ),
        ConditionExpr::AnyOf(children) => {
            ConditionExpr::AnyOf(substitute_children(children, rename_tag, rename_literal))
        }
        ConditionExpr::AllOf(children) => {
            ConditionExpr::AllOf(substitute_children(children, rename_tag, rename_literal))
        }
        ConditionExpr::ExactlyOneOf(children) => {
            ConditionExpr::ExactlyOneOf(substitute_children(children, rename_tag, rename_literal))
        }
        // `rename_tag` applies to both operands; `rename_literal` has no
        // meaning here because neither side is a literal (configflux-secb.2).
        ConditionExpr::FacetCompare { left, op, right } => ConditionExpr::FacetCompare {
            left: rename_tag(left),
            op: op.clone(),
            right: rename_tag(right),
        },
    }
}

fn substitute_children(
    children: &[ConditionExpr],
    rename_tag: &dyn Fn(&str) -> String,
    rename_literal: &dyn Fn(&str, &str) -> String,
) -> Vec<ConditionExpr> {
    children
        .iter()
        .map(|c| substitute(c, rename_tag, rename_literal))
        .collect()
}

/// Binding-strength context for re-serialization. The grammar precedence is
/// `||` (loosest) < `&&` < unary `!` < primary. A child is parenthesized only
/// when its own strength is looser than the position it sits in, which keeps
/// the output minimal while preserving the parse.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prec {
    Or,
    And,
    Unary,
}

fn serialize(expr: &ConditionExpr, parent: Prec) -> String {
    match expr {
        ConditionExpr::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        ConditionExpr::Predicate(p) => {
            let op = match p.op {
                ConditionPredicateOp::Eq => "==",
                ConditionPredicateOp::NotEq => "!=",
            };
            format!("{} {} '{}'", p.tag, op, p.value)
        }
        ConditionExpr::Not(inner) => {
            // `!` binds tighter than `&&`/`||`; its operand is serialized at
            // unary strength so a bare predicate needs no parens but an
            // `&&`/`||` operand is wrapped.
            format!("!{}", serialize(inner, Prec::Unary))
        }
        ConditionExpr::And(lhs, rhs) => {
            let inner = format!(
                "{} && {}",
                serialize(lhs, Prec::And),
                serialize(rhs, Prec::And)
            );
            wrap_if(inner, Prec::And, parent)
        }
        ConditionExpr::Or(lhs, rhs) => {
            let inner = format!(
                "{} || {}",
                serialize(lhs, Prec::Or),
                serialize(rhs, Prec::Or)
            );
            wrap_if(inner, Prec::Or, parent)
        }
        ConditionExpr::AnyOf(children) => serialize_call("any_of", children),
        ConditionExpr::AllOf(children) => serialize_call("all_of", children),
        ConditionExpr::ExactlyOneOf(children) => serialize_call("exactly_one_of", children),
        // configflux-secb.2 / ADR-0057 §D5: the right-hand side is rendered
        // BARE. Quoting it would turn a facet comparison into a literal
        // comparison, so the round-trip guarantee this serializer owes the
        // scrubber would silently change the condition's meaning.
        ConditionExpr::FacetCompare { left, op, right } => {
            let op = match op {
                ConditionPredicateOp::Eq => "==",
                ConditionPredicateOp::NotEq => "!=",
            };
            format!("{} {} {}", left, op, right)
        }
    }
}

fn serialize_call(name: &str, children: &[ConditionExpr]) -> String {
    // Cardinality arguments are full expressions; each is serialized at the
    // loosest strength (`Or`) because the surrounding commas and parens already
    // delimit them, so no argument ever needs extra wrapping.
    let args: Vec<String> = children.iter().map(|c| serialize(c, Prec::Or)).collect();
    format!("{}({})", name, args.join(", "))
}

fn wrap_if(s: String, own: Prec, parent: Prec) -> String {
    if own < parent {
        format!("({})", s)
    } else {
        s
    }
}

#[cfg(test)]
#[path = "rewrite_tests.rs"]
mod rewrite_tests;
