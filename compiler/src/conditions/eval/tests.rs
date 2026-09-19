// SPDX-License-Identifier: BUSL-1.1

//! Grammar-coverage tests for the hand-rolled string evaluator (ADR-0008),
//! plus the facet-to-facet comparison cases added by configflux-secb.2
//! (ADR-0057 §D5).
//!
//! Split out of `eval.rs` to keep that file under the repo's per-file line
//! limit (`tools/lint_repo.py`). Declared from `eval.rs` via
//! `#[cfg(test)] mod tests;`, so `super::` here resolves to the `eval` module.

// ----------------------------------------------------------------------------
// Grammar coverage tests for the hand-rolled evaluator (ADR-0008).
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::conditions::eval_condition;
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

// ----------------------------------------------------------------------------
// configflux-secb.2 / ADR-0057 §D5: facet-to-facet comparison in the string
// evaluator — the path a selector `condition` takes at resolve time. The typed
// AST parser accepts this form, so this one must too, including on the
// short-circuited branches it only skips over.
// ----------------------------------------------------------------------------
#[cfg(test)]
mod facet_comparison_eval_tests {
    use crate::conditions::eval_condition;
    use std::collections::HashMap;

    fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn equality_holds_when_both_facets_are_bound_to_the_same_value() {
        let env = tags(&[("a", "c1"), ("b", "c1")]);
        assert!(eval_condition("a == b", &env).unwrap());
        assert!(!eval_condition("a != b", &env).unwrap());
    }

    #[test]
    fn equality_fails_when_the_two_facets_differ() {
        let env = tags(&[("a", "c1"), ("b", "c2")]);
        assert!(!eval_condition("a == b", &env).unwrap());
        assert!(eval_condition("a != b", &env).unwrap());
    }

    #[test]
    fn quoted_right_hand_side_still_compares_against_the_literal() {
        // `b` in quotes is the value `b`, not the facet `b`.
        let env = tags(&[("a", "b"), ("b", "c2")]);
        assert!(eval_condition("a == 'b'", &env).unwrap());
        assert!(!eval_condition("a == b", &env).unwrap());
    }

    #[test]
    fn an_unbound_operand_is_an_error_naming_it() {
        let env = tags(&[("a", "c1")]);
        let err = eval_condition("a == b", &env).unwrap_err();
        assert!(
            format!("{err:#}").contains("Missing tag 'b'"),
            "err: {err:#}"
        );
    }

    #[test]
    fn a_short_circuited_facet_comparison_is_skipped_not_rejected() {
        // The `||` left side already decided the value, so the comparison is
        // parsed for structure only and never evaluated — `b` may be unbound.
        let env = tags(&[("a", "c1"), ("c", "yes")]);
        assert!(eval_condition("c == 'yes' || a == b", &env).unwrap());
        assert!(!eval_condition("c == 'no' && a == b", &env).unwrap());
    }

    #[test]
    fn facet_comparison_composes_with_boolean_operators() {
        let env = tags(&[("a", "c1"), ("b", "c1"), ("c", "z")]);
        assert!(eval_condition("!(a != b) && c == 'z'", &env).unwrap());
        assert!(eval_condition("(a == b) || c == 'q'", &env).unwrap());
    }

    #[test]
    fn a_numeric_right_hand_side_is_still_rejected() {
        let env = tags(&[("a", "c1")]);
        let err = eval_condition("a == 3", &env).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("quoted string literal") && msg.contains("identifier"),
            "err: {msg}"
        );
    }
}
