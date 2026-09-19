// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the grammar-v2 cardinality AST node and its parsing.
//!
//! Split out of `ast.rs` to keep that file under the repo's per-file line
//! limit (`tools/lint_repo.py`). Declared from `ast.rs` via
//! `#[cfg(test)] mod parser_tests;`, so `super::` here resolves to the `ast`
//! module and reaches `parse_condition_expr` / `ConditionExpr` directly.

// ----------------------------------------------------------------------------
// configflux-ccs.2: AST variants for the grammar-v2 cardinality operators
// (any_of / all_of / exactly_one_of, ADR-0006). These tests construct each
// new variant directly and inspect it — parsing is added by configflux-ccs.3
// and BDD lowering by configflux-ccs.4-6, so this task only pins the shape of
// the AST node (children carried as a `Vec<ConditionExpr>` in source order)
// and its derived `Clone` / `PartialEq` behaviour.
// ----------------------------------------------------------------------------
mod cardinality_ast_tests {
    use super::super::ConditionExpr;

    /// Build a `Predicate` cheaply for use as a cardinality child without
    /// reaching for the cardinality keywords (the parser does not yet know
    /// them). Round-trips through `parse_condition_expr` so each child is a
    /// real `ConditionExpr::Predicate`, exactly what the parser will produce
    /// for a list element in configflux-ccs.3.
    fn pred(src: &str) -> ConditionExpr {
        super::super::parse_condition_expr(src).expect("predicate fixture must parse")
    }

    #[test]
    fn any_of_carries_children_in_source_order() {
        let children = vec![pred("a == 'x'"), pred("b == 'y'"), pred("c == 'z'")];
        let expr = ConditionExpr::AnyOf(children.clone());
        match &expr {
            ConditionExpr::AnyOf(got) => {
                assert_eq!(got.len(), 3, "all children retained");
                assert_eq!(got, &children, "children preserved in source order");
            }
            other => panic!("expected AnyOf, got {other:?}"),
        }
    }

    #[test]
    fn all_of_carries_children_in_source_order() {
        let children = vec![pred("a == 'x'"), pred("b == 'y'")];
        let expr = ConditionExpr::AllOf(children.clone());
        match &expr {
            ConditionExpr::AllOf(got) => {
                assert_eq!(got, &children, "children preserved in source order");
            }
            other => panic!("expected AllOf, got {other:?}"),
        }
    }

    #[test]
    fn exactly_one_of_carries_children_in_source_order() {
        let children = vec![pred("a == 'x'"), pred("b == 'y'"), pred("c == 'z'")];
        let expr = ConditionExpr::ExactlyOneOf(children.clone());
        match &expr {
            ConditionExpr::ExactlyOneOf(got) => {
                assert_eq!(got.len(), 3);
                assert_eq!(got, &children, "children preserved in source order");
            }
            other => panic!("expected ExactlyOneOf, got {other:?}"),
        }
    }

    #[test]
    fn cardinality_variants_support_nested_children() {
        // Each list element is a full expression, so cardinality nodes may
        // nest other expressions — including each other (ADR-0006 §2).
        let inner = ConditionExpr::AllOf(vec![pred("a == 'x'"), pred("b == 'y'")]);
        let outer = ConditionExpr::AnyOf(vec![inner.clone(), pred("c == 'z'")]);
        match &outer {
            ConditionExpr::AnyOf(children) => {
                assert_eq!(children.len(), 2);
                assert_eq!(children[0], inner, "nested AllOf retained verbatim");
            }
            other => panic!("expected AnyOf, got {other:?}"),
        }
    }

    #[test]
    fn cardinality_variants_clone_and_compare_by_value() {
        // The derived Clone/PartialEq must treat the new variants
        // structurally and keep distinct operators distinct.
        let children = vec![pred("a == 'x'"), pred("b == 'y'")];
        let any = ConditionExpr::AnyOf(children.clone());
        let all = ConditionExpr::AllOf(children.clone());
        let one = ConditionExpr::ExactlyOneOf(children.clone());

        assert_eq!(any, any.clone(), "AnyOf is Clone + PartialEq");
        assert_eq!(all, all.clone(), "AllOf is Clone + PartialEq");
        assert_eq!(one, one.clone(), "ExactlyOneOf is Clone + PartialEq");

        // Same children, different operator => not equal.
        assert_ne!(any, all);
        assert_ne!(all, one);
        assert_ne!(any, one);

        // Same operator, different children => not equal.
        let any_shorter = ConditionExpr::AnyOf(vec![pred("a == 'x'")]);
        assert_ne!(any, any_shorter);
    }
}

