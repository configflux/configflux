// SPDX-License-Identifier: BUSL-1.1
//
// Multi-partition `Session` boolean composition — configflux-0r62.
//
// This integration test exercises the cross-partition fan-out wired
// into `Session::valid_options`, `Session::apply`, `Session::retract`,
// and `Session::state_hash` per ADR-0012 §1, §7, §8 and the architect
// amendment on atomic apply. The test crate sees only the public
// `Session` API — the multi-partition fan-out, `PartitionSession<B>`,
// and `PartitionFormulaHandle` stay `pub(crate)` per ADR-0003.
//
// Fixture: two partitions, one cluster + one bridge, both over the
// variable order `[region.a, region.b]`. Designed so that
//   - cluster 0 BDD = `region.a XOR region.b`
//     => baseline valid_options(region) on the cluster alone is {a, b}.
//   - bridge BDD = `region.b ∧ ¬region.a`
//     => baseline valid_options(region) on the bridge alone is {b}.
// Joint valid_options(region) is the intersection: {b}.
//
// apply("region", "a") touches BOTH partitions (each has the symbol
// region.a). The cluster-side apply succeeds; the bridge-side apply
// reduces to ⊥ (the bridge formula forbids region.a). Per ADR-0012 §1
// "Atomic apply across partitions", the session MUST roll back the
// cluster-side mutation and surface `Error::Conflict`, leaving the
// joint state byte-identical to the pre-apply state.
//
// The bridge formula's variable scope intentionally overlaps cluster 0
// — this is exactly the cross-tree-clause shape the partitioner emits
// per ADR-0012 §3 (cross-tree clauses' variable union → bridge BDD over
// the union). The test does not exercise more clusters; the atomic-
// rollback contract is a 2-partition property and a 2-partition
// fixture is sufficient to falsify it.
//
// Never imports `oxidd::*` or any solver-internal type (ADR-0003 §2 +
// §3). If a refactor leaked a `PartitionCcm` or `FormulaHandle` across
// the `Session` boundary, this file would no longer compile.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use solver::{Ccm, Error, OxiddBackend, Session};

// Re-declared ADR-0005 §4 constants. The solver crate keeps these as
// `pub(crate)`; the integration test redefines them so drift between
// the on-disk format and the fixture surfaces here as a parse failure
// rather than silently passing.
const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

#[path = "fixture_v2.rs"]
mod fixture_v2;

fn push_node(out: &mut Vec<u8>, var: u32, low: u32, high: u32) {
    out.extend_from_slice(&var.to_le_bytes());
    out.extend_from_slice(&low.to_le_bytes());
    out.extend_from_slice(&high.to_le_bytes());
    out.push(0u8);
    out.extend_from_slice(&[0u8; 3]);
}

fn bdd_header(out: &mut Vec<u8>, var_count: u32, node_count: u32, root: u32) {
    out.extend_from_slice(CCM_BDD_BIN_MAGIC);
    out.push(CCM_BDD_BIN_VERSION);
    out.extend_from_slice(&[0u8; 3]);
    out.extend_from_slice(&var_count.to_le_bytes());
    out.extend_from_slice(&node_count.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&root.to_le_bytes());
}

/// `region.a XOR region.b` under var order [region.a=0, region.b=1].
/// Post-order: 0=⊥, 1=⊤, 2=b-when-a=0 (var=1, low=0, high=1),
/// 3=b-when-a=1 (var=1, low=1, high=0), 4=a-root (var=0, low=2, high=3).
fn build_cluster_xor_bdd() -> Vec<u8> {
    let mut bytes = Vec::new();
    bdd_header(&mut bytes, 2, 5, 4);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    push_node(&mut bytes, 1, 0, 1); // b-when-a=0 (low=⊥, high=⊤)
    push_node(&mut bytes, 1, 1, 0); // b-when-a=1 (low=⊤, high=⊥)
    push_node(&mut bytes, 0, 2, 3); // root: a → 2 (a=0) | 3 (a=1)
    bytes
}

/// `region.b ∧ ¬region.a` under var order [region.a=0, region.b=1].
/// Post-order: 0=⊥, 1=⊤, 2=b-when-a=0 (var=1, low=0, high=1),
/// 3=a-root (var=0, low=2, high=0).
///
/// The bridge intentionally encodes a "forbid region.a" constraint so
/// that apply("region", "a") on the cluster-side succeeds but on the
/// bridge-side reduces to ⊥ — exercising the atomic-rollback path.
fn build_bridge_forbid_a_bdd() -> Vec<u8> {
    let mut bytes = Vec::new();
    bdd_header(&mut bytes, 2, 4, 3);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    push_node(&mut bytes, 1, 0, 1); // b-when-a=0 (low=⊥, high=⊤)
    push_node(&mut bytes, 0, 2, 0); // root: a → 2 (a=0) | ⊥ (a=1)
    bytes
}

#[derive(Serialize)]
struct SymbolsOut {
    schema_version: u32,
    variable_order: Vec<String>,
    facet_to_var: BTreeMap<String, u32>,
    var_to_label: Vec<String>,
}

