// SPDX-License-Identifier: BUSL-1.1

//! Scale test for `compiler::ccm_emitter` under
//! `var_order_heuristic = "clause-grouped-dfs"`.
//!
//! Builds N-feature synthetic [`ConditionModel`]s in-process at
//! N ∈ {1k, 5k, 10k}, emits each one through
//! [`emit_ccm_dir_with_heuristic`]; wall-time is asserted vs the documented
//! thresholds below only when CONFIGFLUX_SCALE_WALLTIME_ASSERT is set:
//!
//!   * N = 1000  → ≤ 30 s   (per-change gate)
//!   * N = 5000  → ≤ 90 s   (per-change gate)
//!   * N = 10000 → ≤ 300 s  (`#[ignore]`d — on-demand acceptance tier via
//!     `//compiler:ccm_emitter_scale_10k_test`; configflux-nnob, ADR-0045;
//!     cross-tree=50/50 is `#[ignore]`d under bd-93oj as bd-9xgw's wall-time gate)
//!
//! Thresholds are calibrated for the dev-container reference machine
//! (see `docs/benchmarks/v0.3.0-results.md`). They are best-effort
//! and may need adjustment if the host hardware differs significantly.
//! Treat threshold churn as a hardware-class signal, not a policy
//! signal — bd-9xgw owns the canonical benchmark numbers.
//!
//! The compiler crate (and its integration tests) intentionally do
//! NOT depend on `tools/gen_synthetic`. The cluster-forest generator
//! is mirrored inline in `mod fixture` (cluster_size=100, max_depth=5,
//! xorshift64* RNG seed 0x00C0_FFEE) and matches the offline fixture
//! shape used by bd-8dm.7. The in-gate rotation (1k/5k cross-tree=0) is
//! `#[ignore]`-free; the 10k tier (heavy/acceptance) and the bd-93oj
//! cross-tree=50/50 case (bd-9xgw-owned) are `#[ignore]`d.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use compiler::ccm_emitter::emit_ccm_dir_with_heuristic;

/// Wire tag for the opt-in clause-grouped-DFS heuristic. Pinned by
/// ADR-0005 §2.
const HEURISTIC_CLAUSE_DFS: &str = "clause-grouped-dfs";

#[test]
fn scale_emit_1k_under_clause_grouped_dfs() {
    run_scale_emit(1_000, Duration::from_secs(30));
}

#[test]
fn scale_emit_5k_under_clause_grouped_dfs() {
    run_scale_emit(5_000, Duration::from_secs(90));
}

#[test]
#[ignore = "10k tier is heavy (RSS); runs on-demand via //validation:acceptance (ADR-0045)"]
fn scale_emit_10k_under_clause_grouped_dfs() {
    run_scale_emit(10_000, Duration::from_secs(300));
}

/// bd-93oj: scale test with cross-tree=50/50 at N=10k under
/// clause-grouped-dfs. This is the fixture shape that bd-9xgw needs
/// to regen and that previously OOMed the host (peak RSS ~7.2 GB,
/// killed at 293 s by `tools/resource_guard.sh`). The 5-min wall-time
/// threshold matches the M1 envelope per ADR-0004 §M1. RSS itself is
/// validated out-of-band by the resource-guard-wrapped regen invoked
/// from the bd-93oj task brief, since in-process peak RSS measurement
/// from Rust is unreliable across kernels.
///
/// `#[ignore]` because the 5-min wall-time envelope is the
/// configflux-9xgw acceptance gate, not bd-93oj's. bd-93oj's job is
/// to land the memo-bounding *mechanism*. The env-sensitive wall-time
/// assertion runs on demand via:
///
///     bazel test //compiler:ccm_emitter_scale_test \
///       --test_arg=--ignored \
///       --test_arg=scale_emit_10k_cross_tree_50_50_under_clause_grouped_dfs
///
/// or out-of-band via `tools/resource_guard.sh` around the
/// `gen_synthetic` regen (see configflux-9xgw).
#[test]
#[ignore = "wall-time envelope is configflux-9xgw's gate, not 93oj's"]
fn scale_emit_10k_cross_tree_50_50_under_clause_grouped_dfs() {
    run_scale_emit_with_params(
        fixture::GenParams {
            features: 10_000,
            requires: 50,
            excludes: 50,
            ..fixture::GenParams::default()
        },
        Duration::from_secs(300),
    );
}

fn run_scale_emit(features: u32, threshold: Duration) {
    run_scale_emit_with_params(
        fixture::GenParams {
            features,
            ..fixture::GenParams::default()
        },
        threshold,
    );
}

