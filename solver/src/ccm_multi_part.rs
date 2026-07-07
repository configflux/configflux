// SPDX-License-Identifier: BUSL-1.1
//
// `ccm_multi_part` — the v2 multi-part loader per ADR-0005 Amendment 1
// §11–§16 and ADR-0012 §4 §5.
//
// Under v2 every `<out>/ccm/` directory is multi-part. Even a
// single-partition model lays itself out as:
//
//   <out>/ccm/ccm.manifest.json          (top-level, includes `partition_manifest`)
//   <out>/ccm/ccm.symbols.json           (top-level union)
//   <out>/ccm/partition-manifest.json    (NEW; v2)
//   <out>/ccm/partition-0000/            (per-partition triple)
//       ccm.manifest.json
//       ccm.symbols.json
//       ccm.bdd.bin
//   <out>/ccm/partition-bridge/          (OPTIONAL; iff has_bridge)
//
// This module owns the cross-file dance: read the top-level manifest,
// open the partition manifest it points at, walk each named subdir
// through the existing per-partition parser in `ccm_format`, and verify
// the top-level hash chain.
//
// Per ADR-0003 §2 the loader is `pub(crate)`; the public surface is
// `Ccm` in `ccm.rs`. PartitionCcm and friends never leak.
//
// CUDD manager isolation (per the bd issue and ADR-0011 §3): each
// partition's BDD MUST end up in its own backend `new_session` so
// `DdNode` handles do not cross managers. This module does NOT
// instantiate backends — it just parses partition triples into
// `PartitionCcm` records. The `Session::new` constructor in
// `session.rs` builds one backend per `PartitionCcm` it sees.

use std::fs;
use std::path::Path;

use crate::ccm_format::{
    compute_top_level_ccm_hash, decode_hex32, load_ccm_from_dir, parse_manifest, BddPayload,
    Manifest, ParseError, Symbols, CCM_MANIFEST_SCHEMA_MAX, CCM_MANIFEST_SCHEMA_MIN,
    RECOGNIZED_ALGORITHMS,
};
use crate::partition_manifest::{PartitionManifest, PARTITION_MANIFEST_SCHEMA_MAX};

/// One partition's fully-parsed payload. Crate-private per ADR-0003 §2
/// — the multi-partition fan-out is internal; the public `Ccm` API is
/// unchanged from v1 by design (per the bd issue's hard constraint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartitionCcm {
    pub(crate) symbols: Symbols,
    pub(crate) bdd: BddPayload,
    /// The 32-byte ccm_hash this partition carries on disk. Verified
    /// against the recomputed hash during `load_multi_part` and again
    /// folded into the top-level chain check.
    pub(crate) ccm_hash: [u8; 32],
}

/// The full v2 multi-part payload — N cluster partitions plus an
/// optional bridge. Crate-private per ADR-0003 §2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultiPartCcm {
    /// Top-level `ccm_hash` (Snapshot.ccm_hash per ADR-0012 §5). Set
    /// from the top-level manifest's `ccm_hash` after the chain check
    /// in `load_multi_part` succeeds.
    pub(crate) top_level_ccm_hash: [u8; 32],
    /// Top-level `bound_model_hash` decoded from the manifest. Matches
    /// every per-partition `bound_model_hash`; the duplication is the
    /// v2 wire format's design (each per-partition manifest is a
    /// self-contained v2 manifest re-tagged from v1).
    pub(crate) bound_model_hash: [u8; 32],
    /// Top-level manifest schema version (currently always 2).
    pub(crate) schema_version: u32,
    /// One entry per cluster, in emission order.
    pub(crate) clusters: Vec<PartitionCcm>,
    /// Optional bridge partition, present iff `has_bridge` in the
    /// partition manifest.
    pub(crate) bridge: Option<PartitionCcm>,
}

impl MultiPartCcm {
    /// Convenience: number of partitions, bridge included.
    /// Used by `Ccm` accessors and lit up further in configflux-0r62
    /// (per-partition `Session` fan-out).
    #[allow(dead_code)]
    pub(crate) fn partition_count(&self) -> usize {
        self.clusters.len() + if self.bridge.is_some() { 1 } else { 0 }
    }
}