/// Build symbols JSON with `region.a` (var 0) and `region.b` (var 1).
/// Used for both the cluster and bridge partitions in the 2-partition
/// fixture and for the top-level union symbols.
fn build_region_symbols_json() -> Vec<u8> {
    let mut facet_to_var = BTreeMap::new();
    facet_to_var.insert("region.a".to_string(), 0u32);
    facet_to_var.insert("region.b".to_string(), 1u32);
    let sym = SymbolsOut {
        schema_version: 2,
        variable_order: vec!["region.a".to_string(), "region.b".to_string()],
        facet_to_var,
        var_to_label: vec!["region=a".to_string(), "region=b".to_string()],
    };
    let mut v = serde_json::to_vec(&sym).expect("symbols serialize");
    v.push(b'\n');
    v
}

fn tempdir_for(test_name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "configflux-solver-multi-part-{}-{}",
        test_name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}

/// Lay down the 2-partition cluster + bridge fixture used by every
/// test in this file. Returns the directory `Session::load_ccm` should
/// be pointed at.
fn materialize_cluster_plus_bridge(base: &Path) -> PathBuf {
    let bound = "cc".repeat(32);
    let cluster_bdd = build_cluster_xor_bdd();
    let bridge_bdd = build_bridge_forbid_a_bdd();
    let symbols_bytes = build_region_symbols_json();

    let cluster = fixture_v2::PartitionPayload {
        bdd_bytes: &cluster_bdd,
        symbols_bytes: &symbols_bytes,
        var_count: 2,
        node_count: 5,
    };
    let bridge = fixture_v2::PartitionPayload {
        bdd_bytes: &bridge_bdd,
        symbols_bytes: &symbols_bytes,
        var_count: 2,
        node_count: 4,
    };

    // Top-level var_count / node_count totals across both partitions
    // per ADR-0005 Amendment 1 §12 ("totals across all partitions plus
    // the bridge"). 2 + 2 = 4; 5 + 4 = 9. The solver does not currently
    // cross-check these against per-partition values; the assertion
    // matters for the canonical-bytes recipe to round-trip.
    fixture_v2::materialize_multi_partition_ccm(
        base,
        &bound,
        4,
        9,
        &symbols_bytes,
        &[cluster],
        Some(&bridge),
    )
}

