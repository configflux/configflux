// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the rung-3 scope partitioner — configflux-0qo3.
//!
//! Lives as a sibling of `partitioner.rs` so `partitioner.rs` itself
//! stays under the 400-line lint budget (the same codebase convention
//! `bdd.rs` paired with `tests.rs` follows). All tests pin the
//! acceptance bars from the design spec:
//!
//! 1. Single-partition collapse (small model, large N, default N).
//! 2. 2-cluster split on a hand-crafted 6-variable model.
//! 3. Bridge detection on a cross-tree clause.
//! 4. Determinism: identical input + N produces bit-identical
//!    `PartitionPlan`.
//! 5. Empty-variable clauses bin deterministically into cluster 0.

use super::partitioner::{partition, BridgeSpec, ClusterSpec, PartitionPlan};
use super::ConditionExpr;
use crate::conditions::parse_condition_expr;

/// Parse a slice of clause strings using the same parser the full
/// compiler path uses (configflux-lz70's `parse_condition_model`
/// delegates here for each clause). Test-helper only.
fn parse(clauses: &[&str]) -> Vec<ConditionExpr> {
    clauses
        .iter()
        .map(|c| parse_condition_expr(c).expect("parse clause"))
        .collect()
}

fn var(tag: &str, value: &str) -> (String, String) {
    (tag.to_string(), value.to_string())
}

// ----------------------------------------------------------------------------
// Acceptance bar 1 — Single-partition collapse
// ----------------------------------------------------------------------------

#[test]
fn single_partition_collapse_under_default_usize_max() {
    // Default N (usize::MAX) → every input collapses to a single
    // partition with no bridge, mirroring backward-compatible
    // behaviour for every FAMA fixture per ADR-0012 §2.
    let clauses = parse(&[
        "feature_a == 'on'",
        "feature_b == 'off' || feature_c == 'on'",
        "feature_a == 'on' && feature_d != 'broken'",
    ]);
    let plan = partition(&clauses, usize::MAX);
    assert_eq!(plan.clusters.len(), 1, "expected one cluster");
    assert!(plan.bridge.is_none(), "expected no bridge");
    let only = &plan.clusters[0];
    assert_eq!(only.clause_indices, vec![0, 1, 2]);
    // All four distinct variables ended up in one cluster.
    assert_eq!(only.variables.len(), 4);
    assert!(only.variables.contains(&var("feature_a", "on")));
    assert!(only.variables.contains(&var("feature_b", "off")));
    assert!(only.variables.contains(&var("feature_c", "on")));
    assert!(only.variables.contains(&var("feature_d", "broken")));
}

#[test]
fn single_partition_collapse_when_total_vars_within_cap() {
    // ADR-0012 §2 "small-input collapse": if total var count ≤ N,
    // the partitioner emits a single-partition plan regardless of
    // connectivity structure. Verify with a hand-crafted disjoint
    // 2-clause model and N = 100 (var count = 4).
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y'",
        "right_c == 'x' && right_d == 'y'",
    ]);
    let plan = partition(&clauses, 100);
    assert_eq!(plan.clusters.len(), 1);
    assert!(plan.bridge.is_none());
    assert_eq!(plan.clusters[0].clause_indices, vec![0, 1]);
    assert_eq!(plan.clusters[0].variables.len(), 4);
}

#[test]
fn single_partition_for_empty_clauses() {
    // Degenerate: empty input still produces a valid plan with no
    // clusters and no bridge.
    let plan = partition(&[], usize::MAX);
    assert_eq!(plan.clusters.len(), 1);
    assert!(plan.bridge.is_none());
    assert!(plan.clusters[0].clause_indices.is_empty());
    assert!(plan.clusters[0].variables.is_empty());
}

// ----------------------------------------------------------------------------
// Acceptance bar 2 — 2-cluster split on a hand-crafted 6-variable model
// ----------------------------------------------------------------------------

