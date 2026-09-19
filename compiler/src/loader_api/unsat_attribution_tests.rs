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

/// The asserted-only baseline: no closed-facet domains at all. This is exactly
/// what the runtime surface has — `RuntimeSnapshot` carries no `ModelHandle`,
/// so it can reach no facet declarations — and every test below that predates
/// closed-facet entailment (configflux-pt6v) pins the behavior under it.
fn no_domains() -> ClosedFacetDomains {
    ClosedFacetDomains::default()
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
        &no_domains(),
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
        &no_domains(),
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
        &no_domains(),
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
            attribute_core_clauses(
                &[prior_choice_clause(), clause],
                &cf("environment", "dev"),
                &hero_roster(),
                &no_domains(),
            );
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
        &no_domains(),
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
    let out = attribute_core_clauses(
        &[hero_policy_clause()],
        &cf("log_level", "debug"),
        &roster,
        &no_domains(),
    );
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
    let out = attribute_core_clauses(&[clause], &cf("psu", "bronze"), &roster, &no_domains());
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
    let out = attribute_core_clauses(
        &[clause],
        &cf("replica_class", "scaled"),
        &hero_roster(),
        &no_domains(),
    );
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
    let out = attribute_core_clauses(&[clause], &cf("environment", "prod"), &roster, &no_domains());
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
    let out = attribute_core_clauses(
        &[hero_policy_clause()],
        &cf("log_level", "debug"),
        &roster,
        &no_domains(),
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].constraint_id, None);
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}

#[test]
fn a_core_with_no_model_clauses_reports_no_model_rule_at_all() {
    let out =
        attribute_core_clauses(
            &[prior_choice_clause()],
            &cf("log_level", "debug"),
            &hero_roster(),
            &no_domains(),
        );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, ConstraintKind::Selection);
}

#[test]
fn an_empty_roster_can_never_name_a_constraint() {
    let out = attribute_core_clauses(
        &[hero_policy_clause()],
        &cf("log_level", "debug"),
        &[],
        &no_domains(),
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].constraint_id, None);
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}

// ---------------------------------------------------------------------------
// configflux-pt6v: closed-facet domain entailment.
//
// A facet that appears in a core clause ONLY negatively has no asserted
// literal, so the asserted-only collection below leaves it unassigned and the
// declared constraint over it evaluates `Unknown` — not a violation (ADR-0054
// §2) — and the core degrades to MODEL_OVER_CONSTRAINED_SUMMARY. When the
// facet is CLOSED, the model itself carries `exactly_one_of` over its declared
// values, so "all declared values but one negated" ENTAILS the remaining one.
// Completing the assignment that way makes the constraint evaluate genuinely
// `False`, which preserves §2 rather than weakening it.
//
// The pins below are the regression cases (a)-(d) from the issue, expressed
// over the same shapes the end-to-end repro uses.
// ---------------------------------------------------------------------------

/// The pt6v repro's roster: the cross-brand exclusion the user plainly broke.
fn cooling_roster() -> Vec<DeclaredConstraint> {
    vec![DeclaredConstraint {
        id: "aeroflux_excludes_x200".to_string(),
        condition: "cooling_brand != 'aeroflux' || cooling_model != 'x200'".to_string(),
        root_index: 0,
    }]
}

/// The clause the repro's BDD actually produces: `cooling_brand` asserted,
/// and `cooling_model` present ONLY as the negated `!a9`. Under the closed
/// two-value domain `{a9, x200}` this forbids `{aeroflux, x200}`.
fn cooling_negative_only_clause() -> CoreClause {
    CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![
            cf("cooling_brand", "aeroflux"),
            cf("cooling_brand", "hydra"),
            cf("cooling_model", "a9"),
        ],
        summary: "relating cooling_brand.aeroflux, cooling_brand.hydra, cooling_model.a9"
            .to_string(),
        forbidden: vec![
            (cf("cooling_brand", "aeroflux"), true),
            (cf("cooling_brand", "hydra"), false),
            (cf("cooling_model", "a9"), false),
        ],
    }
}

/// Build a closed-domain map from `(facet, values)` pairs. The values are
/// taken in the given order deliberately: the map must not depend on it.
fn closed(entries: &[(&str, &[&str])]) -> ClosedFacetDomains {
    let mut domains = ClosedFacetDomains::default();
    for (facet, values) in entries {
        domains.insert(
            (*facet).to_string(),
            values.iter().map(|value| (*value).to_string()).collect(),
        );
    }
    domains
}