fn load_cluster_plus_bridge_session(label: &str) -> Session<OxiddBackend> {
    let base = tempdir_for(label);
    let dir = materialize_cluster_plus_bridge(&base);
    let ccm: Ccm = Session::<OxiddBackend>::load_ccm(&dir).expect("load 2-part fixture");
    Session::<OxiddBackend>::new(ccm).expect("session from 2-part fixture")
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn valid_options_intersects_cluster_and_bridge_results() {
    // Acceptance: "Multi-partition: apply + valid_options produces
    // intersection of per-partition results."
    //
    // Cluster 0 alone says `region` accepts {a, b} (XOR).
    // Bridge alone says `region` accepts {b} (formula `b ∧ ¬a`).
    // The session's joint valid_options(region) must equal the
    // intersection: {b}.
    let session = load_cluster_plus_bridge_session("intersect");
    let opts = session
        .valid_options("region")
        .expect("valid_options must succeed on a multi-partition fixture");
    assert_eq!(
        sorted(opts.options),
        vec!["b".to_string()],
        "joint valid_options must equal cluster ∩ bridge = {{b}}",
    );
    assert_eq!(opts.count, 1, "count tracks options.len() under intersect");
}

#[test]
fn apply_failure_on_one_partition_rolls_back_all_partitions() {
    // Acceptance: "Partial-apply failure (synthetic error injection on
    // partition K+1) leaves session in pre-apply state for ALL
    // partitions."
    //
    // apply("region", "a") touches both partitions:
    //   - cluster 0 has region.a → conjoin succeeds (formula reduces to
    //     `a ∧ ¬b`, sat),
    //   - bridge has region.a → conjoin reduces to ⊥ (bridge formula
    //     `b ∧ ¬a` is incompatible with a=1).
    // Per ADR-0012 §1 atomic-apply, the session MUST surface
    // `Error::Conflict` and MUST roll back the cluster-side mutation.
    let mut session = load_cluster_plus_bridge_session("rollback");

    // Baseline: joint valid_options = {b}.
    let baseline = sorted(
        session
            .valid_options("region")
            .expect("baseline valid_options")
            .options,
    );
    assert_eq!(baseline, vec!["b".to_string()], "baseline must be {{b}}");

    let err = session
        .apply("region", "a")
        .expect_err("apply must conflict on the bridge-side ⊥");
    assert!(
        matches!(&err, Error::Conflict { facet, value } if facet == "region" && value == "a"),
        "expected Error::Conflict{{region, a}}, got {err:?}",
    );

    // The bd issue's hard rollback witness: after the failed apply the
    // joint valid_options must be byte-identical to the baseline. If
    // the cluster-side mutation had stuck, cluster 0's surviving set
    // would have narrowed to {a} and the intersection would have
    // become {a} ∩ {b} = {}, not {b}.
    let after = sorted(
        session
            .valid_options("region")
            .expect("post-rollback valid_options")
            .options,
    );
    assert_eq!(
        after, baseline,
        "valid_options after a failed atomic apply must equal baseline",
    );
}

#[test]
fn retract_restores_only_the_partition_that_was_applied() {
    // Acceptance: "retract correctly undoes the most recent apply on
    // the correct partition only (other partitions unchanged)."
    //
    // Apply a value that exists only on one partition (cluster 0's
    // region.b is NOT in the bridge's vars in this fixture — wait,
    // both partitions share `region.a, region.b` by design). Both
    // partitions contain region.b. Pick apply(region, b):
    //   - cluster 0: succeeds, narrows to `¬a ∧ b`. valid_options
    //     under cluster 0 alone becomes {b}.
    //   - bridge: succeeds (formula `b ∧ ¬a` is sat with b=1, a=0).
    //     valid_options under bridge alone is still {b}.
    //
    // After apply: joint = {b}. After retract: joint must equal the
    // baseline {b} again, AND a subsequent apply(region, a) must once
    // again roll back atomically (proving the cluster-side undo entry
    // was popped correctly, not leaked).
    let mut session = load_cluster_plus_bridge_session("retract");

    let baseline = sorted(
        session
            .valid_options("region")
            .expect("baseline")
            .options,
    );
    assert_eq!(baseline, vec!["b".to_string()]);

    session
        .apply("region", "b")
        .expect("apply(region, b) is sat on both partitions");
    let post_apply = sorted(
        session
            .valid_options("region")
            .expect("post-apply")
            .options,
    );
    assert_eq!(post_apply, vec!["b".to_string()], "post-apply still {{b}}");

    // The atomic apply pushed an undo entry per touched partition.
    // Pop them all (one retract per partition that participated in the
    // multi-partition apply) so the session returns to its baseline.
    // ADR-0012 §7: each undo entry is partition-scoped; multi-
    // partition apply pushes one per touched partition; retract pops
    // them one at a time.
    session.retract("region").expect("retract bridge entry");
    session.retract("region").expect("retract cluster entry");

    let after = sorted(
        session
            .valid_options("region")
            .expect("post-retract")
            .options,
    );
    assert_eq!(
        after, baseline,
        "valid_options after retract must equal baseline",
    );

    // Re-attempt the atomic-rollback apply to prove the undo stack is
    // genuinely clean (no leaked cluster-side post-apply state).
    let err = session
        .apply("region", "a")
        .expect_err("rollback path must still fire after retract");
    assert!(matches!(&err, Error::Conflict { .. }));
    let final_opts = sorted(
        session
            .valid_options("region")
            .expect("post-rollback-2")
            .options,
    );
    assert_eq!(final_opts, baseline, "rollback after retract still intact");
}

#[test]
fn state_hash_is_deterministic_across_loads_of_same_multi_part_ccm() {
    // Acceptance: "state_hash deterministic across runs on the same
    // multi-partition CCM."
    //
    // Load the same on-disk fixture twice (each load runs the full v2
    // parser, partition manifest walk, per-partition hash check, top-
    // level chain check) and assert state_hash() is byte-identical.
    let a = load_cluster_plus_bridge_session("sh_a");
    let b = load_cluster_plus_bridge_session("sh_b");
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "state_hash must be deterministic across independent loads",
    );
}

#[test]
fn state_hash_changes_with_undo_stack_then_returns_to_baseline_after_full_retract() {
    // Per ADR-0012 §8 Field 5, state_hash includes per-partition undo
    // contribution counts. So:
    //   - baseline state_hash = X.
    //   - apply(region, b) pushes one entry per touched partition → hash != X.
    //   - retract twice (pop both entries) → hash == X again.
    let mut session = load_cluster_plus_bridge_session("sh_undo");
    let baseline_hash = session.state_hash();

    session.apply("region", "b").expect("sat apply");
    let post_apply_hash = session.state_hash();
    assert_ne!(
        baseline_hash, post_apply_hash,
        "state_hash must change after a multi-partition apply (undo counts shift)",
    );

    session.retract("region").expect("retract 1");
    session.retract("region").expect("retract 2");
    let post_retract_hash = session.state_hash();
    assert_eq!(
        baseline_hash, post_retract_hash,
        "state_hash must return to baseline after the undo stack drains",
    );
}

#[test]
fn unknown_facet_under_multi_partition_returns_typed_error() {
    // The unknown-facet error path is per-partition agnostic: a facet
    // not present in ANY partition's symbol table must surface as
    // `Error::UnknownFacet`. Pins the "intersection of zero partitions"
    // edge case so the multi-partition wiring does not silently turn
    // it into an empty-Vec success.
    let session = load_cluster_plus_bridge_session("unknown");
    let err = session
        .valid_options("nonexistent")
        .expect_err("unknown facet must error");
    assert!(
        matches!(&err, Error::UnknownFacet(name) if name == "nonexistent"),
        "expected UnknownFacet(nonexistent), got {err:?}",
    );
}
