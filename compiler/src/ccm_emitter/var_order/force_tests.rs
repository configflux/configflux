// SPDX-License-Identifier: BUSL-1.1

//! FORCE-specific unit tests (configflux-pqi4, rung 2.5a).
//!
//! Split out of `var_order.rs` so the implementation file stays
//! within the 400-line lint budget. These tests pin three properties
//! that the FORCE heuristic must satisfy:
//!
//!   (a) **determinism** — the byte-stability contract from
//!       ADR-0005 §6 R1: same input + same heuristic = same output
//!       bytes across calls.
//!   (b) **centre-of-gravity invariant** — verified on a
//!       hand-computed 4-variable / 3-clause hypergraph; FORCE must
//!       converge to the chain order that minimises clause span.
//!   (c) **span reduction vs adversarial input** — on an input
//!       order that interleaves co-occurring variables, FORCE must
//!       produce an order whose total clause span is no worse than
//!       the seed `clause-grouped-dfs` order. This is the falsifiable
//!       property; if it fails, FORCE has no chance of reducing the
//!       BDD `unique`-table peak on the gf2o workload.

use super::{
    collect_predicates_btree, compute_variable_order, ConditionExpr, VarOrderHeuristic,
};
use crate::conditions::parse_condition_expr;
use std::collections::BTreeSet;

fn parse_all(clauses: &[&str]) -> Vec<ConditionExpr> {
    clauses
        .iter()
        .map(|c| parse_condition_expr(c).expect("valid clause"))
        .collect()
}

fn names(order: &[(String, String)]) -> Vec<String> {
    order.iter().map(|(t, v)| format!("{t}.{v}")).collect()
}

fn span_under_order(
    clauses: &[ConditionExpr],
    order: &[(String, String)],
) -> usize {
    // Total span = sum over clauses of (max_pos - min_pos + 1) for the
    // variables that clause references. Bool-only clauses contribute
    // 0. This is the metric FORCE minimises, and the same metric that
    // bounds intermediate BDD width during the clause AND-fold.
    use std::collections::BTreeMap as Map;
    let mut pos: Map<(String, String), usize> = Map::new();
    for (i, k) in order.iter().enumerate() {
        pos.insert(k.clone(), i);
    }
    let mut total = 0usize;
    for c in clauses {
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        collect_predicates_btree(c, &mut seen);
        if seen.is_empty() {
            continue;
        }
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        for k in &seen {
            let p = *pos.get(k).expect("var present");
            if p < lo {
                lo = p;
            }
            if p > hi {
                hi = p;
            }
        }
        total += hi - lo + 1;
    }
    total
}

#[test]
fn force_is_deterministic() {
    // R1 byte-stability: same input + same heuristic = same output
    // across calls. FORCE uses a `HashSet` only for membership (never
    // for ordering), and ties are broken by original variable index —
    // both are deterministic operations.
    let clauses = parse_all(&[
        "z == 'on' && a == 'enabled'",
        "a == 'enabled' || m == 'auto'",
        "!(b == 'off')",
        "m == 'auto' && z == 'on'",
        "(p == 'x' || q == 'y') && r == 'z'",
    ]);
    let a1 = compute_variable_order(&clauses, VarOrderHeuristic::Force);
    let a2 = compute_variable_order(&clauses, VarOrderHeuristic::Force);
    let a3 = compute_variable_order(&clauses, VarOrderHeuristic::Force);
    assert_eq!(a1, a2);
    assert_eq!(a2, a3);
}

#[test]
fn force_empty_clause_list_yields_empty_order() {
    let clauses: Vec<ConditionExpr> = Vec::new();
    assert!(compute_variable_order(&clauses, VarOrderHeuristic::Force).is_empty());
}

#[test]
fn force_bool_only_clauses_yield_empty_order() {
    // Bool-only clauses contribute no variables to any hyperedge, so
    // the FORCE input is empty; the heuristic must emit an empty order
    // rather than panic on a divide-by-zero.
    let clauses = parse_all(&["true", "false", "!(true)"]);
    assert!(compute_variable_order(&clauses, VarOrderHeuristic::Force).is_empty());
}

#[test]
fn force_single_variable_round_trips() {
    // A single variable is a fixed point of the centroid sweep: there
    // is nothing else to move it relative to. The output must contain
    // exactly that variable in `(tag, value)` form.
    let clauses = parse_all(&["a == 'on'"]);
    let order = compute_variable_order(&clauses, VarOrderHeuristic::Force);
    assert_eq!(order, vec![("a".to_string(), "on".to_string())]);
}