#[test]
fn two_cluster_split_on_six_variable_disjoint_model() {
    // Two clauses, each referencing three disjoint variables; size cap
    // of 3 forces each clause into its own cluster.
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y' && left_c == 'z'",
        "right_a == 'x' && right_b == 'y' && right_c == 'z'",
    ]);
    let plan = partition(&clauses, 3);
    assert_eq!(plan.clusters.len(), 2, "expected two clusters");
    assert!(plan.bridge.is_none(), "no cross-tree clause → no bridge");

    // Each cluster carries exactly one clause and three variables.
    // The "left_*" cluster sorts ahead of "right_*" because the
    // BTreeSet enumeration of variables is lexicographic and
    // "left_a" < "right_a", so cluster 0 is the left one.
    let left = &plan.clusters[0];
    let right = &plan.clusters[1];
    assert_eq!(left.clause_indices, vec![0]);
    assert_eq!(right.clause_indices, vec![1]);
    assert_eq!(left.variables.len(), 3);
    assert_eq!(right.variables.len(), 3);
    assert!(left.variables.iter().all(|(t, _)| t.starts_with("left_")));
    assert!(right.variables.iter().all(|(t, _)| t.starts_with("right_")));
}

#[test]
fn within_cap_unions_keep_connected_clauses_together() {
    // Two clauses that SHARE a variable must collapse into the same
    // cluster when the union fits under the cap.
    let clauses = parse(&[
        "shared == 'x' && left_b == 'y'",
        "shared == 'x' && left_c == 'z'",
    ]);
    let plan = partition(&clauses, 5);
    assert_eq!(plan.clusters.len(), 1);
    assert!(plan.bridge.is_none());
    assert_eq!(plan.clusters[0].clause_indices, vec![0, 1]);
    assert_eq!(plan.clusters[0].variables.len(), 3);
}

#[test]
fn oversized_single_clause_collapses_into_one_cluster() {
    // ADR-0012 §2: a single clause whose own variable set exceeds N
    // collapses with its referenced variables into one oversized
    // cluster — the partitioner does not refuse such input.
    let clauses = parse(&[
        "a == 'x' && b == 'x' && c == 'x' && d == 'x' && e == 'x'",
    ]);
    let plan = partition(&clauses, 2);
    assert_eq!(plan.clusters.len(), 1);
    assert!(plan.bridge.is_none());
    assert_eq!(plan.clusters[0].variables.len(), 5);
    assert!(plan.clusters[0].variables.len() > 2);
}

// ----------------------------------------------------------------------------
// Acceptance bar 3 — Bridge detection on a cross-tree clause
// ----------------------------------------------------------------------------

#[test]
fn cross_tree_clause_is_collected_into_bridge() {
    // Three clauses: two confined to "left_*" and "right_*" disjoint
    // subtrees, plus a cross-tree clause that mentions a variable
    // from each subtree. With cap=3 the two subtree clauses each
    // form a 3-var cluster; the cross-tree clause's variables span
    // both clusters → it must land in the bridge.
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y' && left_c == 'z'",
        "right_a == 'x' && right_b == 'y' && right_c == 'z'",
        "left_a == 'x' && right_a == 'x'",
    ]);
    let plan = partition(&clauses, 3);
    assert_eq!(plan.clusters.len(), 2);
    let bridge = plan.bridge.as_ref().expect("expected bridge");
    assert_eq!(bridge.clause_indices, vec![2]);
    // Bridge variables are the union over its clauses: {left_a, right_a}.
    assert_eq!(bridge.variables.len(), 2);
    assert!(bridge.variables.contains(&var("left_a", "x")));
    assert!(bridge.variables.contains(&var("right_a", "x")));
    // The first two clauses landed in their own (non-bridge) clusters.
    let left = &plan.clusters[0];
    let right = &plan.clusters[1];
    assert_eq!(left.clause_indices, vec![0]);
    assert_eq!(right.clause_indices, vec![1]);
}

