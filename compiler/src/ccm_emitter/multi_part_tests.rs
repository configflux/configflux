// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the v2 multi-part wire-format emitter
//! (configflux-vmlb, ADR-0012 §4 + ADR-0005 Amendment 1 §11–§16).
//!
//! Sibling-from-parent layout per `bdd.rs` + `tests.rs` and
//! `partitioner.rs` + `partitioner_tests.rs`. Heavier scenarios
//! (multi-cluster, bridge, determinism, aggregation) live in the
//! sibling `multi_part_more_tests.rs`; this file holds the
//! single-partition / contract-shape tests that exercise the v2
//! layout shape and required fields.

use super::*;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn tempdir_for(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "configflux-multi-part-{label}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}

pub(super) fn small_model() -> ConditionModel {
    ConditionModel {
        bound_model_hash: "11".repeat(32),
        clauses: vec!["a == 'on' && b == 'enabled'".to_string()],
    }
}

pub(super) fn six_variable_model() -> ConditionModel {
    // Two disjoint clause groups, each touching three variables:
    //   group A: (a, b, c) — clauses 0 and 1
    //   group B: (d, e, f) — clauses 2 and 3
    // No cross-tree edges → no bridge.
    ConditionModel {
        bound_model_hash: "33".repeat(32),
        clauses: vec![
            "a == 'on' && b == 'on'".to_string(),
            "b == 'on' && c == 'on'".to_string(),
            "d == 'on' && e == 'on'".to_string(),
            "e == 'on' && f == 'on'".to_string(),
        ],
    }
}

pub(super) fn cross_tree_model() -> ConditionModel {
    // Two disjoint groups + one cross-tree clause linking a and d.
    ConditionModel {
        bound_model_hash: "55".repeat(32),
        clauses: vec![
            "a == 'on' && b == 'on'".to_string(),
            "b == 'on' && c == 'on'".to_string(),
            "d == 'on' && e == 'on'".to_string(),
            "e == 'on' && f == 'on'".to_string(),
            "a == 'on' && d == 'on'".to_string(),
        ],
    }
}

pub(super) fn parse_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).expect("canonical JSON parses")
}

#[test]
fn single_partition_collapse_emits_one_partition_dir_with_full_v2_layout() {
    // Combined contract: top-level + partition-manifest + partition-0000
    // all present, no bridge, no extra partitions.
    let dir = tempdir_for("single_collapse").join("ccm");
    let model = small_model();
    emit_ccm_dir(&model, &dir).expect("emit single-partition v2");

    // Top-level files.
    assert!(dir.join("ccm.manifest.json").exists());
    assert!(dir.join("ccm.symbols.json").exists());
    assert!(dir.join("partition-manifest.json").exists());
    // Exactly one partition subdir.
    assert!(dir.join("partition-0000").exists());
    assert!(!dir.join("partition-0001").exists());
    assert!(!dir.join("partition-bridge").exists());
    // Per-partition triple inside partition-0000/.
    let p0 = dir.join("partition-0000");
    assert!(p0.join("ccm.manifest.json").exists());
    assert!(p0.join("ccm.symbols.json").exists());
    assert!(p0.join("ccm.bdd.bin").exists());
}

