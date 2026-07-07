// SPDX-License-Identifier: BUSL-1.1

//! Variable-ordering heuristics for the CCM emitter.
//!
//! Per ADR-0005 §2 + §6 ("Per-heuristic determinism and cross-heuristic
//! equivalence"), `var_order_heuristic` is a first-class per-emit
//! parameter that names the strategy used to pick the BDD variable
//! order. Two stable values exist in schema_version 1:
//!
//! * [`VarOrderHeuristic::FacetNameAscending`] — legacy default. The
//!   variable order is the `BTreeSet` ordering of canonical
//!   `(tag, value)` pairs (Unicode code-point ascending). This is
//!   byte-identical to the pre-existing `BTreeMap`-driven path in
//!   `ccm_emitter::build_bdd_bin` and is the value emitted by every
//!   caller that does not opt in.
//!
//! * [`VarOrderHeuristic::ClauseGroupedDfs`] — opt-in. The walk visits
//!   clauses in input order; within each clause, variables are
//!   emitted in left-to-right DFS pre-order over the boolean
//!   expression tree. Each variable is emitted on **first** visit
//!   only; subsequent re-occurrences are skipped. This produces a
//!   deterministic order that keeps variables co-occurring in a
//!   clause adjacent in the BDD, which the M2 plan (configflux-gf2o
//!   and successors) treats as the cheap path to a tractable BDD on
//!   constraint sets with cluster-shaped supports.
//!
//! * [`VarOrderHeuristic::Force`] — opt-in. FORCE
//!   (Aloul/Markov/Sakallah, GLSVLSI 2003) computes a static variable
//!   order by iterative centre-of-gravity sweeps over the
//!   variable-clause hypergraph: each clause becomes a hyperedge over
//!   the variables it touches, each hyperedge's centroid is the mean
//!   of its variables' positions, and each variable's new position is
//!   the mean of the centroids of the hyperedges it touches. The
//!   sweep iterates until the variable order is a fixed point (no
//!   permutation change between sweeps) or 20 sweeps complete,
//!   whichever first. Ties on the floating-point score are broken by
//!   original variable index, making the output a deterministic pure
//!   function of the parsed clause list. The seed order is the
//!   clause-grouped-DFS first-seen order, which already biases the
//!   sweep toward locality-aware solutions (see configflux-pqi4).
//!
//! All three heuristics are pure functions of the parsed clause list
//! and therefore deterministic for a given input — the determinism
//! guarantee required by ADR-0005 §6 R1.

use crate::conditions::{ConditionExpr, ConditionPredicate};
use std::collections::{BTreeSet, HashSet};

/// Variable-ordering strategy for [`compute_variable_order`].
///
/// New stable values added to this enum require an ADR-0005 §2
/// amendment that documents the new value's determinism guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VarOrderHeuristic {
    /// `BTreeSet` ordering of `(tag, value)` pairs.
    FacetNameAscending,
    /// DFS walk of the clause co-occurrence graph in first-seen order.
    ClauseGroupedDfs,
    /// FORCE (Aloul/Markov/Sakallah 2003) iterative centre-of-gravity
    /// sweep on the variable-clause hypergraph. configflux-pqi4
    /// (rung 2.5a).
    Force,
}

impl VarOrderHeuristic {
    /// Wire tag string for `algorithm_params.var_order_heuristic` in
    /// `ccm.manifest.json`. The strings are pinned by ADR-0005 §2 and
    /// MUST NOT change under any v1-schema build.
    pub(crate) fn manifest_tag(self) -> &'static str {
        match self {
            VarOrderHeuristic::FacetNameAscending => "facet-name-ascending",
            VarOrderHeuristic::ClauseGroupedDfs => "clause-grouped-dfs",
            VarOrderHeuristic::Force => "force",
        }
    }
}

