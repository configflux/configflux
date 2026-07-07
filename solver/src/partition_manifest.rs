// SPDX-License-Identifier: BUSL-1.1
//
// `partition_manifest` — reader for the v2 `partition-manifest.json`
// file introduced by ADR-0005 Amendment 1 §13 / ADR-0012 §4.
//
// The partition manifest is the entry-point index that the v2 multi-part
// loader walks: it names the per-partition subdirectories, flags whether
// a bridge partition is present, and carries the top-level `ccm_hash`
// that the solver verifies against the top-level `ccm.manifest.json`'s
// `ccm_hash` field (ADR-0005 Amendment 1 §13, ADR-0012 §5 Step 5).
//
// Per ADR-0003 §1 the on-disk layout is implementation detail; this
// module is `pub(crate)` and its parsed `PartitionManifest` value never
// leaves the solver crate's internals. The public `Ccm` surface in
// `ccm.rs` is the only thing downstream consumers see.

use serde::Deserialize;

/// Maximum partition-manifest schema version this build accepts. Per
/// ADR-0005 Amendment 1 §13 the schema starts at 2 (the file did not
/// exist in v1). The solver rejects any value higher than this constant
/// with the existing `BackendError::UnsupportedCcmSchema` mapping
/// (`CcmError::UnsupportedSchemaVersion`).
pub(crate) const PARTITION_MANIFEST_SCHEMA_MAX: u32 = 2;

/// On-disk layout of `partition-manifest.json` per ADR-0005 Amendment 1 §13.
///
/// Every field is required: ADR-0005 §6 canonical JSON forbids optional
/// fields because their absence is a byte-stability hazard. A missing
/// field is a parse error, not a silent default.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PartitionManifest {
    /// Schema counter for this file, evolving independently from the
    /// top-level / per-partition `ccm.manifest.json.schema_version`.
    pub schema_version: u32,
    /// Directory names in emission order: `"partition-0000"` first,
    /// ascending through every cluster, then `"partition-bridge"` last
    /// iff `has_bridge` is `true`.
    pub partitions: Vec<String>,
    /// `true` iff the last entry in `partitions` is the bridge
    /// partition. Provided redundantly with `partitions` so the loader
    /// can dispatch without string-matching on the last element.
    pub has_bridge: bool,
    /// Lowercase hex SHA-256 of the multi-part artifact as a whole per
    /// ADR-0012 §5. Carries the same value as the top-level
    /// `ccm.manifest.json.ccm_hash` field — the duplication is
    /// intentional so the partition manifest is independently
    /// verifiable.
    pub top_level_ccm_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_json(bridge: bool, hash: &str) -> String {
        let parts = if bridge {
            r#"["partition-0000","partition-bridge"]"#
        } else {
            r#"["partition-0000"]"#
        };
        format!(
            r#"{{"has_bridge":{bridge},"partitions":{parts},"schema_version":2,"top_level_ccm_hash":"{hash}"}}"#,
        )
    }

    #[test]
    fn parse_single_partition_no_bridge() {
        let h = "0".repeat(64);
        let json = good_json(false, &h);
        let pm: PartitionManifest = serde_json::from_str(&json).expect("parse");
        assert_eq!(pm.schema_version, 2);
        assert_eq!(pm.partitions, vec!["partition-0000"]);
        assert!(!pm.has_bridge);
        assert_eq!(pm.top_level_ccm_hash, h);
    }

    #[test]
    fn parse_multi_with_bridge() {
        let h = "a".repeat(64);
        let json = good_json(true, &h);
        let pm: PartitionManifest = serde_json::from_str(&json).expect("parse");
        assert!(pm.has_bridge);
        assert_eq!(pm.partitions.last().map(String::as_str), Some("partition-bridge"));
    }

    #[test]
    fn parse_rejects_missing_field() {
        // Missing `top_level_ccm_hash` must be a parse error, not a
        // silent default. ADR-0005 §6: no optional fields under
        // canonical JSON.
        let bad = r#"{"has_bridge":false,"partitions":["partition-0000"],"schema_version":2}"#;
        let err = serde_json::from_str::<PartitionManifest>(bad).unwrap_err();
        assert!(err.to_string().contains("top_level_ccm_hash"));
    }

    #[test]
    fn schema_max_matches_amendment() {
        // Pin against accidental relax. ADR-0005 Amendment 1 §13: the
        // partition-manifest starts at schema 2.
        assert_eq!(PARTITION_MANIFEST_SCHEMA_MAX, 2);
    }
}
