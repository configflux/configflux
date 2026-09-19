// SPDX-License-Identifier: BUSL-1.1

//! Variable-connectivity bipartite graph partitioner — configflux-0qo3,
//! per ADR-0012 §2 (rung-3 scope partitioning).
//!
//! Splits a `Vec<ConditionExpr>` into clusters whose variable sets are
//! bounded (target ≤ `cluster_size`), plus an optional bridge of
//! clauses whose variables span more than one cluster.
//!
//! Determinism contract (ADR-0012 §2 + ADR-0005 G1):
//! the function is a pure function of `(clauses_in_input_order,
//! cluster_size)`. Variables are collected via `BTreeSet` so iteration
//! order is the sorted lexicographic order of `(tag, value)`; clusters
//! are scanned in ascending root-slot order; per-cluster clause lists
//! are sorted ascending before emission. Two invocations on the same
//! input produce bit-identical `PartitionPlan` output.
//!
//! This module operates only on `ConditionExpr` trees and variable
//! name strings — no CUDD, no BDD construction. Downstream sub-issues
//! (configflux-vmlb) wire the produced plan into the per-cluster BDD
//! emission pipeline.

use super::ConditionExpr;
use crate::conditions::ConditionPredicate;
use std::collections::{BTreeMap, BTreeSet};

/// One cluster of clauses whose variable sets are connected and whose
/// union does not exceed `cluster_size` (modulo oversized single
/// clauses, see ADR-0012 §2). Clause and variable orderings are
/// deterministic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClusterSpec {
    /// Indices into the original `Vec<ConditionExpr>`, sorted ascending.
    pub(crate) clause_indices: Vec<usize>,
    /// All variables `(tag, value)` referenced by any clause in this
    /// cluster. Sorted lexicographically (BTreeSet iteration order).
    pub(crate) variables: Vec<(String, String)>,
}

/// Bridge: the set of clauses whose variable sets span more than one
/// cluster (i.e. cross-tree clauses). The variable scope is the union
/// of all variables those clauses reference. Empty bridge is encoded
/// as `None` on `PartitionPlan::bridge`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeSpec {
    /// Indices into the original `Vec<ConditionExpr>`, sorted ascending.
    pub(crate) clause_indices: Vec<usize>,
    /// Union of variables referenced by any bridge clause. Sorted.
    pub(crate) variables: Vec<(String, String)>,
}

/// Output of the partitioner: a deterministic plan describing how a
/// `Vec<ConditionExpr>` decomposes into N clusters plus an optional
/// bridge. Construction is guaranteed deterministic per the module
/// doc-comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartitionPlan {
    /// Clusters in emission order (ascending root-slot order; see
    /// module doc-comment for the determinism rationale).
    pub(crate) clusters: Vec<ClusterSpec>,
    /// Cross-tree bridge clauses. `None` when no clause's variables
    /// span more than one cluster.
    pub(crate) bridge: Option<BridgeSpec>,
}

impl PartitionPlan {
    /// Total number of distinct variables across every cluster plus
    /// the bridge (with bridge variables generally a subset of the
    /// per-cluster unions, so this is **not** simply a sum).
    #[cfg(test)]
    pub(crate) fn total_variables(&self) -> usize {
        let mut seen: BTreeSet<&(String, String)> = BTreeSet::new();
        for c in &self.clusters {
            for v in &c.variables {
                seen.insert(v);
            }
        }
        if let Some(b) = &self.bridge {
            for v in &b.variables {
                seen.insert(v);
            }
        }
        seen.len()
    }
}

