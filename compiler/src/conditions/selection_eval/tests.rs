// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the typed-AST selection-constraint helpers.
//!
//! Split out of `selection_eval.rs` to keep that file under the repo's
//! per-file line limit (`tools/lint_repo.py`). Declared from
//! `selection_eval.rs` via `#[cfg(test)] mod tests;`, so `super::` here
//! resolves to the `selection_eval` module.

#[cfg(test)]
mod tests {
    use crate::conditions::parse_condition_expr;
    use super::super::*;

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

    // ---- is_contradicted over a restricted world (configflux-vfh5) ----------

    fn restricted(pairs: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
        pairs
            .iter()
            .map(|(facet, values)| {
                (
                    (*facet).to_string(),
                    values.iter().map(|v| (*v).to_string()).collect(),
                )
            })
            .collect()
    }

    fn contradicted(src: &str, world: &[(&str, &[&str])]) -> bool {
        let open = restricted(world);
        is_contradicted(&parse(src), &FacetWorld::Restricted(&open))
    }

    #[test]
    fn a_one_value_restriction_decides_a_predicate_exactly_as_a_binding_does() {
        // The two worlds must agree wherever both can answer, because that
        // agreement is what lets `not_contradicted` delegate here without
        // moving any verdict the option-validity and resolve callers see.
        let expr = parse("a == 'x' && b != 'y'");
        assert!(!is_contradicted(
            &expr,
            &FacetWorld::Bound(&assignments(&[("a", "x"), ("b", "z")]))
        ));
        assert!(!contradicted("a == 'x' && b != 'y'", &[("a", &["x"]), ("b", &["z"])]));

        assert!(is_contradicted(
            &expr,
            &FacetWorld::Bound(&assignments(&[("a", "x"), ("b", "y")]))
        ));
        assert!(contradicted("a == 'x' && b != 'y'", &[("a", &["x"]), ("b", &["y"])]));
    }

    #[test]
    fn a_predicate_is_false_once_its_value_is_ruled_out() {
        // The capability the restricted world exists for: `x` is gone from the
        // values `a` may still take, so the predicate cannot hold — even though
        // two values remain and no single one is entailed.
        assert!(contradicted("a == 'x'", &[("a", &["y", "z"])]));
        // ... and the disjunction over two ruled-out values goes with it, which
        // is the shape a requirement's `accepts` list lowers to.
        assert!(contradicted("any_of(a == 'x', a == 'w')", &[("a", &["y", "z"])]));
    }

    #[test]
    fn a_predicate_whose_value_is_still_open_stays_unknown() {
        // Two values left including the one asked about: the predicate may or
        // may not hold, and `Unknown` is not a contradiction (ADR-0054 §2).
        assert!(!contradicted("a == 'x'", &[("a", &["x", "y"])]));
        assert!(!contradicted("a != 'x'", &[("a", &["x", "y"])]));
    }

    #[test]
    fn an_equality_is_contradicted_exactly_when_the_two_sides_are_disjoint() {
        // ADR-0057 §D5 under partial facts. Disjoint sets can never be equal,
        // whatever their sizes; sets that overlap may still agree.
        assert!(contradicted("a == b", &[("a", &["x"]), ("b", &["y", "z"])]));
        assert!(!contradicted("a == b", &[("a", &["x"]), ("b", &["x", "z"])]));
        // The inequality is the negation, so it flips both verdicts: two sides
        // pinned to the same value certainly agree, hence `!=` is contradicted.
        assert!(!contradicted("a != b", &[("a", &["x"]), ("b", &["y", "z"])]));
        assert!(contradicted("a != b", &[("a", &["x"]), ("b", &["x"])]));
    }

    #[test]
    fn a_facet_the_world_says_nothing_about_is_never_a_contradiction() {
        // Both the absent facet and the EMPTY set are `Unknown`. The empty set
        // is what an ADR-0054 §5.4 at-least-one clause produces — every
        // declared value ruled out — and reading it as "satisfies nothing"
        // would let the model's own bookkeeping be reported as authored policy.
        assert!(!contradicted("a == 'x'", &[("b", &["y"])]));
        assert!(!contradicted("a == 'x'", &[("a", &[])]));
        assert!(!contradicted("a == b", &[("a", &[]), ("b", &["y"])]));
    }
}