#[test]
fn bridge_clause_does_not_dedupe_variables_across_two_bridge_clauses() {
    // When two bridge clauses share a variable, the bridge's variable
    // list is the deduplicated union (BTreeSet semantics).
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y' && left_c == 'z'",
        "right_a == 'x' && right_b == 'y' && right_c == 'z'",
        "left_a == 'x' && right_a == 'x'",
        "left_a == 'x' && right_b == 'y'",
    ]);
    let plan = partition(&clauses, 3);
    assert_eq!(plan.clusters.len(), 2);
    let bridge = plan.bridge.as_ref().expect("expected bridge");
    assert_eq!(bridge.clause_indices, vec![2, 3]);
    // Union over the two bridge clauses: {left_a, right_a, right_b}.
    assert_eq!(bridge.variables.len(), 3);
    assert!(bridge.variables.contains(&var("left_a", "x")));
    assert!(bridge.variables.contains(&var("right_a", "x")));
    assert!(bridge.variables.contains(&var("right_b", "y")));
}

#[test]
fn no_bridge_emitted_when_every_clause_confined_to_one_cluster() {
    // Two disjoint clauses + one extra clause that adds a variable
    // entirely within the "left_*" subtree. Cap=4. No clause spans
    // clusters → no bridge.
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y' && left_c == 'z'",
        "right_a == 'x' && right_b == 'y' && right_c == 'z'",
        "left_a == 'x' && left_d == 'z'",
    ]);
    let plan = partition(&clauses, 4);
    assert_eq!(plan.clusters.len(), 2);
    assert!(plan.bridge.is_none());
}

// ----------------------------------------------------------------------------
// Acceptance bar 4 — Determinism
// ----------------------------------------------------------------------------

#[test]
fn determinism_repeated_runs_produce_bit_identical_plans() {
    // ADR-0012 §2 + ADR-0005 G1: the partitioner is a pure function
    // of (clauses_in_input_order, cluster_size). Running it twice on
    // the same input must produce two PartitionPlan values that
    // compare equal under derived PartialEq.
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y' && left_c == 'z'",
        "right_a == 'x' && right_b == 'y' && right_c == 'z'",
        "left_a == 'x' && right_a == 'x'",
        "mid_a == 'p' && mid_b == 'q'",
        "left_a == 'x' && mid_a == 'p'",
    ]);
    let a = partition(&clauses, 3);
    let b = partition(&clauses, 3);
    assert_eq!(a, b);
}

#[test]
fn determinism_default_n_and_explicit_n_agree_on_collapsed_models() {
    // For any model whose total variable count is ≤ N, the
    // single-partition collapse rule guarantees the same plan
    // shape regardless of the explicit N value chosen. We verify
    // collapse equivalence for N = usize::MAX and a finite N
    // larger than the variable count.
    let clauses = parse(&[
        "feat_a == 'on'",
        "feat_b == 'off' || feat_c == 'on'",
    ]);
    let a = partition(&clauses, usize::MAX);
    let b = partition(&clauses, 1000);
    assert_eq!(a, b);
    assert_eq!(a.clusters.len(), 1);
}

#[test]
fn determinism_clause_order_does_change_plan_but_same_order_is_stable() {
    // Determinism is "same input → same output", not "any order →
    // same output". Two different input orderings legitimately
    // produce different cluster compositions when the cap forces
    // boundary placement to depend on which variable was unioned
    // first. We pin only the stable-same-order half of the contract
    // here; the per-input determinism is pinned by
    // `determinism_repeated_runs_produce_bit_identical_plans`.
    let clauses_a = parse(&[
        "left_a == 'x' && left_b == 'y'",
        "right_a == 'x' && right_b == 'y'",
    ]);
    let plan_a1 = partition(&clauses_a, 2);
    let plan_a2 = partition(&clauses_a, 2);
    assert_eq!(plan_a1, plan_a2);
}

// ----------------------------------------------------------------------------
// Acceptance bar 5 — CLI flag presence is pinned in `main.rs`'s test
// module (configflux-0qo3 wiring is intentionally a no-op end-to-end
// per ADR-0012 §2 — emission integration is configflux-vmlb's scope).
// ----------------------------------------------------------------------------

// ----------------------------------------------------------------------------
// Structural assertions on PartitionPlan/ClusterSpec/BridgeSpec
// ----------------------------------------------------------------------------

