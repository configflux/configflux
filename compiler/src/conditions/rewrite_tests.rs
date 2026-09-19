// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the condition identifier-rewrite and -enumeration entry
//! points (configflux-p2sz.4 / ADR-0034 D3). These prove that the scrubber's
//! mandated parse → substitute → re-serialize transform preserves the
//! constraint structure exactly: every rewritten condition re-parses to an AST
//! isomorphic to the substituted one, and the enumerator derives the complete
//! tag/option set the discrimination rule depends on.

use super::super::{parse_condition_expr, ConditionExpr};
use super::{condition_identifiers, rewrite_condition_identifiers};

/// Identity renamers: leave every identifier unchanged. Used to assert that
/// re-serialization alone is structure-preserving (the rewritten string
/// re-parses to the *same* AST as the input) independent of substitution.
fn id_tag(t: &str) -> String {
    t.to_string()
}
fn id_lit(_tag: &str, lit: &str) -> String {
    lit.to_string()
}

fn ast(src: &str) -> ConditionExpr {
    parse_condition_expr(src).expect("fixture condition must parse")
}

#[test]
fn reserialize_roundtrips_to_same_ast_for_all_shapes() {
    // A representative condition exercising every grammar construct: ==, !=,
    // &&, ||, !, parentheses (precedence), and all three cardinality operators
    // including nesting. The re-serialized string must re-parse to the SAME AST.
    let cases = [
        "a == 'x'",
        "a != 'y'",
        "a == 'x' && b == 'y'",
        "a == 'x' || b == 'y'",
        "!(a == 'x')",
        "a == 'x' && (b == 'y' || c == 'z')",
        "(a == 'x' || b == 'y') && c != 'z'",
        "!a == 'x' || b == 'y'",
        "any_of(a == 'x', b == 'y')",
        "all_of(a == 'x', b == 'y', c == 'z')",
        "exactly_one_of(a == 'x', b == 'y')",
        "any_of(a == 'x', all_of(b == 'y', c == 'z'))",
        "a == 'x' && any_of(b == 'y', c == 'z')",
        "true",
        "false",
    ];
    for src in cases {
        let rewritten = rewrite_condition_identifiers(src, &id_tag, &id_lit)
            .unwrap_or_else(|e| panic!("rewrite of {src:?} failed: {e}"));
        let before = ast(src);
        let after = ast(&rewritten);
        assert_eq!(
            before, after,
            "re-serialized {src:?} -> {rewritten:?} parsed to a different AST"
        );
    }
}

#[test]
fn substitution_renames_tags_and_literals_consistently() {
    let rename_tag = |t: &str| format!("tag_{t}");
    let rename_lit = |_tag: &str, lit: &str| format!("opt_{lit}");
    let out =
        rewrite_condition_identifiers("variant == 'heavy' && region != 'eu'", &rename_tag, &rename_lit)
            .unwrap();
    // The original identifiers must not survive as standalone tokens. We check
    // the parsed AST rather than substrings, because a pseudonym legitimately
    // *contains* the original spelling (`tag_variant` contains `variant`); only
    // a token-level recurrence of the original would be a real leak.
    let ids = condition_identifiers(&out).unwrap();
    assert!(!ids.tags.contains("variant"), "original tag leaked: {out}");
    assert!(!ids.tags.contains("region"), "original tag leaked: {out}");
    for (_t, lit) in &ids.options {
        assert_ne!(lit, "heavy", "original literal leaked: {out}");
        assert_ne!(lit, "eu", "original literal leaked: {out}");
    }
    assert!(ids.tags.contains("tag_variant"), "renamed tag missing: {out}");
    assert!(
        ids.options
            .contains(&("tag_variant".to_string(), "opt_heavy".to_string())),
        "renamed literal missing: {out}"
    );
    assert!(out.contains("&&"), "operator structure lost: {out}");
    assert!(out.contains("!="), "operator structure lost: {out}");
    // And the rewritten string is itself valid grammar.
    parse_condition_expr(&out).expect("rewritten condition must re-parse");
}

