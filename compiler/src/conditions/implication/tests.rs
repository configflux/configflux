// SPDX-License-Identifier: BUSL-1.1

//! Unit and parity tests for the exhaustive-implication evaluator
//! (configflux-uiyo).
//!
//! Split out of `implication.rs` to keep that file under the repo's per-file
//! line limit (`tools/lint_repo.py`). Declared from `implication.rs` via
//! `#[cfg(test)] mod tests;`, so `super::` here resolves to the `implication`
//! module and reaches its private engine helpers (`collect_atom_values`,
//! `for_each_predicate`, `condition_expr_implies`) directly, and
//! `super::super::` reaches the parent `conditions` module for the retained
//! string evaluator (`eval_condition`) and the AST parser
//! (`parse_condition_expr`).

use super::super::{eval_condition, parse_condition_expr};
use super::*;
use std::collections::HashMap;

fn parse(src: &str) -> ConditionExpr {
    parse_condition_expr(src).expect("fixture must parse")
}

/// Independent reference implementation of the SOUND exhaustive-implication
/// matrix that the production code carried before configflux-uiyo
/// (`eval_implication_by_matrix`). It is rebuilt here, in test code, on the
/// retained string evaluator (`eval_condition`) so the parity guard for
/// `condition_expr_implies` survives the deletion of the production matrix: the
/// typed evaluator must keep matching this oracle forever, not just at the
/// migration moment.
///
/// Mirrors the original engine exactly: collect every atom's tag/value out of
/// both condition strings, extend each tag's domain with one fresh sentinel,
/// enumerate the full product of those domains, and check `!(a && !b)` with the
/// strict string evaluator over each total assignment. Restricted (like the
/// original) to the pre-grammar-v2 subset the string evaluator accepts —
/// predicates, `&&`, `||`, `!`, parens, and boolean literals — since that
/// evaluator errors on cardinality operators.
fn matrix_oracle(a: &str, b: &str) -> bool {
    let mut tag_values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for cond in [a, b] {
        collect_atom_values(&parse(cond), &mut tag_values);
    }
    let tags: Vec<String> = tag_values.keys().cloned().collect();
    let domains: Vec<Vec<String>> = tags
        .iter()
        .map(|tag| {
            let mut domain: Vec<String> = tag_values[tag].iter().cloned().collect();
            let sentinel = if domain.iter().any(|v| v == "__other__") {
                "__other__sentinel".to_string()
            } else {
                "__other__".to_string()
            };
            domain.push(sentinel);
            domain
        })
        .collect();

    // Enumerate the Cartesian product via a mixed-radix counter.
    let total: usize = domains.iter().map(|d| d.len()).product::<usize>().max(1);
    for n in 0..total {
        let mut rem = n;
        let mut env: HashMap<String, String> = HashMap::new();
        for (tag, domain) in tags.iter().zip(domains.iter()) {
            let pick = rem % domain.len();
            rem /= domain.len();
            env.insert(tag.clone(), domain[pick].clone());
        }
        let a_val = eval_condition(a, &env).expect("oracle: antecedent must evaluate");
        let b_val = eval_condition(b, &env).expect("oracle: consequent must evaluate");
        if a_val && !b_val {
            return false;
        }
    }
    true
}

// ---- direct truth-table unit tests --------------------------------------

#[test]
fn implies_conjunction_subset_is_true() {
    // `a && b` implies `a` (every model of the stronger condition models the
    // weaker one).
    assert!(condition_expr_implies(
        &parse("variant == 'heavy' && region == 'us'"),
        &parse("variant == 'heavy'"),
    ));
}

#[test]
fn implies_unrelated_atoms_is_false() {
    assert!(!condition_expr_implies(
        &parse("variant == 'heavy'"),
        &parse("region == 'us'"),
    ));
}

#[test]
fn implies_true_consequent_is_always_true() {
    // Anything implies a tautology.
    assert!(condition_expr_implies(
        &parse("variant == 'heavy'"),
        &parse("true"),
    ));
}

#[test]
fn implies_false_antecedent_is_always_true() {
    // A contradiction implies anything (no model to falsify the implication).
    assert!(condition_expr_implies(
        &parse("false"),
        &parse("region == 'eu'"),
    ));
}

