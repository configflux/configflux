// SPDX-License-Identifier: BUSL-1.1

//! Typed-AST helpers for the loader_api selection-constraint heuristics.
//!
//! configflux-ccs.7 migrates the option-validity / facet-domain resolution in
//! `compiler/src/loader_api/shared_ops.rs` off the string-scanning
//! `parse_conjunction_atoms` / `scan_condition_predicates` path and onto the
//! typed [`ConditionExpr`] produced by [`super::parse_condition_expr`]. These
//! helpers are the typed evaluator that replaces the old
//! string-scanning `SelectionConstraintModel` walks.
//!
//! Behaviour is intentionally a faithful re-expression of the legacy
//! heuristics so the resolver stays byte-stable (ADR-0005). In particular the
//! legacy `option_is_valid` only ever consulted *pure conjunction* conditions
//! (those `parse_condition_conjunction` accepted as a `&&`-chain of `==`/`!=`
//! atoms); disjunctions, negations, parentheses, and the grammar-v2
//! cardinality operators contributed facet domains but never formed a
//! compatibility group. [`is_pure_conjunction`] reproduces that exact
//! eligibility test over the AST, and [`not_contradicted`] reproduces the
//! old `group_compatible` partial evaluation (an unassigned tag is skipped;
//! the condition is contradicted only when an *assigned* atom is false).

use std::collections::BTreeMap;

use super::{ConditionExpr, ConditionPredicate, ConditionPredicateOp};

/// Three-valued result of partially evaluating a [`ConditionExpr`] against an
/// incomplete tag assignment. `Unknown` means at least one referenced tag is
/// unassigned, mirroring the legacy "skip the predicate" behaviour rather than
/// the strict evaluator's missing-tag error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ternary {
    True,
    False,
    Unknown,
}

impl Ternary {
    fn from_bool(value: bool) -> Self {
        if value {
            Ternary::True
        } else {
            Ternary::False
        }
    }

    fn not(self) -> Self {
        match self {
            Ternary::True => Ternary::False,
            Ternary::False => Ternary::True,
            Ternary::Unknown => Ternary::Unknown,
        }
    }

    /// Kleene AND: `False` is absorbing, then `Unknown`, else `True`.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Ternary::False, _) | (_, Ternary::False) => Ternary::False,
            (Ternary::Unknown, _) | (_, Ternary::Unknown) => Ternary::Unknown,
            _ => Ternary::True,
        }
    }

    /// Kleene OR: `True` is absorbing, then `Unknown`, else `False`.
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Ternary::True, _) | (_, Ternary::True) => Ternary::True,
            (Ternary::Unknown, _) | (_, Ternary::Unknown) => Ternary::Unknown,
            _ => Ternary::False,
        }
    }
}

/// Partially evaluate `expr` against `assignments`, skipping any predicate
/// whose tag is unassigned (`Unknown`). This is the typed-AST replacement for
/// the legacy `group_compatible` walk and generalises it to every grammar-v2
/// node, so non-conjunction conditions evaluate sensibly even though the
/// legacy code never grouped them.
fn eval_partial(expr: &ConditionExpr, assignments: &BTreeMap<String, String>) -> Ternary {
    match expr {
        ConditionExpr::Bool(value) => Ternary::from_bool(*value),
        ConditionExpr::Predicate(predicate) => eval_predicate(predicate, assignments),
        ConditionExpr::Not(inner) => eval_partial(inner, assignments).not(),
        ConditionExpr::And(lhs, rhs) => {
            eval_partial(lhs, assignments).and(eval_partial(rhs, assignments))
        }
        ConditionExpr::Or(lhs, rhs) => {
            eval_partial(lhs, assignments).or(eval_partial(rhs, assignments))
        }
        ConditionExpr::AnyOf(children) => children
            .iter()
            .map(|child| eval_partial(child, assignments))
            .fold(Ternary::False, Ternary::or),
        ConditionExpr::AllOf(children) => children
            .iter()
            .map(|child| eval_partial(child, assignments))
            .fold(Ternary::True, Ternary::and),
        ConditionExpr::ExactlyOneOf(children) => eval_exactly_one_of(children, assignments),
    }
}

fn eval_predicate(
    predicate: &ConditionPredicate,
    assignments: &BTreeMap<String, String>,
) -> Ternary {
    let Some(value) = assignments.get(&predicate.tag) else {
        return Ternary::Unknown;
    };
    let holds = match predicate.op {
        ConditionPredicateOp::Eq => value == &predicate.value,
        ConditionPredicateOp::NotEq => value != &predicate.value,
    };
    Ternary::from_bool(holds)
}