#[test]
fn partition_manifest_records_single_entry_no_bridge() {
    let dir = tempdir_for("pm_single_entry").join("ccm");
    emit_ccm_dir(&small_model(), &dir).expect("emit");

    let v = parse_json(&fs::read(dir.join("partition-manifest.json")).unwrap());
    assert_eq!(v["schema_version"], 2);
    assert_eq!(v["has_bridge"], false);
    let partitions = v["partitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(partitions, vec!["partition-0000"]);
    assert_eq!(v["top_level_ccm_hash"].as_str().unwrap().len(), 64);
}

#[test]
fn top_level_manifest_carries_partition_manifest_field() {
    let dir = tempdir_for("top_pm_field").join("ccm");
    emit_ccm_dir(&small_model(), &dir).expect("emit");

    let top = parse_json(&fs::read(dir.join("ccm.manifest.json")).unwrap());
    assert_eq!(top["schema_version"], 2);
    assert_eq!(
        top["partition_manifest"].as_str(),
        Some("partition-manifest.json")
    );
}

#[test]
fn per_partition_manifest_does_not_carry_partition_manifest_field() {
    // Per ADR-0005 Amendment 1 §12: the partition_manifest field
    // lives only on the top-level manifest.
    let dir = tempdir_for("per_part_no_pm_field").join("ccm");
    emit_ccm_dir(&small_model(), &dir).expect("emit");
    let per = parse_json(
        &fs::read(dir.join("partition-0000").join("ccm.manifest.json")).unwrap(),
    );
    assert_eq!(per["schema_version"], 2);
    assert!(
        per.get("partition_manifest").is_none(),
        "per-partition manifest must NOT carry partition_manifest field"
    );
}

#[test]
fn top_level_ccm_hash_duplicated_between_top_manifest_and_partition_manifest() {
    // Per ADR-0005 Amendment 1 §13: top-level ccm_hash and
    // partition_manifest.top_level_ccm_hash carry the same value.
    let dir = tempdir_for("hash_duplication").join("ccm");
    emit_ccm_dir(&small_model(), &dir).expect("emit");
    let top = parse_json(&fs::read(dir.join("ccm.manifest.json")).unwrap());
    let pm = parse_json(&fs::read(dir.join("partition-manifest.json")).unwrap());
    assert_eq!(top["ccm_hash"], pm["top_level_ccm_hash"]);
    let h = top["ccm_hash"].as_str().unwrap();
    assert_eq!(h.len(), 64);
    assert!(
        h.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "ccm_hash must be lowercase hex"
    );
    // The v2 chain must produce a non-zero hash — confirms the
    // partition-bridged top-level pre-image is non-trivial.
    assert_ne!(h, "0".repeat(64));
}

#[test]
fn schema_version_two_in_every_manifest() {
    let dir = tempdir_for("schema_two_everywhere").join("ccm");
    emit_ccm_dir(&small_model(), &dir).expect("emit");

    let top = parse_json(&fs::read(dir.join("ccm.manifest.json")).unwrap());
    let per = parse_json(
        &fs::read(dir.join("partition-0000").join("ccm.manifest.json")).unwrap(),
    );
    let pm = parse_json(&fs::read(dir.join("partition-manifest.json")).unwrap());
    let top_sym = parse_json(&fs::read(dir.join("ccm.symbols.json")).unwrap());
    let per_sym = parse_json(
        &fs::read(dir.join("partition-0000").join("ccm.symbols.json")).unwrap(),
    );

    assert_eq!(top["schema_version"], 2);
    assert_eq!(per["schema_version"], 2);
    assert_eq!(pm["schema_version"], 2);
    assert_eq!(top_sym["schema_version"], 2);
    assert_eq!(per_sym["schema_version"], 2);
}

#[test]
fn canonical_json_field_order_in_top_level_manifest() {
    // ADR-0005 §6: object keys must be lexicographic ascending.
    let dir = tempdir_for("canonical_order").join("ccm");
    emit_ccm_dir(&small_model(), &dir).expect("emit");
    let bytes = fs::read(dir.join("ccm.manifest.json")).expect("read");
    let s = std::str::from_utf8(&bytes).expect("utf-8");
    let expected_keys = [
        "algorithm",
        "algorithm_params",
        "bound_model_hash",
        "ccm_hash",
        "construction_wall_time_us",
        "emitted_at",
        "node_count",
        "partition_manifest",
        "schema_version",
        "var_count",
    ];
    let mut prev_idx: i32 = -1;
    for k in &expected_keys {
        let needle = format!("\"{}\":", k);
        let idx = s
            .find(&needle)
            .unwrap_or_else(|| panic!("missing expected key '{}' in top-level manifest", k))
            as i32;
        assert!(
            idx > prev_idx,
            "top-level keys not in lex ascending order at '{}'",
            k
        );
        prev_idx = idx;
    }
}