/// The single named constraint id in an attribution result, if any.
fn named_ids(out: &[ConflictingConstraint]) -> Vec<&str> {
    out.iter().filter_map(|c| c.constraint_id.as_deref()).collect()
}

#[test]
fn a_negative_only_closed_facet_is_completed_by_domain_entailment() {
    // REGRESSION PIN (a). `cooling_model` appears only as `!a9`; the facet is
    // closed over {a9, x200}, so x200 is entailed and the exclusion is broken.
    // TODAY this reports MODEL_OVER_CONSTRAINED_SUMMARY.
    let out = attribute_core_clauses(
        &[cooling_negative_only_clause()],
        &cf("cooling_model", "x200"),
        &cooling_roster(),
        &closed(&[
            ("cooling_brand", &["aeroflux", "hydra"]),
            ("cooling_model", &["a9", "x200"]),
        ]),
    );
    assert_eq!(
        named_ids(&out),
        vec!["aeroflux_excludes_x200"],
        "a closed facet's negated-value entailment must name the broken policy: {out:?}"
    );
    let named = out
        .iter()
        .find(|c| c.constraint_id.is_some())
        .expect("a named constraint");
    assert_eq!(
        named.facets,
        vec![cf("cooling_brand", "aeroflux"), cf("cooling_model", "x200")],
        "the entailed value is what the user broke, and must be reported: {named:?}"
    );
    assert_eq!(
        named.summary,
        "cooling_brand != 'aeroflux' || cooling_model != 'x200'"
    );
}

#[test]
fn domain_entailment_does_not_depend_on_declared_value_order() {
    // REGRESSION PIN (b), folding in configflux-gpwf: the committed goldens
    // must not pass on alphabetical luck. Declaring the SAME closed domain in
    // the reverse order must attribute identically.
    let forward = attribute_core_clauses(
        &[cooling_negative_only_clause()],
        &cf("cooling_model", "x200"),
        &cooling_roster(),
        &closed(&[
            ("cooling_brand", &["aeroflux", "hydra"]),
            ("cooling_model", &["a9", "x200"]),
        ]),
    );
    let reversed = attribute_core_clauses(
        &[cooling_negative_only_clause()],
        &cf("cooling_model", "x200"),
        &cooling_roster(),
        &closed(&[
            ("cooling_brand", &["hydra", "aeroflux"]),
            ("cooling_model", &["x200", "a9"]),
        ]),
    );
    assert_eq!(
        named_ids(&reversed),
        vec!["aeroflux_excludes_x200"],
        "declared value order must not decide whether a policy is named: {reversed:?}"
    );
    assert_eq!(
        forward, reversed,
        "attribution must be identical under either declared value order"
    );
}

#[test]
fn an_open_facet_is_never_completed_by_entailment() {
    // REGRESSION PIN (c). An OPEN facet carries at-most-one only — never
    // at-least-one — so `!a9` entails nothing and `Unknown` stands (ADR-0054
    // §2). An open facet is absent from the closed-domain map by construction.
    let out = attribute_core_clauses(
        &[cooling_negative_only_clause()],
        &cf("cooling_model", "x200"),
        &cooling_roster(),
        &closed(&[("cooling_brand", &["aeroflux", "hydra"])]),
    );
    assert!(
        named_ids(&out).is_empty(),
        "an open facet's negated value entails nothing: {out:?}"
    );
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}

#[test]
fn a_three_valued_closed_facet_completes_when_one_value_is_left() {
    // REGRESSION PIN (d), the hero-example shape: `environment` closed over
    // three values, two of them negated, leaves exactly `prod`.
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![
            cf("environment", "dev"),
            cf("environment", "staging"),
            cf("log_level", "debug"),
        ],
        summary: String::new(),
        forbidden: vec![
            (cf("environment", "dev"), false),
            (cf("environment", "staging"), false),
            (cf("log_level", "debug"), true),
        ],
    };
    let out = attribute_core_clauses(
        &[clause],
        &cf("log_level", "debug"),
        &hero_roster(),
        &closed(&[
            ("environment", &["dev", "prod", "staging"]),
            ("log_level", &["debug", "info"]),
        ]),
    );
    assert_eq!(
        named_ids(&out),
        vec!["prod_forbids_debug"],
        "two of three closed values negated entails the third: {out:?}"
    );
    let named = &out[0];
    assert_eq!(
        named.facets,
        vec![cf("environment", "prod"), cf("log_level", "debug")],
        "the entailed `environment.prod` is part of what the user broke: {named:?}"
    );
}

