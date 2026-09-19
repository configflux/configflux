// SPDX-License-Identifier: BUSL-1.1

//! Expansion of the facet-to-facet comparison form into the core grammar —
//! configflux-secb.2, ADR-0057 §D5 (an amendment to ADR-0006's grammar).
//!
//! `a == b` says the two facets are bound to the same value. Its meaning is a
//! pairwise equivalence over the union of the two DECLARED domains:
//!
//! ```text
//! a == b  ≡  AND over v ∈ dom(a) ∪ dom(b) of (a.v ⇔ b.v),  with x.v ≡ false when v ∉ dom(x)
//! a != b  ≡  NOT (a == b)
//! ```
//!
//! Neither BDD backend can evaluate that: `compile_expr` receives only a
//! symbol-name-to-variable-index map and has no way to ask what values a facet
//! declares. So the node is expanded HERE, once, against the declared domains
//! the emitter carries on [`ConditionModel::facet_domains`], before either
//! backend sees the expression. That placement buys two things beyond
//! feasibility. The two backends fold an identical tree through their existing
//! `And`/`Or`/`Not`/`Predicate` arms, so they produce the same canonical BDD by
//! construction rather than by two hand-written folds agreeing. And the
//! authored constraint text is never rewritten, so the ADR-0054 §5.4 manifest
//! roster and `cfx explain` still quote what the author wrote.
//!
//! **Fold order is pinned** (ADR-0006 §5, restated by ADR-0057 §D5): values in
//! `a`'s declared order, then `b`'s remaining values in `b`'s declared order,
//! AND-folded strictly left-associatively. Emitted bytes depend on it.
//!
//! **No new symbols.** Every predicate this emits names a `(facet, value)`
//! pair drawn from a declared domain, and the declared-facet symbol pass
//! (`compiler_core::synthesize_facet_clauses`, ADR-0047 §4 Amendment 1)
//! already contributes all of those. A model that gains a facet comparison
//! therefore keeps its symbol universe; only the root changes.

use std::collections::BTreeMap;

use anyhow::{bail, Result};

use super::{ConditionExpr, ConditionPredicate, ConditionPredicateOp};

/// Declared value domains by facet name, in declared order. The emitter's
/// copy of `compiler_core::declared_facets()`, narrowed to what the expansion
/// needs.
pub(crate) type FacetDomains = BTreeMap<String, Vec<String>>;

/// Rewrite every facet-to-facet comparison in `expr` into the
/// `And`/`Or`/`Not`/`Predicate` fragment, leaving every other node untouched.
///
/// Fails when an operand has no declared domain. That is an internal
/// invariant rather than an authoring error: `link_verify::validate_constraints`
/// rejects an undeclared operand with a diagnostic that names the constraint,
/// long before emission, so reaching this branch means a producer built a
/// `ConditionModel` whose `facet_domains` disagrees with its clauses.
pub(crate) fn expand_facet_comparisons(
    expr: &ConditionExpr,
    domains: &FacetDomains,
) -> Result<ConditionExpr> {
    Ok(match expr {
        ConditionExpr::Bool(_) | ConditionExpr::Predicate(_) => expr.clone(),
        ConditionExpr::Not(inner) => {
            ConditionExpr::Not(Box::new(expand_facet_comparisons(inner, domains)?))
        }
        ConditionExpr::And(left, right) => ConditionExpr::And(
            Box::new(expand_facet_comparisons(left, domains)?),
            Box::new(expand_facet_comparisons(right, domains)?),
        ),
        ConditionExpr::Or(left, right) => ConditionExpr::Or(
            Box::new(expand_facet_comparisons(left, domains)?),
            Box::new(expand_facet_comparisons(right, domains)?),
        ),
        ConditionExpr::AnyOf(children) => {
            ConditionExpr::AnyOf(expand_children(children, domains)?)
        }
        ConditionExpr::AllOf(children) => {
            ConditionExpr::AllOf(expand_children(children, domains)?)
        }
        ConditionExpr::ExactlyOneOf(children) => {
            ConditionExpr::ExactlyOneOf(expand_children(children, domains)?)
        }
        ConditionExpr::FacetCompare { left, op, right } => {
            let equality = expand_equality(left, right, domains)?;
            match op {
                ConditionPredicateOp::Eq => equality,
                ConditionPredicateOp::NotEq => ConditionExpr::Not(Box::new(equality)),
            }
        }
    })
}

fn expand_children(
    children: &[ConditionExpr],
    domains: &FacetDomains,
) -> Result<Vec<ConditionExpr>> {
    children
        .iter()
        .map(|child| expand_facet_comparisons(child, domains))
        .collect()
}