#[test]
fn implies_disjunction_does_not_imply_single_disjunct() {
    // `a || b` does NOT imply `a`: the model `{a=other, b=y}` satisfies the
    // antecedent but not the consequent.
    assert!(!condition_expr_implies(
        &parse("variant == 'heavy' || region == 'us'"),
        &parse("variant == 'heavy'"),
    ));
}

#[test]
fn implies_disjunction_is_implied_by_each_disjunct() {
    assert!(condition_expr_implies(
        &parse("variant == 'heavy'"),
        &parse("variant == 'heavy' || region == 'us'"),
    ));
}

#[test]
fn implies_negation_subsumption() {
    // `region != 'eu'` implies `!(region == 'eu')` — they are equivalent.
    assert!(condition_expr_implies(
        &parse("region != 'eu'"),
        &parse("!(region == 'eu')"),
    ));
    assert!(condition_expr_implies(
        &parse("!(region == 'eu')"),
        &parse("region != 'eu'"),
    ));
}

#[test]
fn implies_cardinality_exactly_one_of() {
    // `exactly_one_of` implies `any_of` (at-least-one is a weaker claim); the
    // reverse fails because `{a=x, b=y}` satisfies `any_of` but breaks the
    // at-most-one half of `exactly_one_of`.
    assert!(condition_expr_implies(
        &parse("exactly_one_of(a == 'x', b == 'y')"),
        &parse("any_of(a == 'x', b == 'y')"),
    ));
    assert!(!condition_expr_implies(
        &parse("any_of(a == 'x', b == 'y')"),
        &parse("exactly_one_of(a == 'x', b == 'y')"),
    ));
}

// ---- PARITY: typed evaluator == exhaustive matrix oracle (uiyo) ---------
//
// The migration of `link_verify` off the string-scanning matrix engine must be
// byte-stable. The former `condition_implies` ran a logically-SOUND exhaustive
// truth-table search (`eval_implication_by_matrix`) guarded by an unsound
// syntactic fast-path: for two pure `&&`-conjunctions it returned
// `B.atoms ⊆ A.atoms`, falling through to the matrix only for non-conjunction
// conditions. The typed `condition_expr_implies` reproduces the SOUND matrix
// for EVERY input.
//
// `matrix_oracle` is an independent re-derivation of that sound matrix on the
// retained string evaluator, so the parity guard outlives the deletion of the
// production matrix. Two properties:
//
//   1. Over disjunction/negation/parenthesised/boolean conditions — exactly the
//      cases the task names and the cases the old code routed to the matrix
//      rather than the subset shortcut — the typed path equals the oracle
//      bit-for-bit. (`PARITY_CONDITIONS`)
//
//   2. Over pure conjunctions the typed path also equals the sound oracle, and
//      is therefore a strict SUPERSET of the old subset shortcut: it accepts
//      every edge the shortcut accepted (so no dependency edge that previously
//      passed `link_verify` now fails — byte-stability holds for every
//      compiling scenario) plus the sound cross-operator implications the
//      shortcut missed (e.g. `a == 'x'` ⟹ `a != 'y'`). Those extra acceptances
//      only relax a previously-failing validation, never tighten a passing one.
//      (`CONJUNCTION_CONDITIONS`)
//
// Cardinality operators are out of the oracle's scope: the string evaluator
// predates grammar-v2 cardinality and errors on `any_of(...)`, and no
// checked-in scenario uses cardinality in a component `condition`
// (grep-confirmed). The typed cardinality semantics are pinned by the
// `implies_cardinality_*` unit tests above.

/// Disjunction/negation/parenthesised/boolean conditions — the cases the task
/// names and the cases the legacy engine evaluated through the sound matrix.
/// Over the full cross-product the typed path must equal the matrix oracle.
const PARITY_CONDITIONS: &[&str] = &[
    "variant == 'heavy' || region == 'us'",
    "variant == 'light' || region == 'eu'",
    "variant != 'heavy' || region != 'us'",
    "!(variant == 'heavy')",
    "!(variant != 'heavy')",
    "!(variant == 'heavy' && region == 'us')",
    "!(variant == 'heavy' || region == 'us')",
    "variant == 'heavy' && (region == 'us' || region == 'eu')",
    "(variant == 'heavy' || variant == 'light') && region == 'us'",
    "variant == 'heavy' || (region == 'us' && variant == 'light')",
    "!(variant == 'heavy') || region == 'us'",
    "true",
    "false",
];