#[test]
fn two_un_negated_closed_values_leave_the_facet_unassigned() {
    // The entailment is exactly-one-of: with `dev` negated, BOTH `prod` and
    // `staging` remain possible, so nothing is entailed and `Unknown` stands.
    // Asserting `prod` here would be FALSE attribution — naming a policy the
    // user may not have broken at all.
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![cf("environment", "dev"), cf("log_level", "debug")],
        summary: String::new(),
        forbidden: vec![
            (cf("environment", "dev"), false),
            (cf("log_level", "debug"), true),
        ],
    };
    let out = attribute_core_clauses(
        &[clause],
        &cf("log_level", "debug"),
        &hero_roster(),
        &closed(&[
            ("environment", &["dev", "prod", "staging"]),
            ("log_level", &["debug", "info"]),
        ]),
    );
    assert!(
        named_ids(&out).is_empty(),
        "two remaining values entail nothing: {out:?}"
    );
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}

#[test]
fn a_facet_absent_from_the_clause_is_never_completed() {
    // Completion reads the clause, never the domain map alone. A closed facet
    // the clause does not mention has no negated values, so "all but one
    // negated" is false for it and it must stay unassigned — inventing a value
    // here would attribute a policy to a clause that never touched it.
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![cf("environment", "prod")],
        summary: String::new(),
        forbidden: vec![(cf("environment", "prod"), true)],
    };
    let out = attribute_core_clauses(
        &[clause],
        &cf("environment", "prod"),
        &hero_roster(),
        // `log_level` is closed and two-valued, but the clause never mentions
        // it. A single-value domain would be the strongest temptation to
        // "complete" it, so pin that shape too.
        &closed(&[
            ("environment", &["dev", "prod", "staging"]),
            ("log_level", &["debug"]),
        ]),
    );
    assert!(
        named_ids(&out).is_empty(),
        "an unmentioned facet must never be assigned: {out:?}"
    );
}