// ----------------------------------------------------------------------------
// configflux-ccs.3: recursive-descent parsing of the grammar-v2 cardinality
// operators. `parse_condition_expr` now recognises any_of/all_of/
// exactly_one_of calls whose arguments are full expressions (each parsed by
// `parse_or`). These tests exercise the multi-arg happy path for each
// operator, the >=2-argument requirement (single-arg rejection), trailing-comma
// rejection, and nested boolean arguments. BDD lowering remains deferred to
// configflux-ccs.4-6 (ADR-0006 §3-5).
// ----------------------------------------------------------------------------
mod parser_cardinality_tests {
    use super::super::{parse_condition_expr, ConditionExpr};

    /// Parse a fixture child via the public entry point so each expected child
    /// is exactly the `ConditionExpr` the parser produces for that source.
    fn pred(src: &str) -> ConditionExpr {
        parse_condition_expr(src).expect("predicate fixture must parse")
    }

    #[test]
    fn any_of_parses_multiple_args() {
        let expr = parse_condition_expr("any_of(a == 'x', b == 'y')").unwrap();
        assert_eq!(
            expr,
            ConditionExpr::AnyOf(vec![pred("a == 'x'"), pred("b == 'y'")])
        );
    }

    #[test]
    fn all_of_parses_three_args() {
        let expr = parse_condition_expr("all_of(a == 'x', b == 'y', c == 'z')").unwrap();
        assert_eq!(
            expr,
            ConditionExpr::AllOf(vec![pred("a == 'x'"), pred("b == 'y'"), pred("c == 'z'")])
        );
    }

    #[test]
    fn exactly_one_of_parses_multiple_args() {
        let expr = parse_condition_expr("exactly_one_of(a == 'x', b == 'y')").unwrap();
        assert_eq!(
            expr,
            ConditionExpr::ExactlyOneOf(vec![pred("a == 'x'"), pred("b == 'y'")])
        );
    }

    #[test]
    fn cardinality_args_preserve_source_order() {
        let expr = parse_condition_expr("any_of(c == 'z', a == 'x', b == 'y')").unwrap();
        match expr {
            ConditionExpr::AnyOf(children) => {
                assert_eq!(children[0], pred("c == 'z'"));
                assert_eq!(children[1], pred("a == 'x'"));
                assert_eq!(children[2], pred("b == 'y'"));
            }
            other => panic!("expected AnyOf, got {other:?}"),
        }
    }

    #[test]
    fn any_of_single_arg_is_rejected() {
        let err = parse_condition_expr("any_of(a == 'x')").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("at least 2 arguments") && msg.contains("any_of"),
            "single-arg error should be descriptive, got: {msg}"
        );
    }

    #[test]
    fn all_of_single_arg_is_rejected() {
        let err = parse_condition_expr("all_of(a == 'x')").unwrap_err();
        assert!(
            format!("{err}").contains("at least 2 arguments"),
            "err: {err}"
        );
    }

    #[test]
    fn exactly_one_of_single_arg_is_rejected() {
        let err = parse_condition_expr("exactly_one_of(a == 'x')").unwrap_err();
        assert!(
            format!("{err}").contains("at least 2 arguments"),
            "err: {err}"
        );
    }

    #[test]
    fn trailing_comma_is_rejected() {
        let err = parse_condition_expr("any_of(a == 'x', b == 'y',)").unwrap_err();
        assert!(
            format!("{err}").contains("Trailing comma"),
            "err: {err}"
        );
    }

    #[test]
    fn nested_boolean_arg_inside_cardinality_call() {
        // An argument is a full expression: a parenthesised disjunction and a
        // negation both round-trip through the same parser the list uses.
        let expr =
            parse_condition_expr("any_of((a == 'x' || b == 'y'), !(c == 'z'))").unwrap();
        assert_eq!(
            expr,
            ConditionExpr::AnyOf(vec![
                pred("(a == 'x' || b == 'y')"),
                pred("!(c == 'z')"),
            ])
        );
    }

    #[test]
    fn cardinality_calls_nest() {
        // Cardinality nodes may appear as arguments to each other.
        let expr =
            parse_condition_expr("all_of(any_of(a == 'x', b == 'y'), c == 'z')").unwrap();
        assert_eq!(
            expr,
            ConditionExpr::AllOf(vec![
                ConditionExpr::AnyOf(vec![pred("a == 'x'"), pred("b == 'y'")]),
                pred("c == 'z'"),
            ])
        );
    }

    #[test]
    fn cardinality_keyword_as_predicate_tag_still_parses() {
        // The keyword is only a cardinality operator when followed by `(`.
        // `any_of == 'x'` is a predicate on a tag named `any_of`.
        let expr = parse_condition_expr("any_of == 'x'").unwrap();
        assert_eq!(expr, pred("any_of == 'x'"));
        match expr {
            ConditionExpr::Predicate(_) => {}
            other => panic!("expected Predicate, got {other:?}"),
        }
    }

    #[test]
    fn cardinality_missing_closing_paren_is_rejected() {
        let err = parse_condition_expr("any_of(a == 'x', b == 'y'").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("missing closing parenthesis") || msg.contains("Unterminated"),
            "err: {msg}"
        );
    }
}