/// Load a v2 multi-part CCM from the directory `<out>/ccm/`.
///
/// Procedure:
///   1. Read top-level `ccm.manifest.json`. Validate schema, algorithm,
///      and that `partition_manifest` is present (per ADR-0005
///      Amendment 1 §12).
///   2. Read `partition-manifest.json` at the path the top-level
///      manifest names.
///   3. For each partition directory listed, load its triple via the
///      existing per-partition parser, verifying that partition's own
///      `ccm_hash` (ADR-0005 §5 Load 1-6) under the v2 domain tag.
///   4. If `has_bridge`, load `partition-bridge/` the same way.
///   5. Verify the top-level chain: recompute the top-level `ccm_hash`
///      per ADR-0012 §5 from (top manifest, partition manifest,
///      per-partition hashes in emission order, bridge hash last if
///      present). Compare against the top-level manifest's `ccm_hash`
///      AND the partition-manifest's `top_level_ccm_hash`; both must
///      agree.
pub(crate) fn load_multi_part(dir: &Path) -> Result<MultiPartCcm, ParseError> {
    // --- Step 1: top-level manifest ---
    let top_manifest_path = dir.join("ccm.manifest.json");
    let top_manifest_bytes = fs::read(&top_manifest_path).map_err(|e| {
        ParseError::Io(format!("read {}: {e}", top_manifest_path.display()))
    })?;
    let top_manifest = parse_manifest(&top_manifest_bytes)?;

    if top_manifest.schema_version < CCM_MANIFEST_SCHEMA_MIN
        || top_manifest.schema_version > CCM_MANIFEST_SCHEMA_MAX
    {
        return Err(ParseError::UnsupportedSchemaVersion);
    }
    if !RECOGNIZED_ALGORITHMS.contains(&top_manifest.algorithm.as_str()) {
        return Err(ParseError::UnknownAlgorithm);
    }
    let Some(pm_filename) = top_manifest.partition_manifest.as_deref() else {
        return Err(ParseError::ManifestParse(
            "top-level ccm.manifest.json missing required v2 field 'partition_manifest'".into(),
        ));
    };

    // --- Step 2: partition manifest ---
    let pm_path = dir.join(pm_filename);
    let pm_bytes = fs::read(&pm_path)
        .map_err(|e| ParseError::Io(format!("read {}: {e}", pm_path.display())))?;
    let pm: PartitionManifest = serde_json::from_slice(&pm_bytes).map_err(|e| {
        ParseError::ManifestParse(format!("partition-manifest.json: {e}"))
    })?;
    if pm.schema_version > PARTITION_MANIFEST_SCHEMA_MAX {
        return Err(ParseError::UnsupportedSchemaVersion);
    }
    // Sanity: `has_bridge` must agree with the last-entry-being-bridge
    // convention (ADR-0005 Amendment 1 §13). This catches a drift
    // between flag and list early.
    let last_is_bridge = pm
        .partitions
        .last()
        .map(|s| s.as_str() == "partition-bridge")
        .unwrap_or(false);
    if pm.has_bridge != last_is_bridge {
        return Err(ParseError::ManifestParse(
            "partition-manifest.has_bridge disagrees with last-entry name".into(),
        ));
    }
    if pm.partitions.is_empty() {
        return Err(ParseError::ManifestParse(
            "partition-manifest.partitions is empty (v2 always has at least 1)".into(),
        ));
    }

    // --- Step 3-4: load each partition ---
    let n = pm.partitions.len();
    let (cluster_names, bridge_name): (Vec<&str>, Option<&str>) = if pm.has_bridge {
        let clusters: Vec<&str> = pm.partitions[..n - 1].iter().map(String::as_str).collect();
        let bridge = pm.partitions[n - 1].as_str();
        (clusters, Some(bridge))
    } else {
        (pm.partitions.iter().map(String::as_str).collect(), None)
    };

    let mut clusters: Vec<PartitionCcm> = Vec::with_capacity(cluster_names.len());
    let mut per_partition_hashes: Vec<[u8; 32]> =
        Vec::with_capacity(pm.partitions.len());
    for (idx, name) in cluster_names.iter().enumerate() {
        let sub_dir = dir.join(name);
        let part = load_partition(&sub_dir, &top_manifest, idx)?;
        per_partition_hashes.push(part.ccm_hash);
        clusters.push(part);
    }
    let bridge: Option<PartitionCcm> = if let Some(bname) = bridge_name {
        let sub_dir = dir.join(bname);
        let b = load_partition(&sub_dir, &top_manifest, clusters.len())?;
        per_partition_hashes.push(b.ccm_hash);
        Some(b)
    } else {
        None
    };

    // --- Step 5: top-level hash chain check ---
    let computed_top = compute_top_level_ccm_hash(
        &top_manifest,
        pm.schema_version,
        &pm.partitions,
        pm.has_bridge,
        &per_partition_hashes,
    )?;

    let claimed_top = decode_hex32(&top_manifest.ccm_hash).ok_or_else(|| {
        ParseError::ManifestParse("top-level ccm_hash: not 64-char lowercase hex".into())
    })?;
    if computed_top != claimed_top {
        return Err(ParseError::HashMismatch);
    }
    let pm_top = decode_hex32(&pm.top_level_ccm_hash).ok_or_else(|| {
        ParseError::ManifestParse(
            "partition-manifest.top_level_ccm_hash: not 64-char lowercase hex".into(),
        )
    })?;
    if pm_top != claimed_top {
        // The partition manifest carries an independent copy of the
        // top-level hash specifically so a caller can verify the
        // artifact without parsing the top-level manifest. The two
        // values MUST match (ADR-0005 Amendment 1 §13 "carries the
        // same value as the top-level ccm.manifest.json.ccm_hash").
        return Err(ParseError::HashMismatch);
    }

    let bound_model_hash = decode_hex32(&top_manifest.bound_model_hash).ok_or_else(|| {
        ParseError::ManifestParse(
            "top-level bound_model_hash: not 64-char lowercase hex".into(),
        )
    })?;

    Ok(MultiPartCcm {
        top_level_ccm_hash: claimed_top,
        bound_model_hash,
        schema_version: top_manifest.schema_version,
        clusters,
        bridge,
    })
}