#[test]
fn typed_implies_matches_matrix_oracle_over_corpus() {
    for &a in PARITY_CONDITIONS {
        for &b in PARITY_CONDITIONS {
            let typed = condition_expr_implies(&parse(a), &parse(b));
            let oracle = matrix_oracle(a, b);
            assert_eq!(
                typed, oracle,
                "matrix parity mismatch for ({a:?} => {b:?}): \
                 typed={typed} oracle={oracle}",
            );
        }
    }
}

#[test]
fn typed_implies_matches_oracle_none_precondition() {
    // `condition_implies_typed(None, Some(b))` substitutes the `true` antecedent
    // stand-in; the typed entry must reproduce the oracle for that always-true
    // antecedent.
    for &b in PARITY_CONDITIONS {
        let typed = condition_expr_implies(&parse("true"), &parse(b));
        let oracle = matrix_oracle("true", b);
        assert_eq!(
            typed, oracle,
            "None-antecedent parity mismatch for (true => {b:?})",
        );
    }
}

/// Pure `&&`-conjunctions — the byte-stability-critical edges `link_verify`
/// actually sees (a depender condition that extends the dependency's by extra
/// conjuncts).
const CONJUNCTION_CONDITIONS: &[&str] = &[
    "variant == 'heavy'",
    "variant != 'heavy'",
    "variant == 'light'",
    "region == 'us'",
    "region != 'eu'",
    "variant == 'heavy' && region == 'us'",
    "variant == 'heavy' && region != 'eu'",
    "variant == 'light' && region == 'us'",
];

/// The legacy syntactic subset shortcut: `true` iff every atom of `b` appears
/// verbatim (same op) among the atoms of `a`. Reproduced here only to assert
/// the typed path never *rejects* an edge the shortcut accepted.
/// `ConditionPredicateOp` is not `Ord`, so atoms are keyed on a `bool` `is_eq`
/// flag rather than the enum.
fn legacy_subset_shortcut(a: &str, b: &str) -> bool {
    let key = |p: &ConditionPredicate| {
        (
            p.tag.clone(),
            matches!(p.op, ConditionPredicateOp::Eq),
            p.value.clone(),
        )
    };
    let mut a_atoms: BTreeSet<(String, bool, String)> = BTreeSet::new();
    for_each_predicate(&parse(a), &mut |p| {
        a_atoms.insert(key(p));
    });
    let mut implied = true;
    for_each_predicate(&parse(b), &mut |p| {
        if !a_atoms.contains(&key(p)) {
            implied = false;
        }
    });
    implied
}

#[test]
fn typed_implies_is_superset_of_legacy_subset_shortcut() {
    // Byte-stability invariant: every edge the legacy subset shortcut ACCEPTED
    // must still be accepted by the typed path. The converse need not hold — the
    // typed path is sound and may accept additional cross-operator implications
    // the shortcut missed, but those only relax a previously-failing validation,
    // never tighten a passing one, so no compiling scenario changes its hashes.
    for &a in CONJUNCTION_CONDITIONS {
        for &b in CONJUNCTION_CONDITIONS {
            if legacy_subset_shortcut(a, b) {
                assert!(
                    condition_expr_implies(&parse(a), &parse(b)),
                    "typed path REJECTED an edge the legacy shortcut accepted \
                     ({a:?} => {b:?}); this would break byte-stability",
                );
            }
        }
    }
}

#[test]
fn typed_implies_matches_sound_matrix_for_conjunctions() {
    // The typed path equals the SOUND truth-table for pure conjunctions and
    // agrees with the matrix oracle across the conjunction corpus — including
    // the cross-operator implication the legacy subset shortcut got wrong
    // (`a == 'x'` entails `a != 'y'`).
    for &a in CONJUNCTION_CONDITIONS {
        for &b in CONJUNCTION_CONDITIONS {
            assert_eq!(
                condition_expr_implies(&parse(a), &parse(b)),
                matrix_oracle(a, b),
                "conjunction oracle mismatch for ({a:?} => {b:?})",
            );
        }
    }
    assert!(condition_expr_implies(
        &parse("variant == 'light'"),
        &parse("variant != 'heavy'"),
    ));
}