/// The equivalence `AND over v ∈ dom(left) ∪ dom(right) of (left.v ⇔ right.v)`.
///
/// A value only one side declares contributes `x.v ⇔ false`, i.e. the single
/// negative literal `x != 'v'` — which is what makes a value the other side
/// cannot take unreachable under equality, without depending on the
/// closed-facet cardinality conjuncts being present.
///
/// An empty union yields the AND identity. Declared domains are non-empty
/// (`link_verify::validate_facets` rejects an empty one), so the case is
/// defensive.
fn expand_equality(
    left: &str,
    right: &str,
    domains: &FacetDomains,
) -> Result<ConditionExpr> {
    let left_values = declared_domain(left, domains)?;
    let right_values = declared_domain(right, domains)?;

    let mut folded: Option<ConditionExpr> = None;
    for value in union_in_fold_order(left_values, right_values) {
        let in_left = left_values.iter().any(|v| v == value);
        let in_right = right_values.iter().any(|v| v == value);
        let term = match (in_left, in_right) {
            (true, true) => ConditionExpr::Or(
                Box::new(ConditionExpr::And(
                    Box::new(predicate(left, ConditionPredicateOp::Eq, value)),
                    Box::new(predicate(right, ConditionPredicateOp::Eq, value)),
                )),
                Box::new(ConditionExpr::And(
                    Box::new(predicate(left, ConditionPredicateOp::NotEq, value)),
                    Box::new(predicate(right, ConditionPredicateOp::NotEq, value)),
                )),
            ),
            (true, false) => predicate(left, ConditionPredicateOp::NotEq, value),
            (false, true) => predicate(right, ConditionPredicateOp::NotEq, value),
            // Unreachable: the union is drawn from the two domains.
            (false, false) => continue,
        };
        folded = Some(match folded {
            None => term,
            Some(previous) => ConditionExpr::And(Box::new(previous), Box::new(term)),
        });
    }

    Ok(folded.unwrap_or(ConditionExpr::Bool(true)))
}

/// `left`'s declared values, then `right`'s values that `left` does not
/// declare — each in its own declared order. This IS the fold order
/// (ADR-0057 §D5), so it must be a pure function of the declarations.
fn union_in_fold_order<'a>(left: &'a [String], right: &'a [String]) -> Vec<&'a String> {
    let mut union: Vec<&String> = Vec::with_capacity(left.len() + right.len());
    for value in left.iter().chain(right.iter()) {
        if !union.iter().any(|seen| *seen == value) {
            union.push(value);
        }
    }
    union
}

fn declared_domain<'a>(name: &str, domains: &'a FacetDomains) -> Result<&'a Vec<String>> {
    match domains.get(name) {
        Some(values) => Ok(values),
        None => bail!(
            "internal: facet comparison names '{}', which has no declared value domain; \
             an undeclared operand must be rejected by `link_verify::validate_constraints` \
             before emission",
            name
        ),
    }
}

fn predicate(tag: &str, op: ConditionPredicateOp, value: &str) -> ConditionExpr {
    ConditionExpr::Predicate(ConditionPredicate {
        tag: tag.to_string(),
        op,
        value: value.to_string(),
    })
}

/// One name a condition puts in operand position, reported in left-to-right
/// DFS pre-order by [`for_each_condition_operand`].
///
/// The variants exist so a caller can tell a facet named as a PREDICATE tag
/// from a facet named as the right-hand side of a comparison — the two are
/// wrong in different ways and deserve different diagnostics. Reporting the
/// right-hand side as a `Symbol` pair would be worse than useless: the
/// closed-domain check would then reject the other facet's NAME as an
/// undeclared VALUE.
pub(crate) enum ConditionOperand<'a> {
    /// A predicate's `(facet, value)` pair, under either operator. Both name
    /// the same `.ccm` variable (`compile_predicate` lowers `!=` to `not(var)`).
    Symbol(&'a str, &'a str),
    /// The left-hand facet of a comparison (`a` in `a == b`).
    ComparisonLeft(&'a str),
    /// The right-hand facet of a comparison (`b` in `a == b`).
    ComparisonRight(&'a str),
}

/// Visit every operand in `expr` in left-to-right DFS pre-order, reporting
/// predicates and comparison sides through one sink.
///
/// One walk rather than two, because callers that report the FIRST violation
/// need the two node kinds interleaved in source order — with separate walks
/// the reported violation would depend on which walk ran first rather than on
/// what the author wrote (configflux-secb.2).
pub(crate) fn for_each_condition_operand<F: FnMut(ConditionOperand<'_>)>(
    expr: &ConditionExpr,
    mut sink: F,
) {
    visit_operands(expr, &mut sink);
}

fn visit_operands<F: FnMut(ConditionOperand<'_>)>(expr: &ConditionExpr, sink: &mut F) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(predicate) => {
            sink(ConditionOperand::Symbol(&predicate.tag, &predicate.value));
        }
        ConditionExpr::FacetCompare { left, right, .. } => {
            sink(ConditionOperand::ComparisonLeft(left));
            sink(ConditionOperand::ComparisonRight(right));
        }
        ConditionExpr::Not(inner) => visit_operands(inner, sink),
        ConditionExpr::And(lhs, rhs) | ConditionExpr::Or(lhs, rhs) => {
            visit_operands(lhs, sink);
            visit_operands(rhs, sink);
        }
        ConditionExpr::AnyOf(children)
        | ConditionExpr::AllOf(children)
        | ConditionExpr::ExactlyOneOf(children) => {
            for child in children {
                visit_operands(child, sink);
            }
        }
    }
}

#[cfg(test)]
mod tests;