// ----------------------------------------------------------------------------
// configflux-secb.2 / ADR-0057 §D5: facet-to-facet comparison. A predicate's
// right-hand side may be an UNQUOTED identifier naming another declared facet
// or binding, so a constraint can require two independently bound choices to
// hold the same value without merging them into one facet. A QUOTED right-hand
// side keeps its literal meaning, so every model authored before this change
// parses unchanged.
// ----------------------------------------------------------------------------
mod facet_comparison_tests {
    use super::super::{parse_condition_expr, ConditionExpr, ConditionPredicateOp};

    fn compare(left: &str, op: ConditionPredicateOp, right: &str) -> ConditionExpr {
        ConditionExpr::FacetCompare {
            left: left.to_string(),
            op,
            right: right.to_string(),
        }
    }

    #[test]
    fn unquoted_rhs_parses_as_facet_equality() {
        let expr = parse_condition_expr("a == b").unwrap();
        assert_eq!(expr, compare("a", ConditionPredicateOp::Eq, "b"));
    }

    #[test]
    fn unquoted_rhs_parses_as_facet_inequality() {
        let expr = parse_condition_expr("a != b").unwrap();
        assert_eq!(expr, compare("a", ConditionPredicateOp::NotEq, "b"));
    }

    #[test]
    fn facet_comparison_composes_with_boolean_operators() {
        let expr = parse_condition_expr("!(a == b) || c == 'x'").unwrap();
        assert_eq!(
            expr,
            ConditionExpr::Or(
                Box::new(ConditionExpr::Not(Box::new(compare(
                    "a",
                    ConditionPredicateOp::Eq,
                    "b"
                )))),
                Box::new(parse_condition_expr("c == 'x'").unwrap()),
            )
        );
    }

    #[test]
    fn facet_comparison_nests_inside_cardinality_call() {
        let expr = parse_condition_expr("any_of(a == b, c == 'x')").unwrap();
        assert_eq!(
            expr,
            ConditionExpr::AnyOf(vec![
                compare("a", ConditionPredicateOp::Eq, "b"),
                parse_condition_expr("c == 'x'").unwrap(),
            ])
        );
    }

    #[test]
    fn quoted_rhs_still_parses_as_a_literal_predicate() {
        // The whole point of the quoting rule: `b` in quotes is the value `b`,
        // not the facet `b`. Every pre-existing model depends on this.
        let expr = parse_condition_expr("a == 'b'").unwrap();
        match expr {
            ConditionExpr::Predicate(ref p) => {
                assert_eq!(p.tag, "a");
                assert_eq!(p.value, "b");
                assert_eq!(p.op, ConditionPredicateOp::Eq);
            }
            other => panic!("expected Predicate, got {other:?}"),
        }
        assert_eq!(expr, parse_condition_expr("a == \"b\"").unwrap());
    }

    #[test]
    fn numeric_rhs_is_rejected() {
        // An identifier must start with an ASCII lowercase letter, so a bare
        // number is neither a literal nor a facet name.
        let err = parse_condition_expr("a == 3").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("quoted string literal") && msg.contains("identifier"),
            "error must name both accepted right-hand sides, got: {msg}"
        );
    }

    #[test]
    fn trailing_identifier_after_facet_comparison_is_rejected() {
        let err = parse_condition_expr("a == b c").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Unexpected trailing input"),
            "err: {msg}"
        );
    }

    #[test]
    fn facet_comparison_tolerates_surrounding_whitespace() {
        let tight = parse_condition_expr("a==b").unwrap();
        let loose = parse_condition_expr("  a   ==   b  ").unwrap();
        assert_eq!(tight, compare("a", ConditionPredicateOp::Eq, "b"));
        assert_eq!(tight, loose);
    }

    #[test]
    fn facet_comparison_operands_may_carry_digits_and_underscores() {
        let expr = parse_condition_expr("sorter_container != line_container2").unwrap();
        assert_eq!(
            expr,
            compare(
                "sorter_container",
                ConditionPredicateOp::NotEq,
                "line_container2"
            )
        );
    }
}