#[test]
fn genuine_cardinality_stays_over_constrained_under_real_closed_domains() {
    // REGRESSION PIN (e), strengthened: the §5.4 hard rule must hold with the
    // REAL closed domains in hand, not merely because the old code was blind
    // to them. Both synthesized shapes, over a fully declared closed model.
    //
    // The at-least-one clause is also the "every declared value negated" case:
    // ZERO values remain, so nothing is entailed and the completion must not
    // fire. The at-most-one clause asserts two values of one facet, which is
    // rejected before completion is even reached.
    let domains = closed(&[
        ("environment", &["dev", "prod", "staging"]),
        ("log_level", &["debug", "info"]),
    ]);
    for clause in [
        cardinality_at_most_one_clause(),
        cardinality_at_least_one_clause(),
    ] {
        let out = attribute_core_clauses(
            &[prior_choice_clause(), clause],
            &cf("environment", "dev"),
            &hero_roster(),
            &domains,
        );
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
fn empty_domains_reproduce_the_asserted_only_behavior_exactly() {
    // The runtime surface has no `ModelHandle` and therefore no domains
    // (`RuntimeSnapshot` carries none). An empty map MUST leave attribution
    // byte-identical to the asserted-only behavior, so the runtime and
    // `session_compose` agree on every core that has no negative-only closed
    // facet.
    let empty = ClosedFacetDomains::default();
    let out = attribute_core_clauses(
        &[cooling_negative_only_clause()],
        &cf("cooling_model", "x200"),
        &cooling_roster(),
        &empty,
    );
    assert!(
        named_ids(&out).is_empty(),
        "no domains means no entailment: {out:?}"
    );
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);

    // And the asserted-only path is untouched: the hero clause still names its
    // constraint with no domains at all.
    let hero = attribute_core_clauses(
        &[prior_choice_clause(), hero_policy_clause()],
        &cf("log_level", "debug"),
        &hero_roster(),
        &empty,
    );
    assert_eq!(named_ids(&hero), vec!["prod_forbids_debug"]);
}

// ---------------------------------------------------------------------------
// configflux-vfh5: a clause's NEGATIVE literals are part of what it forbids.
//
// pt6v taught the completion above to read a negated literal where the closed
// domain turns it into a positive statement — "every declared value but one
// negated" entails the remaining one. That covers a two-value facet, and stops
// at three: rule ONE value out of three and two remain, so nothing is entailed,
// the facet stays unassigned, and a constraint over it evaluates `Unknown`.
//
// A facet-to-facet equality (ADR-0057 §D5) is where that stops being an
// under-attribution and starts being a wrong answer. `a == b` contributes
// clauses that assert one side and rule ONE value out of the other, which is
// exactly the shape the completion cannot finish — so the one rule that made a
// selection impossible went unnamed while the mirror-image clause of the SAME
// constraint was named. The tests below pin the rule that closed it: a facet is
// carried as the SET of values the clause leaves it, and a constraint is
// violated when no value in those sets can satisfy it.
//
// The §5.4 hard rule is unmoved and the pins for it are above: two remaining
// values still entail nothing, and a synthesized cardinality clause is still
// named by nothing. The cases below add the boundary in the other direction.
// ---------------------------------------------------------------------------

/// The issue's roster: one authored facet-to-facet equality.
fn equality_roster() -> Vec<DeclaredConstraint> {
    vec![DeclaredConstraint {
        id: "sorter_matches_line".to_string(),
        condition: "sorter_container == line_container".to_string(),
        root_index: 0,
    }]
}

/// The three-entry catalogue both bindings draw from, plus the site.
fn container_domains() -> ClosedFacetDomains {
    closed(&[
        ("line_container", &["c1", "c2", "c3"]),
        ("site", &["factory_a", "factory_b"]),
        ("sorter_container", &["c1", "c2", "c3"]),
    ])
}

/// The clause the equality actually contributes: `line_container` asserted at
/// `c1`, `sorter_container` present ONLY as `!c1`. Two of its three values
/// remain, so no single value is entailed — and yet no value it can take
/// equals `c1`, so the equality cannot hold.
fn equality_clause() -> CoreClause {
    CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![
            cf("line_container", "c1"),
            cf("site", "factory_a"),
            cf("sorter_container", "c1"),
        ],
        summary: String::new(),
        forbidden: vec![
            (cf("line_container", "c1"), true),
            (cf("line_container", "c2"), false),
            (cf("line_container", "c3"), false),
            (cf("site", "factory_a"), true),
            (cf("site", "factory_b"), false),
            (cf("sorter_container", "c1"), false),
        ],
    }
}

#[test]
fn an_equality_is_violated_when_no_value_left_to_one_side_is_open_to_the_other() {
    // The defect, at the rule. `line_container` is pinned to `c1` and
    // `sorter_container` may be `c2` or `c3` — disjoint, so every configuration
    // this clause forbids breaks the equality. Naming it is a deduction, not a
    // preference: there is no completion of the clause under which it holds.
    let out = attribute_core_clauses(
        &[equality_clause()],
        &cf("sorter_container", "c3"),
        &equality_roster(),
        &container_domains(),
    );
    assert_eq!(
        named_ids(&out),
        vec!["sorter_matches_line"],
        "an equality whose two sides share no possible value is broken: {out:?}"
    );
}

#[test]
fn an_equality_with_a_shared_possible_value_is_never_named() {
    // The boundary. Rule `c3` out of the sorter and `c1`/`c2` remain — and the
    // line may still be `c1` or `c2` as well, so a configuration this clause
    // forbids can satisfy the equality. `Unknown` is not a violation
    // (ADR-0054 §2), and naming the constraint here would blame a policy the
    // selection may not have broken at all.
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![cf("line_container", "c3"), cf("sorter_container", "c3")],
        summary: String::new(),
        forbidden: vec![
            (cf("line_container", "c3"), false),
            (cf("sorter_container", "c3"), false),
        ],
    };
    let out = attribute_core_clauses(
        &[clause],
        &cf("sorter_container", "c3"),
        &equality_roster(),
        &container_domains(),
    );
    assert!(
        named_ids(&out).is_empty(),
        "the two sides can still agree, so nothing is violated: {out:?}"
    );
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}