/// Compute the BDD variable order over the given clauses under
/// `heuristic`. The returned vector contains the canonical
/// `(tag, value)` pairs in ascending BDD-variable-index order
/// (`result[0]` → variable 0, etc.). Callers join the pair into a
/// `tag.value` symbol name via [`symbol_name`] for hashing, manifest
/// emission, and `var_to_label` rendering.
///
/// Returning the pair (instead of a pre-joined string) preserves the
/// original `tag` and `value` boundaries so callers can build
/// `var_to_label` (`tag=value`) and other diagnostics without
/// re-splitting on a `.` separator. ADR-0005 §3 fixes the canonical
/// dotted-path facet identifier, but values produced by the
/// `compiler/src/conditions` grammar may legally contain dots
/// (e.g. `version == '1.0'`); never split on `.` after the join.
pub(crate) fn compute_variable_order(
    clauses: &[ConditionExpr],
    heuristic: VarOrderHeuristic,
) -> Vec<(String, String)> {
    match heuristic {
        VarOrderHeuristic::FacetNameAscending => facet_name_ascending(clauses),
        VarOrderHeuristic::ClauseGroupedDfs => clause_grouped_dfs(clauses),
        VarOrderHeuristic::Force => force(clauses),
    }
}

/// `BTreeSet`-driven order, byte-identical to the legacy
/// `ccm_emitter::build_symbols` traversal.
fn facet_name_ascending(clauses: &[ConditionExpr]) -> Vec<(String, String)> {
    let mut symbols: BTreeSet<(String, String)> = BTreeSet::new();
    for clause in clauses {
        collect_predicates_btree(clause, &mut symbols);
    }
    symbols.into_iter().collect()
}

fn collect_predicates_btree(
    expr: &ConditionExpr,
    out: &mut BTreeSet<(String, String)>,
) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(p) => {
            out.insert((p.tag.clone(), p.value.clone()));
        }
        ConditionExpr::Not(inner) => collect_predicates_btree(inner, out),
        ConditionExpr::And(left, right) | ConditionExpr::Or(left, right) => {
            collect_predicates_btree(left, out);
            collect_predicates_btree(right, out);
        }
        // Cardinality operators: recurse into children in ascending Vec index
        // order (ADR-0006 §3), consistent with the DFS variant below.
        ConditionExpr::AnyOf(c) | ConditionExpr::AllOf(c) | ConditionExpr::ExactlyOneOf(c) => {
            c.iter().for_each(|child| collect_predicates_btree(child, out));
        }
    }
}

/// Clause-grouped DFS: walk clauses in input order; within each
/// clause walk the expression tree left-to-right DFS pre-order;
/// emit each `(tag, value)` pair on first sight only.
fn clause_grouped_dfs(clauses: &[ConditionExpr]) -> Vec<(String, String)> {
    let mut order: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for clause in clauses {
        collect_predicates_dfs(clause, &mut order, &mut seen);
    }
    order
}

fn collect_predicates_dfs(
    expr: &ConditionExpr,
    order: &mut Vec<(String, String)>,
    seen: &mut HashSet<(String, String)>,
) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(p) => emit(p, order, seen),
        ConditionExpr::Not(inner) => collect_predicates_dfs(inner, order, seen),
        ConditionExpr::And(left, right) | ConditionExpr::Or(left, right) => {
            collect_predicates_dfs(left, order, seen);
            collect_predicates_dfs(right, order, seen);
        }
        // Cardinality operators: walk children left-to-right in ascending Vec
        // index order so first-sight variable assignment matches the pinned
        // left-fold visitation order (ADR-0006 §3, §5).
        ConditionExpr::AnyOf(c) | ConditionExpr::AllOf(c) | ConditionExpr::ExactlyOneOf(c) => {
            c.iter().for_each(|child| collect_predicates_dfs(child, order, seen));
        }
    }
}

fn emit(
    predicate: &ConditionPredicate,
    order: &mut Vec<(String, String)>,
    seen: &mut HashSet<(String, String)>,
) {
    let key = (predicate.tag.clone(), predicate.value.clone());
    if seen.insert(key.clone()) {
        order.push(key);
    }
}