#[test]
fn rewrite_is_idempotent_under_stable_map() {
    // Re-serialization is canonical: rewriting an already-canonical string with
    // identity renamers is a fixed point.
    let canonical = rewrite_condition_identifiers(
        "a == 'x' && (b == 'y' || c == 'z')",
        &id_tag,
        &id_lit,
    )
    .unwrap();
    let again = rewrite_condition_identifiers(&canonical, &id_tag, &id_lit).unwrap();
    assert_eq!(canonical, again, "re-serialization is not idempotent");
}

#[test]
fn enumerate_collects_all_tags_and_options() {
    let ids = condition_identifiers(
        "variant == 'heavy' && (region != 'eu' || region == 'us')",
    )
    .unwrap();
    assert!(ids.tags.contains("variant"));
    assert!(ids.tags.contains("region"));
    assert_eq!(ids.tags.len(), 2);
    assert!(ids.options.contains(&("variant".to_string(), "heavy".to_string())));
    assert!(ids.options.contains(&("region".to_string(), "eu".to_string())));
    assert!(ids.options.contains(&("region".to_string(), "us".to_string())));
    assert_eq!(ids.options.len(), 3);
}

#[test]
fn enumerate_walks_cardinality_children() {
    let ids =
        condition_identifiers("any_of(a == 'x', all_of(b == 'y', c != 'z'))").unwrap();
    assert_eq!(ids.tags, ["a", "b", "c"].iter().map(|s| s.to_string()).collect());
    assert!(ids.options.contains(&("c".to_string(), "z".to_string())));
}

#[test]
fn malformed_condition_surfaces_parse_error() {
    let err = condition_identifiers("variant ==").unwrap_err();
    assert!(
        format!("{err}").to_lowercase().contains("quoted")
            || format!("{err}").to_lowercase().contains("literal"),
        "unexpected error: {err}"
    );
    // `a == b` is no longer malformed — an unquoted right-hand side is a
    // facet-to-facet comparison (configflux-secb.2 / ADR-0057 §D5). A
    // right-hand side that is neither a literal nor an identifier still is.
    let err2 = rewrite_condition_identifiers("a == 3", &id_tag, &id_lit).unwrap_err();
    assert!(!format!("{err2}").is_empty());
}

#[test]
fn facet_comparison_round_trips_with_an_unquoted_right_hand_side() {
    // The serializer must NOT quote the right-hand side: quoting it would turn
    // a comparison between two facets into a comparison against the literal
    // `b`, silently changing what the scrubbed model means
    // (configflux-secb.2 / ADR-0057 §D5).
    for src in [
        "a == b",
        "a != b",
        "!(a == b) || c == 'x'",
        "any_of(a == b, c != d)",
    ] {
        let out = rewrite_condition_identifiers(src, &id_tag, &id_lit).unwrap();
        assert_eq!(ast(src), ast(&out), "round-trip lost for {src:?} -> {out:?}");
    }
}

#[test]
fn facet_comparison_pseudonymizes_both_operands() {
    // Both sides are facet identifiers. Leaving the right-hand one alone would
    // publish a real facet name out of a scrubbed model (ADR-0034 D3).
    let ids = condition_identifiers("sorter_container == line_container").unwrap();
    assert_eq!(
        ids.tags,
        ["line_container", "sorter_container"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    assert!(
        ids.options.is_empty(),
        "a facet comparison names no option literal, got {:?}",
        ids.options
    );

    let renamed = rewrite_condition_identifiers(
        "sorter_container == line_container",
        &|tag| format!("tag_{tag}"),
        &id_lit,
    )
    .unwrap();
    assert_eq!(renamed, "tag_sorter_container == tag_line_container");
}

#[test]
fn double_negation_and_nested_parens_preserved() {
    // Round-trip a condition whose AST shape is sensitive to precedence to
    // guard against the serializer dropping a needed paren.
    for src in [
        "!(a == 'x' && b == 'y')",
        "!(a == 'x' || b == 'y') && c == 'z'",
        "a == 'x' || b == 'y' && c == 'z'", // && binds tighter than ||
    ] {
        let out = rewrite_condition_identifiers(src, &id_tag, &id_lit).unwrap();
        assert_eq!(ast(src), ast(&out), "precedence lost for {src:?} -> {out:?}");
    }
}
