// SPDX-License-Identifier: BUSL-1.1

//! Exhaustive-implication evaluator over the typed [`ConditionExpr`] AST
//! (configflux-uiyo).
//!
//! Typed replacement for the legacy string-scanning `eval_implication_by_matrix`
//! engine (the former `condition_implies` matrix cluster). `link_verify` uses
//! [`condition_expr_implies`] to decide component-dependency activation
//! subsumption: a component may only `depends_on` another when the dependent's
//! activation condition holds whenever the depender's does (`A => B` over every
//! tag assignment).
//!
//! Semantics are a faithful re-expression of the legacy matrix so the resolver
//! stays byte-stable (ADR-0005, ADR-0008). The legacy engine:
//!   1. scanned *every* atom (both `==` and `!=`) out of both conditions to
//!      learn each tag's relevant value set;
//!   2. extended each tag's domain with a single fresh sentinel so "some other
//!      value" is represented exactly once;
//!   3. enumerated the full Cartesian product of those finite domains and, for
//!      each *total* assignment, checked `!(A && !B)` with the strict evaluator.
//! Because every referenced tag is assigned in a total assignment, the strict
//! evaluator never short-circuits over a missing tag, so the walk reduces to a
//! plain two-valued evaluation. [`eval_total`] reproduces that two-valued walk
//! over the typed AST, and [`condition_expr_implies`] reproduces the truth-table
//! search. The sentinel-selection rule matches the legacy `unique_sentinel`
//! exactly so identical domains are enumerated.

use std::collections::{BTreeMap, BTreeSet};

use super::{ConditionExpr, ConditionPredicate, ConditionPredicateOp};

/// Exhaustively decide whether `antecedent` implies `consequent` (`A => B`):
/// returns `true` iff there is no tag assignment, drawn from the finite domains
/// induced by the atoms of both expressions plus a fresh per-tag sentinel,
/// under which `antecedent` is true and `consequent` is false. This is the
/// typed equivalent of the legacy `eval_implication_by_matrix`.
pub(crate) fn condition_expr_implies(
    antecedent: &ConditionExpr,
    consequent: &ConditionExpr,
) -> bool {
    let mut tag_values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    collect_atom_values(antecedent, &mut tag_values);
    collect_atom_values(consequent, &mut tag_values);

    // Tags in sorted order (BTreeMap iterates sorted), each with its sorted
    // observed values plus one fresh sentinel standing for "any other value".
    let tags: Vec<String> = tag_values.keys().cloned().collect();
    let domains: Vec<Vec<String>> = tags
        .iter()
        .map(|tag| {
            let mut domain: Vec<String> = tag_values[tag].iter().cloned().collect();
            domain.push(unique_sentinel(&domain));
            domain
        })
        .collect();

    let mut assignments: BTreeMap<String, String> = BTreeMap::new();
    holds_for_all_assignments(0, &tags, &domains, &mut assignments, antecedent, consequent)
}

/// Recurse over the Cartesian product of `domains`, building a total assignment
/// one tag at a time. Returns `false` as soon as a counterexample is found
/// (`antecedent` true while `consequent` false), mirroring the legacy
/// `check_assignments` early-exit.
fn holds_for_all_assignments(
    idx: usize,
    tags: &[String],
    domains: &[Vec<String>],
    assignments: &mut BTreeMap<String, String>,
    antecedent: &ConditionExpr,
    consequent: &ConditionExpr,
) -> bool {
    if idx == tags.len() {
        let a_val = eval_total(antecedent, assignments);
        let b_val = eval_total(consequent, assignments);
        return !(a_val && !b_val);
    }

    let tag = &tags[idx];
    for value in &domains[idx] {
        assignments.insert(tag.clone(), value.clone());
        if !holds_for_all_assignments(idx + 1, tags, domains, assignments, antecedent, consequent) {
            assignments.remove(tag);
            return false;
        }
    }
    assignments.remove(tag);
    true
}

