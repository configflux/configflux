// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the ADR-0054 §5.4 unsat-core constraint attribution
//! (`unsat_attribution.rs`, configflux-p571.8).
//!
//! A sibling file rather than an inline `mod` because the suite is
//! self-contained: it pins one pure function's rules and needs none of the
//! compile/open/resolve fixtures `loader_api/tests.rs` is built around. It
//! follows the `conditions/rewrite_tests.rs` convention this crate already
//! uses for exactly that shape.
//!
//! The hard rule under test is §5.4's: synthesized cardinality conjuncts are
//! not in the roster and must never be named in a user-facing core. Several
//! tests below deliberately hand the function a roster whose constraint LOOKS
//! applicable, and assert it is not reached for.

use super::*;

fn cf(facet: &str, option: &str) -> ConstraintFacet {
    ConstraintFacet {
        facet: facet.to_string(),
        option: option.to_string(),
    }
}

/// The hero example's roster: one declared constraint at root index 0.
fn hero_roster() -> Vec<DeclaredConstraint> {
    vec![DeclaredConstraint {
        id: "prod_forbids_debug".to_string(),
        condition: "environment != 'prod' || log_level != 'debug'".to_string(),
        root_index: 0,
    }]
}

/// The clause the hero example's BDD actually produces: the two atoms that
/// matter plus a `beta_dashboard` variable the falsifying path branched on.
fn hero_policy_clause() -> CoreClause {
    CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![
            cf("beta_dashboard", "off"),
            cf("environment", "prod"),
            cf("log_level", "debug"),
        ],
        summary: "relating beta_dashboard.off, environment.prod, log_level.debug".to_string(),
        forbidden: vec![
            (cf("beta_dashboard", "off"), true),
            (cf("environment", "prod"), true),
            (cf("log_level", "debug"), true),
        ],
    }
}

/// A synthesized at-most-one clause: two values of one facet asserted.
fn cardinality_at_most_one_clause() -> CoreClause {
    CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![cf("environment", "dev"), cf("environment", "prod")],
        summary: "relating environment.dev, environment.prod".to_string(),
        forbidden: vec![
            (cf("environment", "dev"), true),
            (cf("environment", "prod"), true),
        ],
    }
}

/// A synthesized at-least-one clause: every value of one facet ruled out.
fn cardinality_at_least_one_clause() -> CoreClause {
    CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![
            cf("environment", "dev"),
            cf("environment", "prod"),
            cf("environment", "staging"),
        ],
        summary: "relating environment.dev, environment.prod, environment.staging".to_string(),
        forbidden: vec![
            (cf("environment", "dev"), false),
            (cf("environment", "prod"), false),
            (cf("environment", "staging"), false),
        ],
    }
}

fn prior_choice_clause() -> CoreClause {
    CoreClause {
        kind: ConstraintKind::Selection,
        facets: vec![cf("environment", "prod")],
        summary: "blocked by your earlier choice: environment.prod".to_string(),
        forbidden: vec![(cf("environment", "prod"), true)],
    }
}

#[test]
fn attribution_names_the_declared_constraint_the_clause_violates() {
    // The acceptance criterion: the hero example's core must name
    // `prod_forbids_debug` — not a component, not a synthetic name, and not
    // the raw variables the falsifying path carried.
    let out = attribute_core_clauses(
        &[prior_choice_clause(), hero_policy_clause()],
        &cf("log_level", "debug"),
        &hero_roster(),
    );
    let named: Vec<&ConflictingConstraint> = out
        .iter()
        .filter(|c| c.kind == ConstraintKind::ModelRule)
        .collect();
    assert_eq!(named.len(), 1, "one model clause, one named constraint: {out:?}");
    assert_eq!(named[0].constraint_id.as_deref(), Some("prod_forbids_debug"));
    assert_eq!(named[0].summary, "environment != 'prod' || log_level != 'debug'");
}