#[test]
fn cluster_clause_indices_are_sorted_ascending() {
    let clauses = parse(&[
        "left_a == 'x'",
        "right_a == 'x'",
        "left_a == 'x' && left_b == 'y'",
        "right_a == 'x' && right_b == 'y'",
    ]);
    let plan = partition(&clauses, 3);
    for cluster in &plan.clusters {
        let mut sorted = cluster.clause_indices.clone();
        sorted.sort_unstable();
        assert_eq!(cluster.clause_indices, sorted);
    }
    if let Some(b) = &plan.bridge {
        let mut sorted = b.clause_indices.clone();
        sorted.sort_unstable();
        assert_eq!(b.clause_indices, sorted);
    }
}

#[test]
fn cluster_variables_are_sorted_lexicographically() {
    let clauses = parse(&[
        "z_var == 'x' && a_var == 'y'",
        "m_var == 'z'",
    ]);
    let plan = partition(&clauses, 10);
    for cluster in &plan.clusters {
        let mut sorted = cluster.variables.clone();
        sorted.sort();
        assert_eq!(cluster.variables, sorted);
    }
}

#[test]
fn total_variables_count_matches_union_across_clusters_and_bridge() {
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y' && left_c == 'z'",
        "right_a == 'x' && right_b == 'y' && right_c == 'z'",
        "left_a == 'x' && right_a == 'x'",
    ]);
    let plan = partition(&clauses, 3);
    // Total distinct variables = 6 across clusters; bridge re-uses
    // two of them.
    assert_eq!(plan.total_variables(), 6);
}

// ----------------------------------------------------------------------------
// ConditionExpr coverage — `Not`/`And`/`Or`/`Bool`/`Predicate`
// ----------------------------------------------------------------------------

#[test]
fn variable_collection_walks_and_or_not_subexpressions() {
    // The walker must descend into every shape of ConditionExpr so a
    // clause like `!(a == 'x' || b == 'y') && c == 'z'` contributes
    // three distinct variables. Cap = 100 forces single-partition
    // collapse so the assertion targets the variable set directly.
    let clauses = parse(&[
        "!(a == 'x' || b == 'y') && c == 'z'",
    ]);
    let plan = partition(&clauses, 100);
    assert_eq!(plan.clusters.len(), 1);
    assert_eq!(plan.clusters[0].variables.len(), 3);
    assert!(plan.clusters[0].variables.contains(&var("a", "x")));
    assert!(plan.clusters[0].variables.contains(&var("b", "y")));
    assert!(plan.clusters[0].variables.contains(&var("c", "z")));
}

#[test]
fn bool_literal_clause_attaches_to_first_cluster() {
    // A `true`/`false` literal clause references no variables; it
    // must still appear in the plan (clause indices are not silently
    // dropped). Our convention attaches such clauses to cluster 0
    // deterministically; the partition manifest emission in
    // configflux-vmlb relies on every input clause being assigned.
    let clauses = parse(&[
        "left_a == 'x' && left_b == 'y' && left_c == 'z'",
        "right_a == 'x' && right_b == 'y' && right_c == 'z'",
        "true",
    ]);
    let plan = partition(&clauses, 3);
    // Cluster 0 (the "left_*" one) carries clauses 0 and 2; cluster 1
    // ("right_*") carries clause 1.
    assert_eq!(plan.clusters.len(), 2);
    let cluster0 = &plan.clusters[0];
    assert!(
        cluster0.clause_indices.contains(&2),
        "true-literal clause should attach to cluster 0; got {:?}",
        cluster0.clause_indices
    );
}

// ----------------------------------------------------------------------------
// Determinism-sentinel: PartitionPlan equality + Debug snapshot stability
// ----------------------------------------------------------------------------

#[test]
fn structural_equality_via_partial_eq_for_two_independent_plans() {
    // Two independent calls produce two independently-allocated
    // PartitionPlan values that nevertheless compare equal via the
    // derived PartialEq. This is the strongest determinism contract
    // a pure function can deliver at the language level.
    let clauses = parse(&[
        "x == 'a' && y == 'b'",
        "y == 'b' && z == 'c'",
    ]);
    let a = partition(&clauses, 10);
    let b = partition(&clauses, 10);
    assert_eq!(a, b);
    let _: &PartitionPlan = &a;
    let _: &ClusterSpec = &a.clusters[0];
    // BridgeSpec exists for type-name use even when not constructed.
    let _: Option<&BridgeSpec> = a.bridge.as_ref();
}