fn run_scale_emit_with_params(params: fixture::GenParams, threshold: Duration) {
    let label = format!(
        "{}_r{}_e{}",
        params.features, params.requires, params.excludes
    );
    let model = fixture::build_model(params);
    let dir = tempdir_for(&format!("ccm_emitter_scale_{label}"));

    let start = Instant::now();
    emit_ccm_dir_with_heuristic(&model, &dir, HEURISTIC_CLAUSE_DFS)
        .expect("emit under clause-grouped-dfs");
    let elapsed = start.elapsed();

    // configflux-pz81: wall-time was a flaky CI gate; assert only when opted in.
    eprintln!("[scale] label={label} elapsed={elapsed:?} threshold={threshold:?}");
    if std::env::var_os("CONFIGFLUX_SCALE_WALLTIME_ASSERT").is_some() {
        assert!(elapsed <= threshold, "label={label} took {elapsed:?} > {threshold:?}");
    }
    // configflux-vmlb / ADR-0012 §4: v2 multi-part layout — the
    // single-partition triple lives in `partition-0000/`.
    assert!(dir.join("partition-manifest.json").is_file());
    assert!(dir.join("ccm.manifest.json").is_file());
    let p0 = dir.join("partition-0000");
    assert!(p0.join("ccm.bdd.bin").is_file());
    assert!(p0.join("ccm.symbols.json").is_file());
    assert!(p0.join("ccm.manifest.json").is_file());
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}

// ----------------------------------------------------------------------------
// Synthetic feature-model generator (mirrors tools/gen_synthetic shape).
// ----------------------------------------------------------------------------
//
// This module is a faithful in-tree copy of the cluster-forest
// algorithm in `tools/gen_synthetic/src/generator.rs::build_model`,
// reduced to the minimum surface this test needs:
//
//   * Cluster-forest topology (cluster_size=100, max_depth=5).
//   * Deterministic xorshift64* RNG seeded with 0x00C0_FFEE.
//   * Cross-tree counts default to 0 (this test is pure cluster
//     scale, not the cross-tree regression envelope — that's bd-9xgw).
//
// We do NOT take a dependency on `tools/gen_synthetic` because:
//   1. The compiler crate's deps must stay minimal.
//   2. Tests should be self-contained.
//   3. The generator is offline tooling, not part of the public API.
mod fixture {
    use super::*;

    pub(super) struct GenParams {
        pub(super) features: u32,
        pub(super) seed: u64,
        pub(super) requires: u32,
        pub(super) excludes: u32,
        pub(super) mandatory_pct: u8,
        pub(super) cluster_size: u32,
        pub(super) max_depth: u32,
    }

    impl Default for GenParams {
        fn default() -> Self {
            Self {
                features: 10_000,
                // Matches tools/gen_synthetic's default seed so the
                // emitted model shape stays comparable to the gate
                // fixture (bd-8dm.7).
                seed: 0x00C0_FFEE,
                // Cross-tree=0 per bd-lxah scope. Cross-tree=50/50 is
                // bd-9xgw's territory.
                requires: 0,
                excludes: 0,
                mandatory_pct: 20,
                cluster_size: 100,
                max_depth: 5,
            }
        }
    }

    /// Deterministic xorshift64* RNG. Mirrors `tools/gen_synthetic/
    /// src/generator.rs::Rng`.
    struct Rng {
        state: u64,
    }