/// FORCE static variable ordering (configflux-pqi4, rung 2.5a).
///
/// Reference: Aloul, Markov, Sakallah, "FORCE: A Fast and Easy-To-
/// Implement Variable-Ordering Heuristic", GLSVLSI 2003. This
/// implementation follows the spring-embedder formulation in §3 of
/// the paper.
///
/// ## Pipeline
///
///   1. Seed the order with [`clause_grouped_dfs`]. The DFS first-seen
///      order is already locality-aware; FORCE refines it by spreading
///      variables to minimise total clause span.
///   2. Build the hypergraph: each clause becomes a hyperedge over the
///      de-duplicated set of variables it touches. Bool-only clauses
///      become empty hyperedges and are dropped.
///   3. Sweep up to [`FORCE_MAX_SWEEPS`] times:
///      a. For each non-empty hyperedge `e`, centroid(e) = mean of
///         positions of variables in `e`.
///      b. For each variable `v`, new score = mean of centroids of the
///         hyperedges that touch `v`. Variables with no hyperedge keep
///         their seed position.
///      c. Stable-sort variables by `(score, original_seed_index)`;
///         the seed-index tie-break is what makes the output a pure
///         deterministic function of the parsed clause list (no f64
///         NaN handling needed because scores are means of finite
///         positive integers).
///      d. If the resulting permutation matches the previous sweep's,
///         declare a fixed point and break.
///
/// ## Determinism
///
/// `seen: HashSet` is used only as a membership check; the produced
/// order comes from `clause_grouped_dfs`'s deterministic walk. Tie-
/// breaking by original index makes the stable sort total. The output
/// is therefore a byte-deterministic function of the input, satisfying
/// ADR-0005 §6 R1.
const FORCE_MAX_SWEEPS: usize = 20;

fn force(clauses: &[ConditionExpr]) -> Vec<(String, String)> {
    // Seed: clause-grouped-DFS first-seen order. This gives every
    // variable an integer index in [0, n) before the centroid sweep
    // starts.
    let seed = clause_grouped_dfs(clauses);
    let n = seed.len();
    if n <= 1 {
        // Empty / single variable is a fixed point of the sweep —
        // there is nothing to move and no edge to average. Returning
        // the seed verbatim avoids a division-by-zero on the empty
        // hypergraph and keeps the n=1 path zero-cost.
        return seed;
    }

    // Map (tag, value) -> seed index, used to translate hyperedge
    // members from string keys into compact indices.
    let mut seed_of: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::with_capacity(n);
    for (i, k) in seed.iter().enumerate() {
        seed_of.insert(k.clone(), i);
    }

    // Build hyperedges. Each non-bool-only clause contributes one
    // hyperedge holding the *de-duplicated* set of variable indices it
    // touches. Empty hyperedges are dropped (they would cause a
    // divide-by-zero in centroid computation and contribute no
    // information about variable proximity).
    let mut hyperedges: Vec<Vec<usize>> = Vec::with_capacity(clauses.len());
    for clause in clauses {
        let mut members: BTreeSet<(String, String)> = BTreeSet::new();
        collect_predicates_btree(clause, &mut members);
        if members.is_empty() {
            continue;
        }
        let mut indices: Vec<usize> = members
            .into_iter()
            .map(|k| *seed_of.get(&k).expect("var in seed by construction"))
            .collect();
        // Sort so per-edge iteration order is stable; otherwise
        // `BTreeSet` iteration order is already lexicographic on the
        // string key, but the index translation can permute it.
        indices.sort_unstable();
        hyperedges.push(indices);
    }

    // For each variable, list the hyperedges that touch it. The
    // adjacency list is a vector indexed by seed_index, so iteration
    // is cache-friendly and deterministic.
    let mut var_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (eid, edge) in hyperedges.iter().enumerate() {
        for &v in edge {
            var_edges[v].push(eid);
        }
    }

    // `pos[v]` = current integer position of variable v in the
    // working order; starts at the identity permutation (each
    // variable is at its seed index).
    let mut pos: Vec<usize> = (0..n).collect();
    // `perm[i]` = seed index of the variable currently at position i.
    let mut perm: Vec<usize> = (0..n).collect();

    for _sweep in 0..FORCE_MAX_SWEEPS {
        // Step (a): centroid of each hyperedge.
        let mut centroid: Vec<f64> = Vec::with_capacity(hyperedges.len());
        for edge in &hyperedges {
            let sum: usize = edge.iter().map(|&v| pos[v]).sum();
            centroid.push(sum as f64 / edge.len() as f64);
        }

        // Step (b): new score per variable. A variable touched by no
        // hyperedge keeps its current position (so it does not collapse
        // to score=0 and pile up at the front).
        let mut score: Vec<f64> = Vec::with_capacity(n);
        for v in 0..n {
            let edges = &var_edges[v];
            if edges.is_empty() {
                score.push(pos[v] as f64);
                continue;
            }
            let s: f64 = edges.iter().map(|&eid| centroid[eid]).sum();
            score.push(s / edges.len() as f64);
        }

        // Step (c): rank variables by (score, original_seed_index).
        // The seed-index tie-break makes the output a pure function of
        // the clause list — no map iteration order, no f64 NaN (scores
        // are means of finite non-negative ints).
        let mut ranked: Vec<usize> = (0..n).collect();
        ranked.sort_by(|&a, &b| {
            let sa = score[a];
            let sb = score[b];
            // partial_cmp is total here because all values are finite.
            sa.partial_cmp(&sb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(&b))
        });

        // Step (d): fixed-point check + commit.
        if ranked == perm {
            break;
        }
        // Update pos[] from the new permutation: the variable at
        // ranked[i] is now at position i.
        for (i, &v) in ranked.iter().enumerate() {
            pos[v] = i;
        }
        perm = ranked;
    }

    // Materialise the final order from `perm`.
    perm.into_iter()
        .map(|seed_index| seed[seed_index].clone())
        .collect()
}