#[test]
fn a_value_predicate_is_violated_by_the_value_being_ruled_out() {
    // The same rule under the ordinary predicate form, which is what makes it
    // one rule rather than a special case for equality. `accepts` lowers to
    // `any_of(line == 'c1', line == 'c2')`, and the catalogue here holds FOUR
    // entries so that ruling both accepted ones out still leaves two standing.
    // The pt6v completion cannot finish that — no single value is entailed —
    // yet neither remaining value is accepted, so the requirement is broken.
    //
    // The contrast clause rules out one accepted entry and one rejected one,
    // leaving an accepted value open. The requirement can still hold, so it
    // must not be named.
    let roster = vec![DeclaredConstraint {
        id: "accepts:compute_service.container".to_string(),
        condition: "any_of(line_container == 'c1', line_container == 'c2')".to_string(),
        root_index: 0,
    }];
    let domains = closed(&[("line_container", &["c1", "c2", "c3", "c4"])]);
    let ruled_out = |values: &[&str]| CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: values.iter().map(|v| cf("line_container", v)).collect(),
        summary: String::new(),
        forbidden: values
            .iter()
            .map(|v| (cf("line_container", v), false))
            .collect(),
    };

    let broken = attribute_core_clauses(
        &[ruled_out(&["c1", "c2"])],
        &cf("line_container", "c3"),
        &roster,
        &domains,
    );
    assert_eq!(
        named_ids(&broken),
        vec!["accepts:compute_service.container"],
        "only c3 and c4 are left, neither of which the requirement accepts: {broken:?}"
    );

    let satisfied = attribute_core_clauses(
        &[ruled_out(&["c1", "c3"])],
        &cf("line_container", "c3"),
        &roster,
        &domains,
    );
    assert!(
        named_ids(&satisfied).is_empty(),
        "c2 is left, which the requirement accepts: {satisfied:?}"
    );
}

#[test]
fn a_clause_that_rules_out_every_value_of_a_facet_names_nothing_through_it() {
    // §5.4, restated against the widening. A synthesized at-least-one clause
    // rules out EVERY declared value, which under a set-valued reading leaves
    // the facet with no possible value at all — and a facet that can be nothing
    // trivially satisfies no predicate, so a naive widening would name the
    // requirement here. It must not: the clause is the model's own
    // bookkeeping, contradictory on its own, and nothing the author declared
    // forbids it.
    let roster = vec![DeclaredConstraint {
        id: "accepts:compute_service.container".to_string(),
        condition: "any_of(line_container == 'c1', line_container == 'c2')".to_string(),
        root_index: 0,
    }];
    let clause = CoreClause {
        kind: ConstraintKind::ModelRule,
        facets: vec![
            cf("line_container", "c1"),
            cf("line_container", "c2"),
            cf("line_container", "c3"),
        ],
        summary: String::new(),
        forbidden: vec![
            (cf("line_container", "c1"), false),
            (cf("line_container", "c2"), false),
            (cf("line_container", "c3"), false),
        ],
    };
    let out = attribute_core_clauses(
        &[clause],
        &cf("line_container", "c3"),
        &roster,
        &container_domains(),
    );
    assert!(
        named_ids(&out).is_empty(),
        "a cardinality clause must never be named as declared policy: {out:?}"
    );
    assert_eq!(out[0].summary, MODEL_OVER_CONSTRAINED_SUMMARY);
}

#[test]
fn negative_literals_are_read_only_where_the_domain_is_declared_closed() {
    // The same guard pt6v's completion carries, and for the same reason: an
    // OPEN facet is synthesized with at-most-one only, so ruling a value out
    // says nothing about what is left — the facet may still take a value the
    // model never declared. Without the closed declaration in hand there is no
    // set to reason over, and the constraint must stay `Unknown`.
    let out = attribute_core_clauses(
        &[equality_clause()],
        &cf("sorter_container", "c3"),
        &equality_roster(),
        // `sorter_container` deliberately absent: an open facet is never
        // recorded here, which is what makes the guard a property of the type.
        &closed(&[("line_container", &["c1", "c2", "c3"])]),
    );
    assert!(
        named_ids(&out).is_empty(),
        "an undeclared or open facet leaves the equality unknown: {out:?}"
    );
}