/// Load one partition subdirectory (cluster or bridge) into a
/// `PartitionCcm`. Each partition's `bound_model_hash` must agree with
/// the top-level's; mismatch means the partition came from a different
/// compile and the wire format guarantee is broken.
fn load_partition(
    sub_dir: &Path,
    top_manifest: &Manifest,
    partition_index: usize,
) -> Result<PartitionCcm, ParseError> {
    let parsed = load_ccm_from_dir(sub_dir)?;

    // Sanity: per-partition bound_model_hash must match top-level.
    // Without this check, an attacker who swapped one partition's
    // triple for a triple from a different model would pass the
    // per-partition hash check on its own.
    let top_bound = decode_hex32(&top_manifest.bound_model_hash).ok_or_else(|| {
        ParseError::ManifestParse(
            "top-level bound_model_hash invalid; per-partition cross-check skipped".into(),
        )
    })?;
    if parsed.bound_model_hash_bytes != top_bound {
        return Err(ParseError::ManifestParse(format!(
            "partition {partition_index}: bound_model_hash differs from top-level"
        )));
    }

    // Use the recomputed hash we already trust (the per-partition load
    // path re-derived it in `load_ccm_from_dir` and verified the
    // manifest's claimed value against the recompute).
    Ok(PartitionCcm {
        symbols: parsed.symbols,
        bdd: parsed.bdd,
        ccm_hash: parsed.ccm_hash_bytes,
    })
}

/// Public-ish helper for the `Ccm` constructor: detect whether a CCM
/// directory follows the v2 multi-part layout by looking for the
/// `partition-manifest.json` sentinel file. The v0.3.0 no-backcompat
/// policy means there is no v1 fallback — a directory that has
/// `ccm.manifest.json` but not `partition-manifest.json` is a malformed
/// v2 artifact and triggers a parse error downstream.
pub(crate) fn looks_like_multi_part_dir(dir: &Path) -> bool {
    dir.join("partition-manifest.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tempdir_for(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "configflux-mwyp-unit-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("mkdir tempdir");
        base
    }

    #[test]
    fn looks_like_multi_part_dir_returns_true_only_with_partition_manifest() {
        let base = tempdir_for("detect");
        let ccm = base.join("ccm");
        fs::create_dir_all(&ccm).unwrap();
        assert!(!looks_like_multi_part_dir(&ccm));
        fs::write(ccm.join("partition-manifest.json"), b"{}").unwrap();
        assert!(looks_like_multi_part_dir(&ccm));
    }

    #[test]
    fn load_multi_part_rejects_missing_top_manifest() {
        let base = tempdir_for("missing_top");
        let ccm = base.join("ccm");
        fs::create_dir_all(&ccm).unwrap();
        let err = load_multi_part(&ccm).unwrap_err();
        assert!(matches!(err, ParseError::Io(_)));
    }

    #[test]
    fn partition_count_reports_clusters_plus_bridge() {
        // PartitionCcm/MultiPartCcm are crate-private, but as the
        // owner of the type within the same crate we can construct a
        // synthetic instance for the count math.
        let empty_symbols = Symbols {
            schema_version: 2,
            variable_order: Vec::new(),
            facet_to_var: std::collections::BTreeMap::new(),
            var_to_label: Vec::new(),
        };
        let empty_bdd = BddPayload {
            var_count: 0,
            node_count: 0,
            roots: vec![0],
            nodes: Vec::new(),
        };
        let p = PartitionCcm {
            symbols: empty_symbols.clone(),
            bdd: empty_bdd.clone(),
            ccm_hash: [0u8; 32],
        };
        let single = MultiPartCcm {
            top_level_ccm_hash: [0u8; 32],
            bound_model_hash: [0u8; 32],
            schema_version: 2,
            clusters: vec![p.clone()],
            bridge: None,
        };
        assert_eq!(single.partition_count(), 1);
        let with_bridge = MultiPartCcm {
            top_level_ccm_hash: [0u8; 32],
            bound_model_hash: [0u8; 32],
            schema_version: 2,
            clusters: vec![p.clone(), p.clone()],
            bridge: Some(p),
        };
        assert_eq!(with_bridge.partition_count(), 3);
    }
}