#[test]
fn attribution_names_only_the_facets_the_constraint_mentions() {
    // The `beta_dashboard` atom is a variable the BDD path branched on, not
    // part of what the user violated. It must not appear in the named entry.
    let out = attribute_core_clauses(
        &[prior_choice_clause(), hero_policy_clause()],
        &cf("log_level", "debug"),
        &hero_roster(),
    );
    let named = out
        .iter()
        .find(|c| c.constraint_id.is_some())
        .expect("a named constraint");
    assert_eq!(
        named.facets,
        vec![cf("environment", "prod"), cf("log_level", "debug")],
        "only the constraint's own facets may be named: {named:?}"
    );
}

#[test]
fn prior_selections_pass_through_verbatim_and_are_never_attributed() {
    let out = attribute_core_clauses(
        &[prior_choice_clause(), hero_policy_clause()],
        &cf("log_level", "debug"),
        &hero_roster(),
    );
    let selection = &out[0];
    assert_eq!(selection.kind, ConstraintKind::Selection);
    assert_eq!(selection.facets, vec![cf("environment", "prod")]);
    assert_eq!(
        selection.summary,
        "blocked by your earlier choice: environment.prod"
    );
    assert_eq!(
        selection.constraint_id, None,
        "a prior choice is not a declared constraint"
    );
}

#[test]
fn a_core_reducing_to_cardinality_is_reported_over_constrained_not_named() {
    // ADR-0054 §5.4 HARD RULE. `prod_forbids_debug` is in the roster and its
    // facet is the one in conflict, so a sloppy implementation would reach for
    // it. It must not: nothing the user declared forbids this.
    for clause in [
        cardinality_at_most_one_clause(),
        cardinality_at_least_one_clause(),
    ] {
        let out =
            attribute_core_clauses(&[prior_choice_clause(), clause], &cf("environment", "dev"), &hero_roster());
        let model: Vec<&ConflictingConstraint> = out
            .iter()
            .filter(|c| c.kind == ConstraintKind::ModelRule)
            .collect();
        assert_eq!(model.len(), 1, "one over-constrained report: {out:?}");
        assert_eq!(
            model[0].constraint_id, None,
            "a cardinality clause must never be named as declared policy: {out:?}"
        );
        assert_eq!(model[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
    }
}

#[test]
fn cardinality_noise_is_dropped_once_a_declared_constraint_is_named() {
    // The real hero core carries three cardinality clauses alongside the one
    // policy clause. Naming the policy is the answer; the bookkeeping clauses
    // must not be surfaced as extra "model rule" lines.
    let out = attribute_core_clauses(
        &[
            prior_choice_clause(),
            cardinality_at_most_one_clause(),
            cardinality_at_least_one_clause(),
            hero_policy_clause(),
        ],
        &cf("log_level", "debug"),
        &hero_roster(),
    );
    assert_eq!(
        out.len(),
        2,
        "one prior choice + one named constraint, nothing else: {out:?}"
    );
    assert_eq!(out[1].constraint_id.as_deref(), Some("prod_forbids_debug"));
}

#[test]
fn multiple_violated_constraints_are_named_in_root_index_order() {
    // Declared out of order to prove the report is sorted by `root_index`,
    // not by roster position or id.
    let roster = vec![
        DeclaredConstraint {
            id: "second".to_string(),
            condition: "environment != 'prod' || log_level != 'debug'".to_string(),
            root_index: 1,
        },
        DeclaredConstraint {
            id: "first".to_string(),
            condition: "log_level != 'debug'".to_string(),
            root_index: 0,
        },
    ];
    let out = attribute_core_clauses(&[hero_policy_clause()], &cf("log_level", "debug"), &roster);
    let ids: Vec<&str> = out
        .iter()
        .filter_map(|c| c.constraint_id.as_deref())
        .collect();
    assert_eq!(
        ids,
        vec!["first", "second"],
        "report order is the model's declaration order (root_index)"
    );
}

#[test]
fn a_constraint_the_candidate_does_not_break_is_not_named() {
    // A BDD falsifying path carries variables the walk branched on, not just
    // the ones that matter. On `s_labeled_mus`, explaining `psu.bronze`
    // produced a clause whose path also asserted `cooling.air` — which
    // violates the unrelated cooling rule. Only the constraint the CANDIDATE
    // breaks may be named.
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![cf("cooling", "air"), cf("cpu", "highperf"), cf("psu", "bronze")],
        summary: String::new(),
        forbidden: vec![
            (cf("cooling", "air"), true),
            (cf("cpu", "highperf"), true),
            (cf("psu", "bronze"), true),
        ],
    };
    let roster = vec![
        DeclaredConstraint {
            id: "highperf_requires_gold".to_string(),
            condition: "cpu != 'highperf' || psu != 'bronze'".to_string(),
            root_index: 1,
        },
        DeclaredConstraint {
            id: "highperf_requires_liquid".to_string(),
            condition: "cpu != 'highperf' || cooling != 'air'".to_string(),
            root_index: 2,
        },
    ];
    let out = attribute_core_clauses(&[clause], &cf("psu", "bronze"), &roster);
    let ids: Vec<&str> = out
        .iter()
        .filter_map(|c| c.constraint_id.as_deref())
        .collect();
    assert_eq!(
        ids,
        vec!["highperf_requires_gold"],
        "only the constraint the candidate breaks is named: {out:?}"
    );
}

#[test]
fn a_violation_predating_the_candidate_is_still_named_when_nothing_else_is() {
    // Candidate relevance is a preference, not a hard filter: when NO
    // constraint clears that bar, the plain violation set is still reported
    // rather than degrading a real explanation to "over-constrained".
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![cf("environment", "prod"), cf("log_level", "debug")],
        summary: String::new(),
        forbidden: vec![
            (cf("environment", "prod"), true),
            (cf("log_level", "debug"), true),
        ],
    };
    // The candidate is a facet the constraint never mentions, so withdrawing
    // it changes nothing.
    let out = attribute_core_clauses(&[clause], &cf("replica_class", "scaled"), &hero_roster());
    assert_eq!(
        out.iter()
            .filter_map(|c| c.constraint_id.as_deref())
            .collect::<Vec<_>>(),
        vec!["prod_forbids_debug"],
        "a real violation must not be downgraded to over-constrained: {out:?}"
    );
}

