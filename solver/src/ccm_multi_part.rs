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

use std::path::{Component, Path};

use crate::ccm_format::{
    compute_top_level_ccm_hash, decode_hex32, load_ccm_from_dir, parse_manifest, read_ccm_file,
    BddPayload, ConstraintRosterEntry, Manifest, ParseError, Symbols, CCM_MANIFEST_SCHEMA_MAX,
    CCM_MANIFEST_SCHEMA_MIN, RECOGNIZED_ALGORITHMS,
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
    /// ADR-0054 §5.4 constraint roster, verbatim from the TOP-LEVEL
    /// manifest (per-partition manifests never carry it — `parse_manifest`
    /// rejects one that does). Empty when the model declares no constraint.
    pub(crate) constraints: Vec<ConstraintRosterEntry>,
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
    let top_manifest_bytes = read_ccm_file(&top_manifest_path)?;
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
    if !is_plain_filename(pm_filename) {
        return Err(ParseError::ManifestParse(format!(
            "top-level ccm.manifest.json field 'partition_manifest' must be a plain filename, got {pm_filename:?}"
        )));
    }
    let pm_path = dir.join(pm_filename);
    let pm_bytes = read_ccm_file(&pm_path)?;
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
    // Every entry becomes a directory name joined onto the model
    // directory in step 3-4 below, and those joins run BEFORE the
    // step-5 chain check, so the values are still unauthenticated when
    // they are used. The field is a bare `Vec<String>` off the same
    // untrusted `partition-manifest.json`, so it escapes exactly the way
    // the sibling `partition_manifest` field did — absolute replaces the
    // base, `..` traverses (configflux-w64h). Refuse the whole manifest
    // here rather than per-join, so no entry is reachable by either the
    // cluster loop or the bridge branch.
    for name in &pm.partitions {
        if !is_plain_filename(name) {
            return Err(ParseError::ManifestParse(format!(
                "partition-manifest.json field 'partitions' entry must be a plain filename, got {name:?}"
            )));
        }
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
        // ADR-0054 §5.4: the roster is model-global and top-level only. Moved
        // (not cloned) out of the manifest, which is dead after this point.
        constraints: top_manifest.constraints,
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

/// Is `name` a single plain filename — something that can only name a
/// file INSIDE the directory it is joined onto?
///
/// Both callers take their value out of the model directory's own
/// manifests — `partition_manifest` from `ccm.manifest.json`
/// (configflux-1m0g) and every `partitions` entry from
/// `partition-manifest.json` (configflux-w64h) — which a runtime-open
/// request points at by path and which are therefore untrusted input.
/// `Path::join` REPLACES the base when the joined value is absolute, so
/// `partition_manifest: "/dev/zero"` resolved to the device rather than
/// to a file under the model directory, and a `..` in either field
/// traversed out of it the same way. Neither gate ahead of the joins
/// constrains these fields: `resolve_ccm_dir` only requires the
/// top-level `ccm.manifest.json` to be a regular file, and
/// `looks_like_multi_part_dir` only requires a file literally named
/// `partition-manifest.json` to exist, which a decoy satisfies.
///
/// Accepted is exactly one `Component::Normal`: no root, no prefix, no
/// `.`, no `..`, non-empty. The separator check is explicit because a
/// backslash is not a separator on Unix, so `a\b` is one `Normal`
/// component here and must still be refused rather than being handed to
/// a reader on some other platform.
///
/// The emitter only ever writes `PARTITION_MANIFEST_FILENAME` and, for
/// the partition names, `partition-NNNN` plus `BRIDGE_PARTITION_DIR`
/// (`compiler/src/ccm_emitter/multi_part.rs`), so no artifact
/// ConfigFlux produces is affected by this.
fn is_plain_filename(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
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
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Per-process monotonic discriminator for temp-dir names (configflux-rvpb).
    /// `fetch_add` hands out a value at most once per process, so two names built
    /// from it can never be equal; `nanos` is a triage aid only.
    static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    /// The leaf is created with `create_dir`, not `create_dir_all`, so a
    /// residual collision fails loudly instead of silently sharing a tree.
    fn tempdir_for(label: &str) -> PathBuf {
        let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since_epoch| since_epoch.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!(
            "configflux-mwyp-unit-{label}-{}-{seq}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&base).expect("mkdir tempdir");
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
            constraints: Vec::new(),
            clusters: vec![p.clone()],
            bridge: None,
        };
        assert_eq!(single.partition_count(), 1);
        let with_bridge = MultiPartCcm {
            top_level_ccm_hash: [0u8; 32],
            bound_model_hash: [0u8; 32],
            schema_version: 2,
            constraints: Vec::new(),
            clusters: vec![p.clone(), p.clone()],
            bridge: Some(p),
        };
        assert_eq!(with_bridge.partition_count(), 3);
    }

    /// A top-level v2 `ccm.manifest.json` whose `partition_manifest`
    /// field carries `pm` verbatim. The two hashes are placeholders:
    /// every test built on this fixture is refused before the step-5
    /// chain check runs, so no hash math is needed to exercise the
    /// refusal (configflux-21g0, configflux-1m0g).
    fn top_manifest_json(pm: &str) -> String {
        let zero = "0".repeat(64);
        let pm_json = serde_json::to_string(pm).expect("encode partition_manifest");
        format!(
            r#"{{"algorithm":"robdd-cudd-v1","algorithm_params":{{}},"bound_model_hash":"{zero}","ccm_hash":"{zero}","construction_wall_time_us":0,"emitted_at":"1970-01-01T00:00:00Z","node_count":0,"partition_manifest":{pm_json},"schema_version":2,"var_count":0}}"#
        )
    }

    /// A single-cluster `partition-manifest.json`. Same placeholder-hash
    /// reasoning as `top_manifest_json`.
    fn partition_manifest_json() -> String {
        let zero = "0".repeat(64);
        format!(
            r#"{{"has_bridge":false,"partitions":["partition-0000"],"schema_version":2,"top_level_ccm_hash":"{zero}"}}"#
        )
    }

    /// A `<label>/ccm/` directory holding only the top-level manifest,
    /// with `partition_manifest` set to `pm`.
    fn ccm_dir_naming_partition_manifest(label: &str, pm: &str) -> PathBuf {
        let ccm = tempdir_for(label).join("ccm");
        fs::create_dir(&ccm).expect("mkdir ccm");
        fs::write(ccm.join("ccm.manifest.json"), top_manifest_json(pm))
            .expect("write top-level manifest");
        ccm
    }

    /// `/dev/zero` is the concrete unbounded-stream case these tests are
    /// about. A sandbox without it cannot exercise them, and a silent
    /// pass would be a coverage hole, so the skip is loud.
    fn dev_zero_is_a_device() -> bool {
        match fs::metadata("/dev/zero") {
            Ok(meta) => !meta.file_type().is_file(),
            Err(_) => false,
        }
    }

    #[test]
    fn load_multi_part_refuses_an_absolute_partition_manifest() {
        // `Path::join` REPLACES the base when the joined value is
        // absolute, so this value used to resolve to /dev/zero rather
        // than to a file inside the model directory — and was then read
        // unbounded. The VARIANT is the assertion that matters:
        // `ManifestParse` is reachable only if the value was refused
        // BEFORE the join, which is what proves the device was never
        // opened. An `Io` here would mean the loader went to it.
        let ccm = ccm_dir_naming_partition_manifest("pm_absolute", "/dev/zero");
        match load_multi_part(&ccm).unwrap_err() {
            ParseError::ManifestParse(msg) => {
                assert!(msg.contains("must be a plain filename"), "got {msg}");
                assert!(msg.contains("/dev/zero"), "the refusal names the value: {msg}");
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn load_multi_part_refuses_a_traversing_partition_manifest() {
        let ccm = ccm_dir_naming_partition_manifest("pm_traverse", "../x.json");
        match load_multi_part(&ccm).unwrap_err() {
            ParseError::ManifestParse(msg) => {
                assert!(msg.contains("must be a plain filename"), "got {msg}");
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn load_multi_part_refuses_an_empty_partition_manifest() {
        // An empty value joins to the directory itself, so the read
        // would land on a directory rather than on a manifest.
        let ccm = ccm_dir_naming_partition_manifest("pm_empty", "");
        match load_multi_part(&ccm).unwrap_err() {
            ParseError::ManifestParse(msg) => {
                assert!(msg.contains("must be a plain filename"), "got {msg}");
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn load_multi_part_refuses_a_fifo_top_manifest() {
        // `stat()` on a FIFO does not block, so the file-type refusal
        // fires without the path ever being opened. `open()` on a FIFO
        // with no writer blocks forever — which is exactly why the
        // timeout below is a FAILURE and not a flake: reaching it means
        // the loader opened the path before settling its type.
        let ccm = tempdir_for("fifo_top").join("ccm");
        fs::create_dir(&ccm).expect("mkdir ccm");
        let made = std::process::Command::new("mkfifo")
            .arg(ccm.join("ccm.manifest.json"))
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !made {
            eprintln!("[skip] mkfifo unavailable; the FIFO case did not run");
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let probe = ccm.clone();
        std::thread::spawn(move || {
            let _ = tx.send(load_multi_part(&probe).map(|_| ()));
        });
        let outcome = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("load_multi_part blocked: it opened the FIFO before checking its type");
        match outcome.expect_err("a FIFO is not a readable CCM manifest") {
            ParseError::Io(msg) => {
                assert!(msg.contains("not a regular file"), "got {msg}");
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn load_multi_part_refuses_a_non_regular_partition_file() {
        // The per-partition triple is the deepest read on the load path
        // and had no file-type check at all: a symlink to /dev/zero in
        // place of either payload file was read until the process ran
        // out of address space. `fs::metadata` FOLLOWS the symlink, so
        // the refusal here is about what the link points AT, not about
        // the link itself.
        if !dev_zero_is_a_device() {
            eprintln!("[skip] /dev/zero unavailable; the symlink case did not run");
            return;
        }
        // `load_ccm_from_dir` reads manifest, then symbols, then bdd,
        // all before it parses any of them, so each case only needs the
        // files ahead of it in that order to be regular.
        for (label, replaced) in [
            ("sym_symbols", "ccm.symbols.json"),
            ("sym_bdd", "ccm.bdd.bin"),
        ] {
            let ccm = tempdir_for(label).join("ccm");
            fs::create_dir(&ccm).expect("mkdir ccm");
            fs::write(
                ccm.join("ccm.manifest.json"),
                top_manifest_json("partition-manifest.json"),
            )
            .expect("write top-level manifest");
            fs::write(ccm.join("partition-manifest.json"), partition_manifest_json())
                .expect("write partition manifest");
            let part = ccm.join("partition-0000");
            fs::create_dir(&part).expect("mkdir partition-0000");
            for name in ["ccm.manifest.json", "ccm.symbols.json", "ccm.bdd.bin"] {
                if name == replaced {
                    std::os::unix::fs::symlink("/dev/zero", part.join(name))
                        .expect("symlink /dev/zero");
                } else {
                    fs::write(part.join(name), b"{}").expect("write placeholder");
                }
            }

            match load_multi_part(&ccm).unwrap_err() {
                ParseError::Io(msg) => {
                    assert!(msg.contains("not a regular file"), "got {msg}");
                    assert!(msg.contains(replaced), "the refusal names the file: {msg}");
                }
                other => panic!("expected Io for {replaced}, got {other:?}"),
            }
        }
    }

    /// A `<label>/ccm/` directory whose `partition-manifest.json` carries
    /// `partitions` verbatim. Same placeholder-hash reasoning as
    /// `top_manifest_json`: every test built on this fixture is refused
    /// before the step-5 chain check runs (configflux-w64h).
    fn ccm_dir_naming_partitions(label: &str, partitions: &[&str]) -> PathBuf {
        let ccm = tempdir_for(label).join("ccm");
        fs::create_dir(&ccm).expect("mkdir ccm");
        fs::write(
            ccm.join("ccm.manifest.json"),
            top_manifest_json("partition-manifest.json"),
        )
        .expect("write top-level manifest");
        let zero = "0".repeat(64);
        let list = serde_json::to_string(partitions).expect("encode partitions");
        fs::write(
            ccm.join("partition-manifest.json"),
            format!(
                r#"{{"has_bridge":false,"partitions":{list},"schema_version":2,"top_level_ccm_hash":"{zero}"}}"#
            ),
        )
        .expect("write partition manifest");
        ccm
    }

    #[test]
    fn load_multi_part_refuses_an_absolute_partitions_entry() {
        // `Path::join` REPLACES the base when the joined value is
        // absolute, so this entry used to resolve to `/dev/zero` and the
        // loader went looking for a partition triple under a character
        // device instead of under the model directory. The VARIANT is
        // the assertion that matters: `ManifestParse` is reachable only
        // if the entry was refused BEFORE the join, which is what proves
        // the path outside the directory was never visited. An `Io` here
        // would mean the loader went to it.
        let ccm = ccm_dir_naming_partitions("parts_absolute", &["/dev/zero"]);
        match load_multi_part(&ccm).unwrap_err() {
            ParseError::ManifestParse(msg) => {
                assert!(msg.contains("must be a plain filename"), "got {msg}");
                assert!(msg.contains("/dev/zero"), "the refusal names the value: {msg}");
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn load_multi_part_refuses_a_traversing_partitions_entry() {
        // The decoy below is a complete partition triple one level
        // OUTSIDE the model directory, sitting exactly where this entry
        // traverses to. Reading it would fail on its CONTENT — a serde
        // message naming the manifest's fields — so the plain-filename
        // refusal arriving instead is what proves nothing outside the
        // model directory was read.
        let ccm = ccm_dir_naming_partitions("parts_traverse", &["../outside-partition"]);
        let decoy = ccm
            .parent()
            .expect("the ccm dir has a parent")
            .join("outside-partition");
        fs::create_dir(&decoy).expect("mkdir decoy partition");
        for name in ["ccm.manifest.json", "ccm.symbols.json", "ccm.bdd.bin"] {
            fs::write(decoy.join(name), b"{}").expect("write decoy file");
        }
        match load_multi_part(&ccm).unwrap_err() {
            ParseError::ManifestParse(msg) => {
                assert!(msg.contains("must be a plain filename"), "got {msg}");
                assert!(
                    msg.contains("../outside-partition"),
                    "the refusal names the value: {msg}"
                );
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn load_multi_part_refuses_an_empty_partitions_entry() {
        // An empty entry joins to the model directory itself, so the
        // loader would read the TOP-level manifest back as if it were a
        // partition's own.
        let ccm = ccm_dir_naming_partitions("parts_empty", &[""]);
        match load_multi_part(&ccm).unwrap_err() {
            ParseError::ManifestParse(msg) => {
                assert!(msg.contains("must be a plain filename"), "got {msg}");
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    /// The accept/reject table for `partition_manifest`. The emitter
    /// only ever writes `partition-manifest.json`, so the accepted set
    /// is deliberately just "one plain filename" (configflux-1m0g).
    #[test]
    fn plain_filename_accepts_only_a_single_normal_component() {
        assert!(is_plain_filename("partition-manifest.json"));
        assert!(is_plain_filename("x"));
        for rejected in [
            "",
            ".",
            "..",
            "../x.json",
            "../../etc/passwd",
            "/dev/zero",
            "/",
            "a/b",
            "sub/partition-manifest.json",
            "./partition-manifest.json",
            "a\\b",
            "..\\x.json",
        ] {
            assert!(
                !is_plain_filename(rejected),
                "{rejected:?} must not be accepted as a plain filename"
            );
        }
    }
}