#[test]
fn force_centre_of_gravity_invariant_4var_3clause_toy() {
    // Hand-computed toy: 4 variables {a, b, c, d}, 3 clauses.
    //   clause 0: a == 'on' && b == 'on'   -> {a, b}
    //   clause 1: c == 'on' && d == 'on'   -> {c, d}
    //   clause 2: b == 'on' && c == 'on'   -> {b, c}
    //
    // The clause-grouped-DFS seed positions are
    //   a=0, b=1, c=2, d=3 (visit order in clause 0, 1, 2).
    // Hyperedge centroids in the seed order:
    //   e0 (a,b) = (0+1)/2 = 0.5
    //   e1 (c,d) = (2+3)/2 = 2.5
    //   e2 (b,c) = (1+2)/2 = 1.5
    //
    // New variable scores (mean of the centroids of the edges
    // touching the variable):
    //   a -> mean(0.5)        = 0.5
    //   b -> mean(0.5, 1.5)   = 1.0
    //   c -> mean(2.5, 1.5)   = 2.0
    //   d -> mean(2.5)        = 2.5
    //
    // Ranking ascending by (score, original_index): a, b, c, d.
    // Already a fixed point — FORCE converges in one sweep.
    //
    // The centre-of-gravity invariant we assert: every clause's
    // variables are contiguous in the FORCE order on this toy
    // (span = exactly the clause's size for every clause).
    let clauses = parse_all(&[
        "a == 'on' && b == 'on'",
        "c == 'on' && d == 'on'",
        "b == 'on' && c == 'on'",
    ]);
    let order = compute_variable_order(&clauses, VarOrderHeuristic::Force);
    assert_eq!(
        names(&order),
        vec!["a.on", "b.on", "c.on", "d.on"],
        "FORCE must converge to the chain order on this toy",
    );
    // Total span on this toy is 2 + 2 + 2 = 6, the lower bound.
    assert_eq!(span_under_order(&clauses, &order), 6);
}

#[test]
fn force_does_not_worsen_span_vs_adversarial_input_order() {
    // The falsifiable property: on an input that places co-occurring
    // variables far apart in clause-grouped-DFS first-seen order, FORCE
    // must produce an order whose total clause span is no worse than
    // the seed order. If FORCE cannot at least preserve the seed span
    // on a deliberately bad input, it has no chance of reducing peak
    // unique on the gf2o workload — that is the rung 2.5a falsification
    // chain.
    //
    // The clauses below pair variables that the input order separates:
    //   (a,e), (b,d), (c,c2), (a,d), (b,e).
    let clauses = parse_all(&[
        "a == 'v' && e == 'v'",
        "b == 'v' && d == 'v'",
        "c == 'v' && c2 == 'v'",
        "a == 'v' && d == 'v'",
        "b == 'v' && e == 'v'",
    ]);
    let dfs_order = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
    let force_order = compute_variable_order(&clauses, VarOrderHeuristic::Force);
    let dfs_span = span_under_order(&clauses, &dfs_order);
    let force_span = span_under_order(&clauses, &force_order);
    assert!(
        force_span <= dfs_span,
        "FORCE span {force_span} must be <= clause-grouped-dfs span {dfs_span} \
         on the adversarial input"
    );
    // The same set of variables must appear in both orders.
    let dfs_set: BTreeSet<_> = dfs_order.iter().cloned().collect();
    let force_set: BTreeSet<_> = force_order.iter().cloned().collect();
    assert_eq!(dfs_set, force_set);
}

#[test]
fn force_disconnected_components_stay_internally_contiguous() {
    // Two clusters that never share a clause. FORCE's centre-of-gravity
    // sweep operates on each connected component independently (a
    // variable's score is the mean of the centroids of the edges it
    // touches), so each cluster's variables must remain contiguous in
    // the output — the property that bounds intermediate BDD width on
    // a forest-of-clusters workload.
    let clauses = parse_all(&[
        "x1 == 'v' && x2 == 'v'",
        "x2 == 'v' && x3 == 'v'",
        "y1 == 'v' && y2 == 'v'",
        "y2 == 'v' && y3 == 'v'",
    ]);
    let order = compute_variable_order(&clauses, VarOrderHeuristic::Force);
    let tags: Vec<String> = order.iter().map(|(t, _)| t.clone()).collect();
    let xs: Vec<usize> = tags
        .iter()
        .enumerate()
        .filter(|(_, n)| n.starts_with('x'))
        .map(|(i, _)| i)
        .collect();
    let ys: Vec<usize> = tags
        .iter()
        .enumerate()
        .filter(|(_, n)| n.starts_with('y'))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(xs.len(), 3);
    assert_eq!(ys.len(), 3);
    // Contiguous run = max - min + 1 == count.
    assert_eq!(
        xs.iter().max().unwrap() - xs.iter().min().unwrap() + 1,
        xs.len(),
        "x* cluster must be contiguous; got positions {xs:?} for names {tags:?}"
    );
    assert_eq!(
        ys.iter().max().unwrap() - ys.iter().min().unwrap() + 1,
        ys.len(),
        "y* cluster must be contiguous; got positions {ys:?} for names {tags:?}"
    );
}

#[test]
fn force_output_variable_set_is_invariant_under_clause_permutation() {
    // FORCE's seed is the clause-grouped-DFS first-seen order, which
    // depends on input clause order, so different permutations can
    // legitimately converge to different orders. The weaker but still
    // useful property pinned here: the OUTPUT VARIABLE SET is invariant
    // under input clause permutation. This guards against accidental
    // variable drops (e.g. an off-by-one in the hyperedge sweep).
    let clauses_a = parse_all(&[
        "a == 'on' && b == 'on'",
        "b == 'on' && c == 'on'",
        "c == 'on' && d == 'on'",
    ]);
    let clauses_b = parse_all(&[
        "c == 'on' && d == 'on'",
        "a == 'on' && b == 'on'",
        "b == 'on' && c == 'on'",
    ]);
    let oa: BTreeSet<_> = compute_variable_order(&clauses_a, VarOrderHeuristic::Force)
        .into_iter()
        .collect();
    let ob: BTreeSet<_> = compute_variable_order(&clauses_b, VarOrderHeuristic::Force)
        .into_iter()
        .collect();
    assert_eq!(oa, ob);
}