/// Build a [`PartitionPlan`] from a slice of parsed clauses under the
/// supplied `cluster_size` target.
///
/// `cluster_size == usize::MAX` (the production default per
/// `--cluster-size`'s `default_value` in `compiler/src/main.rs`)
/// short-circuits to a single-partition collapse: one `ClusterSpec`
/// containing every clause and every variable, no bridge. Any
/// `cluster_size` such that the total variable count `≤ cluster_size`
/// also collapses to a single partition (ADR-0012 §2 small-input
/// collapse rule).
pub(crate) fn partition(clauses: &[ConditionExpr], cluster_size: usize) -> PartitionPlan {
    // Phase 1: collect per-clause variable sets in BTreeSet order so
    // every later iteration is byte-deterministic.
    let per_clause_vars: Vec<BTreeSet<(String, String)>> =
        clauses.iter().map(collect_variables).collect();

    // Single-partition collapse — see ADR-0012 §2 small-input rule.
    let mut all_vars: BTreeSet<(String, String)> = BTreeSet::new();
    for vars in &per_clause_vars {
        for v in vars {
            all_vars.insert(v.clone());
        }
    }
    if all_vars.len() <= cluster_size {
        let cluster_indices: Vec<usize> = (0..clauses.len()).collect();
        return PartitionPlan {
            clusters: vec![ClusterSpec {
                clause_indices: cluster_indices,
                variables: all_vars.into_iter().collect(),
            }],
            bridge: None,
        };
    }

    // Phase 2: variable union-find with size-cap guard. Variables are
    // identified by their slot in `var_slots` (input-order assigned
    // as a function of BTreeSet enumeration above, so slot order is
    // lexicographic over `(tag, value)`).
    let var_slot: BTreeMap<(String, String), usize> = all_vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.clone(), i))
        .collect();
    let n_vars = all_vars.len();
    let mut uf = UnionFind::new(n_vars);

    for vars in &per_clause_vars {
        let slots: Vec<usize> = vars.iter().map(|v| var_slot[v]).collect();
        if slots.is_empty() {
            continue;
        }
        // ADR-0012 §2: if a single clause's own variable set already
        // exceeds the cap, all of its variables must collapse into
        // one oversized cluster — the partitioner does not refuse
        // such input. For clauses ≤ cap, fall back to the
        // size-bounded greedy union; refused unions leave variables
        // in separate roots and turn the clause into a bridge
        // candidate (resolved in phase 3).
        let force_union = vars.len() > cluster_size;
        let mut iter = slots.iter().copied();
        let anchor = iter.next().expect("non-empty");
        for s in iter {
            if force_union {
                uf.force_union(anchor, s);
            } else {
                uf.try_union(anchor, s, cluster_size);
            }
        }
    }

    // Phase 3: bin clauses to clusters or to bridge by counting how
    // many distinct roots their variable sets span.
    let mut cluster_of_root: BTreeMap<usize, usize> = BTreeMap::new(); // root_slot -> cluster_index
    let mut clusters: Vec<ClusterSpec> = Vec::new();
    let mut bridge_clauses: Vec<usize> = Vec::new();
    let mut bridge_vars: BTreeSet<(String, String)> = BTreeSet::new();

    // First materialize a deterministic enumeration of cluster roots:
    // walk variables in ascending slot order, taking each var's root.
    // The first time we see a root, allocate the next cluster index
    // for it. This produces clusters in "first-touched-by-ascending-
    // slot-order" order, which is a pure function of the input.
    for slot in 0..n_vars {
        let r = uf.find(slot);
        cluster_of_root.entry(r).or_insert_with(|| {
            let idx = clusters.len();
            clusters.push(ClusterSpec {
                clause_indices: Vec::new(),
                variables: Vec::new(),
            });
            idx
        });
    }

    // Bin each variable into its cluster's variable list.
    for (var, slot) in &var_slot {
        let r = uf.find(*slot);
        let ci = cluster_of_root[&r];
        clusters[ci].variables.push(var.clone());
    }
    // Variables are already in lexicographic order because we iterated
    // `var_slot` (a BTreeMap) in key order.

    // Bin each clause: count distinct cluster indices spanned.
    for (i, vars) in per_clause_vars.iter().enumerate() {
        if vars.is_empty() {
            // Empty-variable clauses (e.g. `true` / `false`) attach to
            // cluster 0 deterministically; they cannot span clusters.
            if !clusters.is_empty() {
                clusters[0].clause_indices.push(i);
            }
            continue;
        }
        let touched: BTreeSet<usize> = vars
            .iter()
            .map(|v| cluster_of_root[&uf.find(var_slot[v])])
            .collect();
        if touched.len() == 1 {
            let ci = *touched.iter().next().expect("non-empty");
            clusters[ci].clause_indices.push(i);
        } else {
            bridge_clauses.push(i);
            for v in vars {
                bridge_vars.insert(v.clone());
            }
        }
    }

    // Clause indices are appended in input order; that is already
    // ascending. Pin the contract with a sort just in case future
    // refactors break the invariant cheaply (n_clauses ≤ N_total).
    for c in &mut clusters {
        c.clause_indices.sort_unstable();
    }
    bridge_clauses.sort_unstable();

    let bridge = if bridge_clauses.is_empty() {
        None
    } else {
        Some(BridgeSpec {
            clause_indices: bridge_clauses,
            variables: bridge_vars.into_iter().collect(),
        })
    };

    PartitionPlan { clusters, bridge }
}