/// Two-valued evaluation of `expr` against a *total* `assignments` map (every
/// referenced tag is present). This is the typed analogue of the strict
/// `eval::eval_condition_impl` walk restricted to complete assignments, so it
/// never needs the `Unknown` rung the selection-eval partial walk carries. A
/// tag absent from `assignments` evaluates its predicate to `false`, matching
/// the legacy matrix where every scanned tag was always assigned (the case
/// never arises for tags drawn from the expressions themselves).
fn eval_total(expr: &ConditionExpr, assignments: &BTreeMap<String, String>) -> bool {
    match expr {
        ConditionExpr::Bool(value) => *value,
        ConditionExpr::Predicate(predicate) => eval_total_predicate(predicate, assignments),
        ConditionExpr::Not(inner) => !eval_total(inner, assignments),
        ConditionExpr::And(lhs, rhs) => {
            eval_total(lhs, assignments) && eval_total(rhs, assignments)
        }
        ConditionExpr::Or(lhs, rhs) => {
            eval_total(lhs, assignments) || eval_total(rhs, assignments)
        }
        ConditionExpr::AnyOf(children) => {
            children.iter().any(|child| eval_total(child, assignments))
        }
        ConditionExpr::AllOf(children) => {
            children.iter().all(|child| eval_total(child, assignments))
        }
        ConditionExpr::ExactlyOneOf(children) => {
            children
                .iter()
                .filter(|child| eval_total(child, assignments))
                .count()
                == 1
        }
    }
}

fn eval_total_predicate(
    predicate: &ConditionPredicate,
    assignments: &BTreeMap<String, String>,
) -> bool {
    match assignments.get(&predicate.tag) {
        Some(value) => match predicate.op {
            ConditionPredicateOp::Eq => value == &predicate.value,
            ConditionPredicateOp::NotEq => value != &predicate.value,
        },
        None => false,
    }
}

/// Collect, for every predicate in `expr` (both `==` and `!=`), the tag and the
/// literal it references into `tag_values`. The legacy `scan_atoms` widened a
/// tag's domain from *every* atom regardless of operator or surrounding
/// structure, so we walk all predicates the same way.
fn collect_atom_values(expr: &ConditionExpr, tag_values: &mut BTreeMap<String, BTreeSet<String>>) {
    for_each_predicate(expr, &mut |predicate| {
        tag_values
            .entry(predicate.tag.clone())
            .or_default()
            .insert(predicate.value.clone());
    });
}

/// Pick a value distinct from every entry in `values`, standing for "any tag
/// value not named by an atom". Mirrors the legacy `unique_sentinel`: prefer
/// `__other__`, then disambiguate with a numeric suffix on collision so the
/// enumerated domain is identical to the matrix engine's.
fn unique_sentinel(values: &[String]) -> String {
    let base = "__other__";
    if !values.iter().any(|v| v == base) {
        return base.to_string();
    }
    let mut counter = 1;
    loop {
        let candidate = format!("{base}{counter}");
        if !values.iter().any(|v| v == &candidate) {
            return candidate;
        }
        counter += 1;
    }
}

/// Invoke `sink` for every predicate node in `expr`, in left-to-right source
/// order. Self-contained so the implication engine carries no dependency on the
/// selection-eval walker.
fn for_each_predicate<F: FnMut(&ConditionPredicate)>(expr: &ConditionExpr, sink: &mut F) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(predicate) => sink(predicate),
        ConditionExpr::Not(inner) => for_each_predicate(inner, sink),
        ConditionExpr::And(lhs, rhs) | ConditionExpr::Or(lhs, rhs) => {
            for_each_predicate(lhs, sink);
            for_each_predicate(rhs, sink);
        }
        ConditionExpr::AnyOf(children)
        | ConditionExpr::AllOf(children)
        | ConditionExpr::ExactlyOneOf(children) => {
            for child in children {
                for_each_predicate(child, sink);
            }
        }
    }
}

#[cfg(test)]
mod tests;
