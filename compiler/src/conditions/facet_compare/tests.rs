// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the facet-to-facet comparison expansion
//! (configflux-secb.2 / ADR-0057 §D5).
//!
//! Every expectation is written as a CONDITION STRING and parsed, so the test
//! states the expansion in the grammar an author would read rather than in
//! constructor calls. Because `&&` binds tighter than `||` and the fold is
//! left-associative, the parsed reference also pins the fold order: a
//! different association parses to a different tree and the assertion fails.

use std::collections::BTreeMap;

use super::super::{parse_condition_expr, ConditionExpr};
use super::{expand_facet_comparisons, FacetDomains};

fn domains(entries: &[(&str, &[&str])]) -> FacetDomains {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, values) in entries {
        out.insert(
            name.to_string(),
            values.iter().map(|v| v.to_string()).collect(),
        );
    }
    out
}

fn expand(source: &str, entries: &[(&str, &[&str])]) -> ConditionExpr {
    let parsed = parse_condition_expr(source).expect("fixture must parse");
    expand_facet_comparisons(&parsed, &domains(entries)).expect("expansion must succeed")
}

fn expect(source: &str) -> ConditionExpr {
    parse_condition_expr(source).expect("expectation must parse")
}

#[test]
fn equality_over_identical_domains_is_a_pairwise_equivalence() {
    let got = expand("a == b", &[("a", &["c1", "c2"]), ("b", &["c1", "c2"])]);
    assert_eq!(
        got,
        expect(
            "(a == 'c1' && b == 'c1' || a != 'c1' && b != 'c1') && \
             (a == 'c2' && b == 'c2' || a != 'c2' && b != 'c2')"
        )
    );
}

#[test]
fn inequality_is_the_negation_of_the_equality() {
    let entries: &[(&str, &[&str])] = &[("a", &["c1", "c2"]), ("b", &["c1", "c2"])];
    let equality = expand("a == b", entries);
    let inequality = expand("a != b", entries);
    assert_eq!(inequality, ConditionExpr::Not(Box::new(equality)));
}

#[test]
fn a_value_only_the_right_side_declares_becomes_a_bare_negative_literal() {
    // ADR-0057 §D5: `x.v ≡ false when v ∉ dom(x)`, so `a.c2 ⇔ b.c2` collapses
    // to `¬b.c2`. This is what makes `b == 'c2'` unreachable under `a == b`
    // without relying on the closed-facet cardinality conjuncts.
    let got = expand("a == b", &[("a", &["c1"]), ("b", &["c1", "c2"])]);
    assert_eq!(
        got,
        expect("(a == 'c1' && b == 'c1' || a != 'c1' && b != 'c1') && b != 'c2'")
    );
}

#[test]
fn a_value_only_the_left_side_declares_becomes_a_bare_negative_literal() {
    let got = expand("a == b", &[("a", &["c1", "c2"]), ("b", &["c1"])]);
    assert_eq!(
        got,
        expect("(a == 'c1' && b == 'c1' || a != 'c1' && b != 'c1') && a != 'c2'")
    );
}

#[test]
fn fold_order_is_the_left_sides_declared_order_not_a_sorted_one() {
    // dom(a) = [c2, c1] and dom(b) = [c1, c2, c3]: the union is c2, c1, c3 —
    // `a`'s DECLARED order first (so `c2` precedes `c1`), then `b`'s remaining
    // value. Emitted bytes depend on this (ADR-0006 §5), and sorting the union
    // instead would reverse the first two terms.
    let got = expand("a == b", &[("a", &["c2", "c1"]), ("b", &["c1", "c2", "c3"])]);
    assert_eq!(
        got,
        expect(
            "(a == 'c2' && b == 'c2' || a != 'c2' && b != 'c2') && \
             (a == 'c1' && b == 'c1' || a != 'c1' && b != 'c1') && \
             b != 'c3'"
        )
    );
}

#[test]
fn the_right_sides_remainder_keeps_its_own_declared_order() {
    // dom(a) = [c1] and dom(b) = [c3, c2]: nothing overlaps, so every term is
    // a one-sided negative literal — `a`'s value first, then `b`'s two in
    // `b`'s declared order (c3 before c2, not sorted).
    let got = expand("a == b", &[("a", &["c1"]), ("b", &["c3", "c2"])]);
    assert_eq!(got, expect("a != 'c1' && b != 'c3' && b != 'c2'"));
}

#[test]
fn a_single_value_domain_yields_one_equivalence_term() {
    let got = expand("a == b", &[("a", &["only"]), ("b", &["only"])]);
    assert_eq!(
        got,
        expect("a == 'only' && b == 'only' || a != 'only' && b != 'only'")
    );
}

#[test]
fn disjoint_domains_make_equality_unsatisfiable_by_construction() {
    // Nothing to agree on: every value is one-sided, so the expansion is the
    // conjunction of both sides' negative literals.
    let got = expand("a == b", &[("a", &["x"]), ("b", &["y"])]);
    assert_eq!(got, expect("a != 'x' && b != 'y'"));
}

#[test]
fn expansion_recurses_through_boolean_and_cardinality_nodes() {
    let entries: &[(&str, &[&str])] = &[("a", &["c1"]), ("b", &["c1"]), ("c", &["z"])];
    let equivalence = "(a == 'c1' && b == 'c1' || a != 'c1' && b != 'c1')";

    assert_eq!(
        expand("!(a == b) || c == 'z'", entries),
        expect(&format!("!{equivalence} || c == 'z'"))
    );
    assert_eq!(
        expand("any_of(a == b, c == 'z')", entries),
        expect(&format!("any_of({equivalence}, c == 'z')"))
    );
    assert_eq!(
        expand("c == 'z' && a != b", entries),
        expect(&format!("c == 'z' && !{equivalence}"))
    );
}

#[test]
fn an_expression_without_comparisons_is_returned_unchanged() {
    let source = "any_of(a == 'c1', !(b != 'c1')) && true";
    let parsed = parse_condition_expr(source).expect("fixture must parse");
    let expanded = expand_facet_comparisons(&parsed, &domains(&[])).expect("expansion");
    assert_eq!(parsed, expanded);
}

#[test]
fn an_operand_with_no_declared_domain_is_an_internal_error_naming_it() {
    let parsed = parse_condition_expr("a == b").expect("fixture must parse");
    let err = expand_facet_comparisons(&parsed, &domains(&[("a", &["c1"])])).unwrap_err();
    let message = format!("{err}");
    assert!(
        message.contains("'b'") && message.contains("no declared value domain"),
        "err: {message}"
    );
}
