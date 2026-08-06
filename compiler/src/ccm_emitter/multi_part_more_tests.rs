// SPDX-License-Identifier: BUSL-1.1

//! Additional unit tests for the v2 multi-part wire-format emitter
//! (configflux-vmlb). Sibling of `multi_part_tests.rs`; covers
//! multi-cluster splits, bridge emission, determinism, aggregation,
//! and the top-level symbols union. Split out per the 400-line
//! sibling-file convention (matches `bdd.rs` + `tests.rs` and
//! `partitioner.rs` + `partitioner_tests.rs`).

use super::multi_part_tests::{
    cross_tree_model, parse_json, six_variable_model, small_model, tempdir_for,
};
use super::*;
use std::fs;

#[test]
fn six_variable_model_with_cluster_size_three_emits_two_partitions_no_bridge() {
    let dir = tempdir_for("six_var_two_parts").join("ccm");
    let model = six_variable_model();
    emit_ccm_dir_with_cluster_size(
        &model,
        &dir,
        "facet-name-ascending",
        "in-crate",
        3,
    )
    .expect("emit multi-partition");

    let pm = parse_json(&fs::read(dir.join("partition-manifest.json")).unwrap());
    let partitions = pm["partitions"]
        .as_array()
        .expect("partitions")
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    // Disjoint groups (a,b,c) and (d,e,f) with cap=3 → exactly 2 clusters.
    assert_eq!(partitions, vec!["partition-0000", "partition-0001"]);
    assert_eq!(pm["has_bridge"], false);
    assert!(dir.join("partition-0000").is_dir());
    assert!(dir.join("partition-0001").is_dir());
    assert!(!dir.join("partition-bridge").exists());
}

#[test]
fn cross_tree_clause_produces_bridge_partition() {
    let dir = tempdir_for("cross_tree_bridge").join("ccm");
    let model = cross_tree_model();
    emit_ccm_dir_with_cluster_size(
        &model,
        &dir,
        "facet-name-ascending",
        "in-crate",
        3,
    )
    .expect("emit cross-tree");

    let pm = parse_json(&fs::read(dir.join("partition-manifest.json")).unwrap());
    assert_eq!(pm["has_bridge"], true);
    let partitions = pm["partitions"]
        .as_array()
        .expect("partitions")
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        partitions.last().expect("non-empty"),
        "partition-bridge",
        "bridge must be the last entry in the partitions list"
    );
    assert!(dir.join("partition-bridge").is_dir());
    assert!(dir.join("partition-bridge").join("ccm.manifest.json").exists());
    assert!(dir.join("partition-bridge").join("ccm.symbols.json").exists());
    assert!(dir.join("partition-bridge").join("ccm.bdd.bin").exists());
}

#[test]
fn emit_is_deterministic_across_repeated_runs() {
    // Same model + same heuristic + same construction must produce
    // bit-identical bytes on every file across two independent emits
    // (ADR-0005 G1 byte-stability).
    let dir_a = tempdir_for("det_a").join("ccm");
    let dir_b = tempdir_for("det_b").join("ccm");
    let model = six_variable_model();

    emit_ccm_dir_with_cluster_size(&model, &dir_a, "facet-name-ascending", "in-crate", 3)
        .expect("emit a");
    emit_ccm_dir_with_cluster_size(&model, &dir_b, "facet-name-ascending", "in-crate", 3)
        .expect("emit b");

    for relpath in &[
        "ccm.manifest.json",
        "ccm.symbols.json",
        "partition-manifest.json",
        "partition-0000/ccm.manifest.json",
        "partition-0000/ccm.symbols.json",
        "partition-0000/ccm.bdd.bin",
        "partition-0001/ccm.manifest.json",
        "partition-0001/ccm.symbols.json",
        "partition-0001/ccm.bdd.bin",
    ] {
        let a = fs::read(dir_a.join(relpath)).expect("read a");
        let b = fs::read(dir_b.join(relpath)).expect("read b");
        assert_eq!(a, b, "{relpath} differs across deterministic emits");
    }
}

#[test]
fn top_level_ccm_hash_changes_when_input_changes() {
    let dir_a = tempdir_for("hash_input_a").join("ccm");
    let dir_b = tempdir_for("hash_input_b").join("ccm");

    emit_ccm_dir(&small_model(), &dir_a).expect("emit a");
    let model_b = ConditionModel::from_clauses(
        "22".repeat(32),
        vec!["x == 'on' && y == 'on'".to_string()],
    );
    emit_ccm_dir(&model_b, &dir_b).expect("emit b");

    let h_a = parse_json(&fs::read(dir_a.join("ccm.manifest.json")).unwrap())["ccm_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let h_b = parse_json(&fs::read(dir_b.join("ccm.manifest.json")).unwrap())["ccm_hash"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(h_a, h_b, "different inputs must produce different ccm_hash");
}

#[test]
fn per_partition_var_and_node_counts_aggregate_to_top_level() {
    let dir = tempdir_for("aggregates").join("ccm");
    let model = six_variable_model();
    emit_ccm_dir_with_cluster_size(&model, &dir, "facet-name-ascending", "in-crate", 3)
        .expect("emit");

    let top = parse_json(&fs::read(dir.join("ccm.manifest.json")).unwrap());
    let p0 = parse_json(
        &fs::read(dir.join("partition-0000").join("ccm.manifest.json")).unwrap(),
    );
    let p1 = parse_json(
        &fs::read(dir.join("partition-0001").join("ccm.manifest.json")).unwrap(),
    );

    let top_var = top["var_count"].as_u64().unwrap();
    let v0 = p0["var_count"].as_u64().unwrap();
    let v1 = p1["var_count"].as_u64().unwrap();
    assert_eq!(top_var, v0 + v1, "var_count must aggregate across partitions");

    let top_nodes = top["node_count"].as_u64().unwrap();
    let n0 = p0["node_count"].as_u64().unwrap();
    let n1 = p1["node_count"].as_u64().unwrap();
    assert_eq!(top_nodes, n0 + n1, "node_count must aggregate across partitions");
}

#[test]
fn top_level_symbols_unions_per_partition_symbols() {
    let dir = tempdir_for("top_symbols_union").join("ccm");
    let model = six_variable_model();
    emit_ccm_dir_with_cluster_size(&model, &dir, "facet-name-ascending", "in-crate", 3)
        .expect("emit");

    let top_sym = parse_json(&fs::read(dir.join("ccm.symbols.json")).unwrap());
    let order = top_sym["variable_order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(order.len(), 6);
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted, "top-level variable_order must be sorted");
}