    impl Rng {
        fn new(seed: u64) -> Self {
            let s = if seed == 0 {
                0xdead_beef_cafe_babe
            } else {
                seed
            };
            Self { state: s }
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.state;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.state = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn gen_range(&mut self, lo: u32, hi: u32) -> u32 {
            assert!(hi > lo, "gen_range: hi must exceed lo");
            let span = (hi - lo) as u64;
            lo + ((self.next_u64() % span) as u32)
        }

        fn gen_pct(&mut self, pct: u8) -> bool {
            (self.next_u64() % 100) < (pct as u64)
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum Relation {
        Optional,
        Mandatory,
    }

    #[derive(Debug, Clone, Copy)]
    struct TreeEdge {
        child: u32,
        parent: u32,
        relation: Relation,
    }

    #[derive(Debug, Clone, Copy)]
    enum CrossTree {
        Requires(u32, u32),
        Excludes(u32, u32),
    }

    fn pick_relation(rng: &mut Rng, mandatory_pct: u8) -> Relation {
        if rng.gen_pct(mandatory_pct) {
            Relation::Mandatory
        } else {
            Relation::Optional
        }
    }

    fn atom(feature_tag: &str) -> String {
        format!("{feature_tag} == 'on'")
    }

    fn implication(lhs_tag: &str, rhs_tag: &str) -> String {
        format!("!({lhs_tag} == 'on') || ({rhs_tag} == 'on')")
    }

    /// Build the synthetic feature model. See module docs for the
    /// reference algorithm — this mirrors `tools/gen_synthetic/
    /// src/generator.rs::build_model` for shape only; we do NOT
    /// reproduce the `bound_model_hash` derivation since the gate
    /// hash is not asserted by this test (we just need a valid hex
    /// 64-char string).
    pub(super) fn build_model(p: GenParams) -> compiler::ccm_emitter::ConditionModel {
        assert!(p.features >= 2, "need at least two features");
        let cluster_size = p.cluster_size.max(1);
        let max_depth = p.max_depth.max(1);
        let mut rng = Rng::new(p.seed);

        let features: Vec<String> = (1..=p.features)
            .map(|i| format!("feat_{i:05}"))
            .collect();

        let mut edges: Vec<TreeEdge> = Vec::with_capacity(p.features as usize);
        let mut cluster_roots: Vec<u32> = Vec::new();
        let mut id: u32 = 1;
        while id <= p.features {
            let cluster_end = (id + cluster_size).min(p.features + 1);
            cluster_roots.push(id);
            let mut stack: Vec<(u32, u32)> = Vec::with_capacity(max_depth as usize + 1);
            stack.push((id, 0));
            let mut next_id = id + 1;
            while next_id < cluster_end {
                let (parent_id, depth) = *stack.last().unwrap();
                let go_deeper = if depth + 1 >= max_depth {
                    false
                } else if stack.len() == 1 {
                    true
                } else {
                    rng.gen_pct(60)
                };
                if go_deeper {
                    let relation = pick_relation(&mut rng, p.mandatory_pct);
                    edges.push(TreeEdge {
                        child: next_id,
                        parent: parent_id,
                        relation,
                    });
                    stack.push((next_id, depth + 1));
                    next_id += 1;
                } else if stack.len() > 1 {
                    stack.pop();
                } else {
                    let relation = pick_relation(&mut rng, p.mandatory_pct);
                    edges.push(TreeEdge {
                        child: next_id,
                        parent: parent_id,
                        relation,
                    });
                    stack.push((next_id, depth + 1));
                    next_id += 1;
                }
            }
            id = cluster_end;
        }

        let mut cross_tree: Vec<CrossTree> = Vec::new();
        let mut seen_pairs: HashSet<(u32, u32)> = HashSet::new();
        let mut emitted = 0u32;
        while emitted < p.requires {
            let a = rng.gen_range(1, p.features + 1);
            let b = rng.gen_range(1, p.features + 1);
            if a == b {
                continue;
            }
            if !seen_pairs.insert((a, b)) {
                continue;
            }
            cross_tree.push(CrossTree::Requires(a, b));
            emitted += 1;
        }
        emitted = 0;
        while emitted < p.excludes {
            let a = rng.gen_range(1, p.features + 1);
            let b = rng.gen_range(1, p.features + 1);
            if a == b {
                continue;
            }
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            if !seen_pairs.insert((lo, 100_000_000 + hi)) {
                continue;
            }
            cross_tree.push(CrossTree::Excludes(lo, hi));
            emitted += 1;
        }

        let mut clauses: Vec<String> = Vec::with_capacity(
            cluster_roots.len() + edges.len() * 2 + cross_tree.len(),
        );
        for &root_id in &cluster_roots {
            clauses.push(atom(&features[(root_id - 1) as usize]));
        }
        for edge in &edges {
            let child = &features[(edge.child - 1) as usize];
            let parent = &features[(edge.parent - 1) as usize];
            match edge.relation {
                Relation::Optional => {
                    clauses.push(implication(child, parent));
                }
                Relation::Mandatory => {
                    clauses.push(implication(parent, child));
                    clauses.push(implication(child, parent));
                }
            }
        }
        for c in &cross_tree {
            match c {
                CrossTree::Requires(a, b) => {
                    clauses.push(implication(
                        &features[(*a - 1) as usize],
                        &features[(*b - 1) as usize],
                    ));
                }
                CrossTree::Excludes(a, b) => {
                    clauses.push(format!(
                        "!({} && {})",
                        atom(&features[(*a - 1) as usize]),
                        atom(&features[(*b - 1) as usize]),
                    ));
                }
            }
        }

        // `bound_model_hash` must be a 64-char lowercase-hex digest
        // per `validate_hash`. The exact value is not asserted by
        // this test, so any deterministic 32-byte string works.
        let bound_model_hash = "44".repeat(32);

        compiler::ccm_emitter::ConditionModel::from_clauses(bound_model_hash, clauses)
    }
}