#[test]
fn an_unsatisfied_constraint_is_not_a_violated_one() {
    // ADR-0054 §2: `Unknown` (a constraint referencing an unbound facet) is
    // not a violation — nothing was chosen, so nothing was violated.
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![cf("environment", "prod")],
        summary: String::new(),
        forbidden: vec![(cf("environment", "prod"), true)],
    };
    let roster = vec![DeclaredConstraint {
        id: "needs_both".to_string(),
        condition: "environment != 'prod' || log_level != 'debug'".to_string(),
        root_index: 0,
    }];
    let out = attribute_core_clauses(&[clause], &cf("environment", "prod"), &roster);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].constraint_id, None,
        "log_level is unbound, so the constraint is Unknown, not violated: {out:?}"
    );
}

#[test]
fn an_unparseable_roster_condition_is_never_named() {
    // Identity fails closed: a condition the evaluator cannot check must not
    // be reported as the thing the user broke.
    let roster = vec![DeclaredConstraint {
        id: "garbage".to_string(),
        condition: "environment ?? 'prod'".to_string(),
        root_index: 0,
    }];
    let out = attribute_core_clauses(&[hero_policy_clause()], &cf("log_level", "debug"), &roster);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].constraint_id, None);
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}

#[test]
fn a_core_with_no_model_clauses_reports_no_model_rule_at_all() {
    let out =
        attribute_core_clauses(&[prior_choice_clause()], &cf("log_level", "debug"), &hero_roster());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, ConstraintKind::Selection);
}

#[test]
fn an_empty_roster_can_never_name_a_constraint() {
    let out = attribute_core_clauses(&[hero_policy_clause()], &cf("log_level", "debug"), &[]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].constraint_id, None);
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}