/// `exactly_one_of` is "at-least-one OR-reduction" conjoined with pairwise
/// "at-most-one" (ADR-0006 §4). Under partial information we keep the same
/// shape so an unassigned child yields `Unknown` rather than a decision.
fn eval_exactly_one_of(
    children: &[ConditionExpr],
    assignments: &BTreeMap<String, String>,
) -> Ternary {
    let evaluated: Vec<Ternary> = children
        .iter()
        .map(|child| eval_partial(child, assignments))
        .collect();

    let at_least_one = evaluated.iter().copied().fold(Ternary::False, Ternary::or);

    let mut at_most_one = Ternary::True;
    for i in 0..evaluated.len() {
        for j in (i + 1)..evaluated.len() {
            // `!(a && b)` for each pair — both true violates at-most-one.
            at_most_one = at_most_one.and(evaluated[i].and(evaluated[j]).not());
        }
    }

    at_least_one.and(at_most_one)
}

/// Returns `true` when `expr` is not contradicted by `assignments`: the typed
/// replacement for the legacy `group_compatible`. A condition that evaluates
/// to `False` under the (partial) assignment is contradicted; `True` and
/// `Unknown` are both compatible, exactly as the old walk skipped unassigned
/// tags and accepted any group with no failing assigned atom.
pub(crate) fn not_contradicted(
    expr: &ConditionExpr,
    assignments: &BTreeMap<String, String>,
) -> bool {
    eval_partial(expr, assignments) != Ternary::False
}

/// Returns `true` when `expr` is a pure conjunction of `==`/`!=` predicates
/// (`p1 && p2 && …`), the typed equivalent of the legacy
/// `parse_condition_conjunction` acceptance test. The parser lowers `&&`
/// left-associatively (`And(And(p1, p2), p3)`), so we recurse through `And`
/// nodes and require every leaf to be a `Predicate`.
pub(crate) fn is_pure_conjunction(expr: &ConditionExpr) -> bool {
    match expr {
        ConditionExpr::Predicate(_) => true,
        ConditionExpr::And(lhs, rhs) => is_pure_conjunction(lhs) && is_pure_conjunction(rhs),
        _ => false,
    }
}

/// Returns `true` when `expr` contains an equality predicate `tag == value`.
/// Reproduces the legacy "group mentions this facet option" membership test
/// (`predicate.op == Eq && predicate.tag == facet && predicate.value ==
/// option`) directly over the AST.
pub(crate) fn mentions_eq(expr: &ConditionExpr, tag: &str, value: &str) -> bool {
    let mut found = false;
    visit_predicates(expr, &mut |predicate| {
        if predicate.op == ConditionPredicateOp::Eq
            && predicate.tag == tag
            && predicate.value == value
        {
            found = true;
        }
    });
    found
}

/// Invoke `sink` for every equality predicate (`tag == value`) in `expr`, in
/// left-to-right source order. The typed replacement for feeding
/// `scan_condition_predicates` output into `register_facet_domains`: the old
/// code only widened a facet domain for `Eq` atoms, so we expose only those.
pub(crate) fn for_each_eq_predicate<F: FnMut(&str, &str)>(expr: &ConditionExpr, mut sink: F) {
    visit_predicates(expr, &mut |predicate| {
        if predicate.op == ConditionPredicateOp::Eq {
            sink(&predicate.tag, &predicate.value);
        }
    });
}

/// Visit every predicate atom in `expr` — **both** `==` and `!=` — in
/// left-to-right DFS pre-order, reporting each as its `(tag, value)` pair.
///
/// This is the *symbol-universe* walk, and it is deliberately broader than
/// [`for_each_eq_predicate`] (the domain-widening walk, which is `Eq`-only).
/// A `(tag, value)` pair names the same `.ccm` variable under either operator:
/// `ccm_emitter::compile_predicate` lowers `Eq` to `var` and `NotEq` to
/// `not(var)` over that one variable (ADR-0054 §5.2). So the set of pairs
/// reported here is exactly the set of symbols the `var_order` traversal
/// collects from the same expression.
///
/// configflux-9xxq / ADR-0054 §5.1 uses this to land a branch selector's
/// symbols without asserting the selector on the BDD root.
pub(crate) fn for_each_predicate_symbol<F: FnMut(&str, &str)>(expr: &ConditionExpr, mut sink: F) {
    visit_predicates(expr, &mut |predicate| {
        sink(&predicate.tag, &predicate.value);
    });
}