/// Collect every `(tag, value)` pair referenced by a `ConditionExpr`
/// into a `BTreeSet` (deterministic ordering). Mirrors the
/// pre-existing `compile_predicate` logic in `ccm_emitter.rs` — both
/// `Eq` and `NotEq` predicates contribute the same `(tag, value)`
/// variable. `Bool` literals contribute nothing.
fn collect_variables(expr: &ConditionExpr) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    walk(expr, &mut out);
    out
}

/// Count the distinct `(tag, value)` variables referenced across a slice
/// of parsed clauses (configflux-9pjy.2 / ADR-0039). This is the
/// `total_vars_hint` the soft-budget derivation
/// (`resource_budget::derive_knobs`) uses to project the unique-table
/// footprint and decide whether to derive a `cluster_size`. Pure and
/// deterministic — the same projection the partitioner itself walks, so
/// the hint and the eventual partition layout agree.
pub(crate) fn count_distinct_variables(clauses: &[ConditionExpr]) -> usize {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for clause in clauses {
        walk(clause, &mut seen);
    }
    seen.len()
}

fn walk(expr: &ConditionExpr, out: &mut BTreeSet<(String, String)>) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(ConditionPredicate { tag, value, .. }) => {
            out.insert((tag.clone(), value.clone()));
        }
        ConditionExpr::Not(inner) => walk(inner, out),
        ConditionExpr::And(l, r) | ConditionExpr::Or(l, r) => {
            walk(l, out);
            walk(r, out);
        }
        // Cardinality operators (ADR-0006 §3): a cardinality node references
        // exactly the union of the variables its children reference, so the
        // partitioner walks each child in ascending Vec index order.
        ConditionExpr::AnyOf(children)
        | ConditionExpr::AllOf(children)
        | ConditionExpr::ExactlyOneOf(children) => {
            for child in children {
                walk(child, out);
            }
        }
        // Expanded before emission (configflux-secb.2 / ADR-0057 §D5), so an
        // unexpanded node references no variable slot here.
        ConditionExpr::FacetCompare { .. } => {}
    }
}

/// Compact union-find over variable slots with a size-cap guard.
///
/// `try_union(a, b, cap)` merges only if the resulting cluster's size
/// would not exceed `cap`; otherwise it leaves both roots intact and
/// returns `false`. The smaller-into-larger tie-break with ascending-
/// root preference keeps `find()` results stable under repeat
/// invocations on the same input (no path compression — see field
/// docs).
struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            size: vec![1; n],
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        // Iterative two-pass path compression: keeps `find` O(α(n))
        // while remaining deterministic — the function is a pure
        // function of `parent` at the start of the call.
        while self.parent[x] != x {
            x = self.parent[x];
        }
        x
    }

    /// Returns true iff the union actually happened.
    fn try_union(&mut self, a: usize, b: usize, cap: usize) -> bool {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return false;
        }
        let combined = self.size[ra] + self.size[rb];
        // Both already-oversized roots can still be unioned only when
        // their existing sizes were oversized inputs (an oversized
        // single clause; ADR-0012 §2). For the common path, refuse
        // when `combined > cap`.
        if combined > cap && self.size[ra] <= cap && self.size[rb] <= cap {
            return false;
        }
        self.merge_roots(ra, rb, combined);
        true
    }

    /// Unconditional union — used for the ADR-0012 §2 oversized
    /// single-clause case where the clause's own variable set is
    /// already larger than `cluster_size` and must collapse into one
    /// oversized cluster regardless of cap.
    fn force_union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let combined = self.size[ra] + self.size[rb];
        self.merge_roots(ra, rb, combined);
    }

    /// Tie-break: smaller into larger; equal sizes → lower slot
    /// becomes root. Both halves are deterministic functions of the
    /// input order, which keeps `find()` results stable across
    /// repeat invocations on the same input.
    fn merge_roots(&mut self, ra: usize, rb: usize, combined: usize) {
        let (root, child) = if self.size[ra] > self.size[rb] {
            (ra, rb)
        } else if self.size[ra] < self.size[rb] {
            (rb, ra)
        } else if ra < rb {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent[child] = root;
        self.size[root] = combined;
    }
}

// Unit tests live in the sibling `partitioner_tests.rs` declared
// from the parent module (`ccm_emitter.rs`). The sibling-from-parent
// layout keeps this file under the 400-line lint budget and matches
// the codebase convention used by `bdd.rs` paired with `tests.rs`.