/// Canonical `tag.value` join used to derive a single canonical
/// symbol name from a `(tag, value)` pair. Test-only; kept here so
/// the unit tests can build expected names without reaching back
/// into the parent's private `ccm_emitter::symbol_name`.
#[cfg(test)]
fn symbol_name(tag: &str, value: &str) -> String {
    format!("{tag}.{value}")
}

// ----------------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::conditions::parse_condition_expr;

    fn parse_all(clauses: &[&str]) -> Vec<ConditionExpr> {
        clauses
            .iter()
            .map(|c| parse_condition_expr(c).expect("valid clause"))
            .collect()
    }

    fn names(order: &[(String, String)]) -> Vec<String> {
        order.iter().map(|(t, v)| symbol_name(t, v)).collect()
    }

    #[test]
    fn manifest_tag_strings_are_pinned() {
        // These strings appear in every emitted manifest and are part
        // of the byte-stability contract. Any change requires an
        // ADR-0005 §2 amendment.
        assert_eq!(
            VarOrderHeuristic::FacetNameAscending.manifest_tag(),
            "facet-name-ascending"
        );
        assert_eq!(
            VarOrderHeuristic::ClauseGroupedDfs.manifest_tag(),
            "clause-grouped-dfs"
        );
        // configflux-pqi4: FORCE (rung 2.5a) is a stable schema-v1
        // value-namespace extension per ADR-0005 §2 +§6. Pin the wire
        // tag so a future rename triggers a deliberate amendment.
        assert_eq!(VarOrderHeuristic::Force.manifest_tag(), "force");
    }

    #[test]
    fn facet_name_ascending_is_btree_order() {
        // BTreeSet sorts (tag, value) pairs lexicographically; the
        // emitted symbol names follow the same order. Hand-compute
        // the expected output for a clause that mixes the order.
        let clauses = parse_all(&[
            "z == 'on' && a == 'enabled'",
            "a == 'enabled' || m == 'auto'",
        ]);
        let order = compute_variable_order(&clauses, VarOrderHeuristic::FacetNameAscending);
        assert_eq!(names(&order), vec!["a.enabled", "m.auto", "z.on"]);
    }

    #[test]
    fn clause_grouped_dfs_concrete_walk() {
        // Hand-computed expected order for a representative input:
        //   clause 0: z == 'on' && a == 'enabled'   -> visits z.on, a.enabled
        //   clause 1: a == 'enabled' || m == 'auto' -> a.enabled already seen, m.auto new
        //   clause 2: !(b == 'off')                 -> b.off new
        //   clause 3: m == 'auto' && z == 'on'      -> both already seen
        // Final order: z.on, a.enabled, m.auto, b.off
        let clauses = parse_all(&[
            "z == 'on' && a == 'enabled'",
            "a == 'enabled' || m == 'auto'",
            "!(b == 'off')",
            "m == 'auto' && z == 'on'",
        ]);
        let order = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
        assert_eq!(names(&order), vec!["z.on", "a.enabled", "m.auto", "b.off"]);
    }

    #[test]
    fn clause_grouped_dfs_handles_nested_or() {
        // Nested Or visits left-then-right per the AST shape.
        // (p == 'x' || q == 'y') && r == 'z'  parses to
        //   And(Or(Pred(p,x), Pred(q,y)), Pred(r,z))
        // DFS pre-order yields p.x, q.y, r.z.
        let clauses = parse_all(&["(p == 'x' || q == 'y') && r == 'z'"]);
        let order = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
        assert_eq!(names(&order), vec!["p.x", "q.y", "r.z"]);
    }

    #[test]
    fn determinism_repeated_calls_match() {
        // Same input + same heuristic must produce identical output
        // across calls. This guards against any iteration-order
        // dependency introduced by `HashSet` (which we use only as a
        // membership-tracker, not as the producer of order).
        let clauses = parse_all(&[
            "z == 'on' && a == 'enabled'",
            "a == 'enabled' || m == 'auto'",
            "!(b == 'off')",
        ]);
        let a1 = compute_variable_order(&clauses, VarOrderHeuristic::FacetNameAscending);
        let a2 = compute_variable_order(&clauses, VarOrderHeuristic::FacetNameAscending);
        assert_eq!(a1, a2);
        let b1 = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
        let b2 = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
        assert_eq!(b1, b2);
    }

    #[test]
    fn empty_clause_list_yields_empty_order() {
        let clauses: Vec<ConditionExpr> = Vec::new();
        assert!(compute_variable_order(&clauses, VarOrderHeuristic::FacetNameAscending).is_empty());
        assert!(compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs).is_empty());
    }

    #[test]
    fn bool_only_clauses_yield_empty_order() {
        let clauses = parse_all(&["true", "false", "!(true)"]);
        assert!(compute_variable_order(&clauses, VarOrderHeuristic::FacetNameAscending).is_empty());
        assert!(compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs).is_empty());
    }

    #[test]
    fn facet_name_ascending_skips_duplicates() {
        // Repeated predicates collapse via BTreeSet de-dup.
        let clauses = parse_all(&["a == 'on'", "a == 'on' && a == 'on'"]);
        let order = compute_variable_order(&clauses, VarOrderHeuristic::FacetNameAscending);
        assert_eq!(names(&order), vec!["a.on"]);
    }

    #[test]
    fn clause_grouped_dfs_skips_duplicates() {
        // Repeated predicates collapse via the `seen` set.
        let clauses = parse_all(&["a == 'on'", "a == 'on' && a == 'on'"]);
        let order = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
        assert_eq!(names(&order), vec!["a.on"]);
    }

    #[test]
    fn distinct_values_for_same_tag_are_distinct_variables() {
        // Per ADR-0005 §3, the variable name space is `tag.value`,
        // so `a == 'on'` and `a == 'off'` are two BDD variables.
        // Both heuristics must respect this.
        let clauses = parse_all(&["a == 'on' && a == 'off'"]);
        let asc = compute_variable_order(&clauses, VarOrderHeuristic::FacetNameAscending);
        assert_eq!(names(&asc), vec!["a.off", "a.on"]);
        let dfs = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
        assert_eq!(names(&dfs), vec!["a.on", "a.off"]);
    }

    #[test]
    fn values_with_dots_round_trip_via_pair_not_split() {
        // ADR-0005 §3 says facet names join with `.`, but values from
        // the conditions grammar may contain dots. Returning pairs
        // (instead of pre-joined strings + a `.split('.')` round-trip)
        // is what makes such values safe.
        let clauses = parse_all(&["version == '1.0' || version == '2.0'"]);
        let order = compute_variable_order(&clauses, VarOrderHeuristic::ClauseGroupedDfs);
        assert_eq!(
            order,
            vec![
                ("version".to_string(), "1.0".to_string()),
                ("version".to_string(), "2.0".to_string()),
            ]
        );
    }

}

// FORCE-specific tests live in a sibling test module so the impl
// file stays under the 400-line lint budget. See
// `var_order_force_tests.rs` for the centre-of-gravity invariant,
// span-reduction, and determinism assertions added by configflux-pqi4.
#[cfg(test)]
mod force_tests;