fn visit_predicates<F: FnMut(&ConditionPredicate)>(expr: &ConditionExpr, sink: &mut F) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(predicate) => sink(predicate),
        ConditionExpr::Not(inner) => visit_predicates(inner, sink),
        ConditionExpr::And(lhs, rhs) | ConditionExpr::Or(lhs, rhs) => {
            visit_predicates(lhs, sink);
            visit_predicates(rhs, sink);
        }
        ConditionExpr::AnyOf(children)
        | ConditionExpr::AllOf(children)
        | ConditionExpr::ExactlyOneOf(children) => {
            for child in children {
                visit_predicates(child, sink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse_condition_expr;
    use super::*;

    fn assignments(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn parse(src: &str) -> ConditionExpr {
        parse_condition_expr(src).expect("fixture must parse")
    }

    // ---- is_pure_conjunction ------------------------------------------------

    #[test]
    fn single_predicate_is_a_conjunction() {
        assert!(is_pure_conjunction(&parse("a == 'x'")));
        assert!(is_pure_conjunction(&parse("a != 'x'")));
    }

    #[test]
    fn and_chain_is_a_conjunction() {
        assert!(is_pure_conjunction(&parse("a == 'x' && b == 'y' && c != 'z'")));
    }

    #[test]
    fn disjunction_negation_and_cardinality_are_not_conjunctions() {
        assert!(!is_pure_conjunction(&parse("a == 'x' || b == 'y'")));
        assert!(!is_pure_conjunction(&parse("!(a == 'x')")));
        assert!(!is_pure_conjunction(&parse(
            "a == 'x' && (b == 'y' || c == 'z')"
        )));
        assert!(!is_pure_conjunction(&parse("any_of(a == 'x', b == 'y')")));
        assert!(!is_pure_conjunction(&parse("true")));
    }

    // ---- not_contradicted (legacy group_compatible parity) ------------------

    #[test]
    fn conjunction_with_no_assigned_tags_is_compatible() {
        // Legacy group_compatible skips every unassigned tag => compatible.
        let expr = parse("a == 'x' && b == 'y'");
        assert!(not_contradicted(&expr, &assignments(&[])));
    }

    #[test]
    fn conjunction_with_matching_assignment_is_compatible() {
        let expr = parse("a == 'x' && b == 'y'");
        assert!(not_contradicted(&expr, &assignments(&[("a", "x")])));
        assert!(not_contradicted(
            &expr,
            &assignments(&[("a", "x"), ("b", "y")])
        ));
    }

    #[test]
    fn conjunction_with_conflicting_assignment_is_incompatible() {
        let expr = parse("a == 'x' && b == 'y'");
        // b assigned to the wrong value => the assigned atom is false.
        assert!(!not_contradicted(
            &expr,
            &assignments(&[("a", "x"), ("b", "WRONG")])
        ));
    }

    #[test]
    fn not_eq_predicate_partial_semantics() {
        let expr = parse("region != 'eu'");
        assert!(not_contradicted(&expr, &assignments(&[]))); // unassigned => skip
        assert!(not_contradicted(&expr, &assignments(&[("region", "us")])));
        assert!(!not_contradicted(&expr, &assignments(&[("region", "eu")])));
    }

    // ---- mentions_eq --------------------------------------------------------

    #[test]
    fn mentions_eq_matches_only_equality_on_tag_and_value() {
        let expr = parse("a == 'x' && b != 'y'");
        assert!(mentions_eq(&expr, "a", "x"));
        assert!(!mentions_eq(&expr, "a", "other"));
        assert!(!mentions_eq(&expr, "b", "y")); // b appears only as !=
        assert!(!mentions_eq(&expr, "missing", "x"));
    }

    #[test]
    fn mentions_eq_descends_into_compound_nodes() {
        let expr = parse("a == 'x' || any_of(b == 'y', c == 'z')");
        assert!(mentions_eq(&expr, "a", "x"));
        assert!(mentions_eq(&expr, "b", "y"));
        assert!(mentions_eq(&expr, "c", "z"));
    }

    // ---- for_each_eq_predicate (facet-domain parity) ------------------------

    #[test]
    fn for_each_eq_predicate_collects_only_eq_atoms_in_order() {
        let expr = parse("a == 'x' && b != 'y' && c == 'z'");
        let mut seen = Vec::new();
        for_each_eq_predicate(&expr, |tag, value| {
            seen.push((tag.to_string(), value.to_string()));
        });
        assert_eq!(
            seen,
            vec![
                ("a".to_string(), "x".to_string()),
                ("c".to_string(), "z".to_string()),
            ]
        );
    }

    #[test]
    fn for_each_eq_predicate_walks_disjunctions_like_scan() {
        // Legacy scan_condition_predicates widened domains from atoms inside
        // any structure, including disjunctions; the typed walk matches.
        let expr = parse("a == 'x' || b == 'y'");
        let mut seen = Vec::new();
        for_each_eq_predicate(&expr, |tag, value| {
            seen.push((tag.to_string(), value.to_string()));
        });
        assert_eq!(
            seen,
            vec![
                ("a".to_string(), "x".to_string()),
                ("b".to_string(), "y".to_string()),
            ]
        );
    }

    // ---- Ternary algebra ----------------------------------------------------

    #[test]
    fn ternary_kleene_tables() {
        assert_eq!(Ternary::True.and(Ternary::Unknown), Ternary::Unknown);
        assert_eq!(Ternary::False.and(Ternary::Unknown), Ternary::False);
        assert_eq!(Ternary::True.or(Ternary::Unknown), Ternary::True);
        assert_eq!(Ternary::False.or(Ternary::Unknown), Ternary::Unknown);
        assert_eq!(Ternary::Unknown.not(), Ternary::Unknown);
    }
}
