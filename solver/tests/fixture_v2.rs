// SPDX-License-Identifier: BUSL-1.1
//
// Shared v2 multi-part CCM fixture builders for the solver integration
// tests under `solver/tests/`. configflux-mwyp.
//
// Each integration test (`load_ccm_real_fixture`, `valid_options_bdd`,
// `apply_retract`) hand-rolls a `(symbols, bdd)` pair appropriate to
// its scenario, then calls into this module to wrap that pair in the
// v2 multi-part wire format defined by ADR-0005 Amendment 1 §11–§16
// and ADR-0012 §4–§5.
//
// This file is NOT a test by itself; it is `#[path]`-included from each
// test crate via `#[path = "fixture_v2.rs"] mod fixture_v2;`. Bazel
// wires it into each test's `srcs` alongside the test file.
//
// The hand-rolled ADR-0005 §4 BDD layout constants stay in each test
// file because they double as drift-detection: if the on-disk format
// ever rotates, every test's local copy of the magic / version / etc.
// must be updated, which forces reviewers to look at every fixture.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Per-process monotonic discriminator for temp-dir names.
///
/// This is the uniqueness primitive: `fetch_add` hands out a value at most
/// once per process, so two names built from it can never be equal. Each
/// including crate gets its own copy of this static, which is fine — the
/// crates are separate test binaries and `pid` separates those.
static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Create a temp dir that cannot collide with any other path this process
/// hands out, and return it.
///
/// Determinism (configflux-q5rr, configflux-rvpb): the solver tests' old name
/// was `<prefix>-<label>-<pid>`, with no clock component at all, so two tests
/// reaching one helper with one label got the *same* directory every time —
/// unique only because each caller happened to pass a distinct label. The
/// atomic `seq` removes the race by construction; `nanos` is a triage aid in
/// the path name and carries no uniqueness guarantee, so a degenerate clock
/// degrades readability rather than correctness.
///
/// The leaf is created with `create_dir` rather than `create_dir_all` so the
/// no-collision invariant is *enforced*, not merely reasoned about: if a name
/// were ever handed out twice the second create would fail loudly instead of
/// silently sharing a directory with another test. For the same reason the
/// callers' old `remove_dir_all` pre-wipe is gone — it was the collision
/// *amplifier*, deleting a concurrent test's tree rather than reporting the
/// clash.
pub fn unique_temp_dir(prefix: &str, label: &str) -> PathBuf {
    let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!(
        "{}-{}-{}-{}-{}",
        prefix,
        label,
        std::process::id(),
        seq,
        nanos
    ));
    if let Some(parent) = base.parent() {
        fs::create_dir_all(parent).expect("create temp dir parent");
    }
    fs::create_dir(&base).expect("mkdir tempdir");
    base
}

/// Encode a 32-byte digest as lowercase hex. Hand-rolled to avoid a
/// `hex` crate dep in the test crates.
pub fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &b in bytes.iter() {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Compute the per-partition `ccm_hash` for a v2 manifest. The compiler
/// emitter's `serialize_per_partition_manifest` produces the same bytes
/// (the canonical lexicographic key ordering is identical), and the
/// solver's `compute_ccm_hash` recomputes them at load time.
pub fn per_partition_hash(
    symbols_bytes: &[u8],
    bdd_bytes: &[u8],
    bound_model_hash_hex: &str,
    var_count: u32,
    node_count: u64,
) -> [u8; 32] {
    #[derive(Serialize)]
    struct PreImage<'a> {
        algorithm: &'a str,
        algorithm_params: &'a BTreeMap<String, String>,
        bound_model_hash: &'a str,
        node_count: u64,
        schema_version: u32,
        var_count: u32,
    }
    let params: BTreeMap<String, String> = BTreeMap::new();
    let pre = PreImage {
        algorithm: "robdd-handrolled-v1",
        algorithm_params: &params,
        bound_model_hash: bound_model_hash_hex,
        node_count,
        schema_version: 2,
        var_count,
    };
    let canon = serde_json::to_vec(&pre).expect("preimage serialize");
    let bdd_digest: [u8; 32] = Sha256::digest(bdd_bytes).into();
    let symbols_digest: [u8; 32] = Sha256::digest(symbols_bytes).into();
    let mut hasher = Sha256::new();
    hasher.update(b"configflux.ccm.v2\n");
    hasher.update(&canon);
    hasher.update(b"\n");
    hasher.update(&bdd_digest);
    hasher.update(&symbols_digest);
    hasher.finalize().into()
}

/// Serialize the v2 per-partition `ccm.manifest.json` carrying the
/// already-computed `ccm_hash`. Does NOT include the v2 top-level
/// `partition_manifest` field (per ADR-0005 Amendment 1 §12).
pub fn build_per_partition_manifest_json(
    bound_model_hash_hex: &str,
    var_count: u32,
    node_count: u64,
    ccm_hash_hex: &str,
) -> Vec<u8> {
    #[derive(Serialize)]
    struct FullManifest<'a> {
        algorithm: &'a str,
        algorithm_params: &'a BTreeMap<String, String>,
        bound_model_hash: &'a str,
        ccm_hash: &'a str,
        construction_wall_time_us: u64,
        emitted_at: &'a str,
        node_count: u64,
        schema_version: u32,
        var_count: u32,
    }
    let params: BTreeMap<String, String> = BTreeMap::new();
    let full = FullManifest {
        algorithm: "robdd-handrolled-v1",
        algorithm_params: &params,
        bound_model_hash: bound_model_hash_hex,
        ccm_hash: ccm_hash_hex,
        construction_wall_time_us: 0,
        emitted_at: "1970-01-01T00:00:00Z",
        node_count,
        schema_version: 2,
        var_count,
    };
    let mut v = serde_json::to_vec(&full).expect("per-partition manifest");
    v.push(b'\n');
    v
}

/// Compute the top-level `ccm_hash` per ADR-0012 §5 Step 3-4.
pub fn top_level_hash(
    bound_model_hash_hex: &str,
    var_count: u32,
    node_count: u64,
    partitions: &[String],
    has_bridge: bool,
    per_partition_hashes: &[[u8; 32]],
) -> [u8; 32] {
    #[derive(Serialize)]
    struct TopPreImage<'a> {
        algorithm: &'a str,
        algorithm_params: &'a BTreeMap<String, String>,
        bound_model_hash: &'a str,
        node_count: u64,
        partition_manifest: &'a str,
        schema_version: u32,
        var_count: u32,
    }
    let params: BTreeMap<String, String> = BTreeMap::new();
    let top_pre = TopPreImage {
        algorithm: "robdd-handrolled-v1",
        algorithm_params: &params,
        bound_model_hash: bound_model_hash_hex,
        node_count,
        partition_manifest: "partition-manifest.json",
        schema_version: 2,
        var_count,
    };
    let top_canon = serde_json::to_vec(&top_pre).expect("top preimage");
    #[derive(Serialize)]
    struct PmPreImage<'a> {
        has_bridge: bool,
        partitions: &'a [String],
        schema_version: u32,
    }
    let pm_pre = PmPreImage {
        has_bridge,
        partitions,
        schema_version: 2,
    };
    let pm_canon = serde_json::to_vec(&pm_pre).expect("pm preimage");
    let mut hasher = Sha256::new();
    hasher.update(b"configflux.ccm.v2\n");
    hasher.update(&top_canon);
    hasher.update(b"\n");
    hasher.update(&pm_canon);
    hasher.update(b"\n");
    for h in per_partition_hashes {
        hasher.update(h);
    }
    hasher.finalize().into()
}

/// Serialize the top-level v2 `ccm.manifest.json` carrying the
/// `partition_manifest` field (the v2-only required key per ADR-0005
/// Amendment 1 §12).
pub fn build_top_level_manifest_json(
    bound_model_hash_hex: &str,
    var_count: u32,
    node_count: u64,
    top_hash_hex: &str,
) -> Vec<u8> {
    #[derive(Serialize)]
    struct TopOut<'a> {
        algorithm: &'a str,
        algorithm_params: &'a BTreeMap<String, String>,
        bound_model_hash: &'a str,
        ccm_hash: &'a str,
        construction_wall_time_us: u64,
        emitted_at: &'a str,
        node_count: u64,
        partition_manifest: &'a str,
        schema_version: u32,
        var_count: u32,
    }
    let params: BTreeMap<String, String> = BTreeMap::new();
    let top = TopOut {
        algorithm: "robdd-handrolled-v1",
        algorithm_params: &params,
        bound_model_hash: bound_model_hash_hex,
        ccm_hash: top_hash_hex,
        construction_wall_time_us: 0,
        emitted_at: "1970-01-01T00:00:00Z",
        node_count,
        partition_manifest: "partition-manifest.json",
        schema_version: 2,
        var_count,
    };
    let mut v = serde_json::to_vec(&top).expect("top manifest");
    v.push(b'\n');
    v
}

/// Serialize a `partition-manifest.json` carrying the
/// `top_level_ccm_hash` per ADR-0005 Amendment 1 §13.
pub fn build_partition_manifest_json(
    partitions: &[String],
    has_bridge: bool,
    top_hash_hex: &str,
) -> Vec<u8> {
    #[derive(Serialize)]
    struct PmOut<'a> {
        has_bridge: bool,
        partitions: &'a [String],
        schema_version: u32,
        top_level_ccm_hash: &'a str,
    }
    let pm = PmOut {
        has_bridge,
        partitions,
        schema_version: 2,
        top_level_ccm_hash: top_hash_hex,
    };
    let mut v = serde_json::to_vec(&pm).expect("partition manifest");
    v.push(b'\n');
    v
}

/// Lay down a single-partition (N=1, no bridge) v2 multi-part CCM
/// directory carrying the supplied BDD + symbols payload. Returns the
/// directory path that callers pass to `Session::load_ccm`.
pub fn materialize_single_partition_ccm(
    base: &Path,
    bdd_bytes: &[u8],
    symbols_bytes: &[u8],
    bound_model_hash_hex: &str,
    var_count: u32,
    node_count: u64,
) -> PathBuf {
    let pp_hash = per_partition_hash(
        symbols_bytes,
        bdd_bytes,
        bound_model_hash_hex,
        var_count,
        node_count,
    );
    let pp_hash_hex = hex32(&pp_hash);
    let pp_manifest = build_per_partition_manifest_json(
        bound_model_hash_hex,
        var_count,
        node_count,
        &pp_hash_hex,
    );

    let partitions = vec!["partition-0000".to_string()];
    let top_hash = top_level_hash(
        bound_model_hash_hex,
        var_count,
        node_count,
        &partitions,
        false,
        &[pp_hash],
    );
    let top_hash_hex = hex32(&top_hash);
    let top_manifest = build_top_level_manifest_json(
        bound_model_hash_hex,
        var_count,
        node_count,
        &top_hash_hex,
    );
    let pm = build_partition_manifest_json(&partitions, false, &top_hash_hex);

    let dir = base.join("ccm");
    fs::create_dir_all(&dir).expect("mkdir ccm/");
    fs::write(dir.join("ccm.manifest.json"), &top_manifest).expect("write top manifest");
    fs::write(dir.join("ccm.symbols.json"), symbols_bytes).expect("write top symbols");
    fs::write(dir.join("partition-manifest.json"), &pm).expect("write partition-manifest");

    let p0 = dir.join("partition-0000");
    fs::create_dir_all(&p0).expect("mkdir partition-0000");
    fs::write(p0.join("ccm.bdd.bin"), bdd_bytes).expect("write p0 bdd.bin");
    fs::write(p0.join("ccm.symbols.json"), symbols_bytes).expect("write p0 symbols");
    fs::write(p0.join("ccm.manifest.json"), &pp_manifest).expect("write p0 manifest");
    dir
}

/// One partition's payload, shared by both cluster and bridge entries
/// when assembling a multi-partition v2 fixture. Each partition carries
/// its own (symbols, bdd) byte-pair and is opened by the solver into
/// its own CUDD/oxidd manager.
pub struct PartitionPayload<'a> {
    pub bdd_bytes: &'a [u8],
    pub symbols_bytes: &'a [u8],
    pub var_count: u32,
    pub node_count: u64,
}

/// Lay down a multi-partition (N clusters plus optional bridge) v2 CCM
/// directory. The bridge entry, when present, is always written last
/// per ADR-0012 §4 (the partition manifest lists it as
/// "partition-bridge" at the tail of `partitions`).
///
/// `top_union_symbols_bytes` is the top-level `ccm.symbols.json` payload
/// (the union of per-partition symbol indices per ADR-0005 §4
/// Amendment); fixtures may pass any well-formed v2 symbols JSON
/// because the solver does not currently cross-check it against the
/// per-partition ones. The `var_count` / `node_count` parameters here
/// are the top-level *totals* per ADR-0005 Amendment 1 §12.
pub fn materialize_multi_partition_ccm(
    base: &Path,
    bound_model_hash_hex: &str,
    top_var_count: u32,
    top_node_count: u64,
    top_union_symbols_bytes: &[u8],
    clusters: &[PartitionPayload<'_>],
    bridge: Option<&PartitionPayload<'_>>,
) -> PathBuf {
    assert!(
        !clusters.is_empty(),
        "v2 always has at least one cluster partition",
    );
    let dir = base.join("ccm");
    fs::create_dir_all(&dir).expect("mkdir ccm/");

    let mut partition_names: Vec<String> = Vec::with_capacity(clusters.len() + 1);
    let mut per_partition_hashes: Vec<[u8; 32]> = Vec::with_capacity(clusters.len() + 1);

    // Cluster partitions, named `partition-NNNN` ascending.
    for (idx, p) in clusters.iter().enumerate() {
        let name = format!("partition-{idx:04}");
        let pp_hash = per_partition_hash(
            p.symbols_bytes,
            p.bdd_bytes,
            bound_model_hash_hex,
            p.var_count,
            p.node_count,
        );
        let pp_hex = hex32(&pp_hash);
        let pp_manifest = build_per_partition_manifest_json(
            bound_model_hash_hex,
            p.var_count,
            p.node_count,
            &pp_hex,
        );
        let sub = dir.join(&name);
        fs::create_dir_all(&sub).expect("mkdir partition-NNNN");
        fs::write(sub.join("ccm.bdd.bin"), p.bdd_bytes).expect("write cluster bdd.bin");
        fs::write(sub.join("ccm.symbols.json"), p.symbols_bytes)
            .expect("write cluster symbols");
        fs::write(sub.join("ccm.manifest.json"), &pp_manifest)
            .expect("write cluster manifest");

        partition_names.push(name);
        per_partition_hashes.push(pp_hash);
    }

    // Bridge partition, name `partition-bridge`, written last.
    let has_bridge = bridge.is_some();
    if let Some(b) = bridge {
        let name = "partition-bridge".to_string();
        let pp_hash = per_partition_hash(
            b.symbols_bytes,
            b.bdd_bytes,
            bound_model_hash_hex,
            b.var_count,
            b.node_count,
        );
        let pp_hex = hex32(&pp_hash);
        let pp_manifest = build_per_partition_manifest_json(
            bound_model_hash_hex,
            b.var_count,
            b.node_count,
            &pp_hex,
        );
        let sub = dir.join(&name);
        fs::create_dir_all(&sub).expect("mkdir partition-bridge");
        fs::write(sub.join("ccm.bdd.bin"), b.bdd_bytes).expect("write bridge bdd.bin");
        fs::write(sub.join("ccm.symbols.json"), b.symbols_bytes)
            .expect("write bridge symbols");
        fs::write(sub.join("ccm.manifest.json"), &pp_manifest)
            .expect("write bridge manifest");

        partition_names.push(name);
        per_partition_hashes.push(pp_hash);
    }

    // Top-level chain.
    let top_hash = top_level_hash(
        bound_model_hash_hex,
        top_var_count,
        top_node_count,
        &partition_names,
        has_bridge,
        &per_partition_hashes,
    );
    let top_hex = hex32(&top_hash);
    let top_manifest = build_top_level_manifest_json(
        bound_model_hash_hex,
        top_var_count,
        top_node_count,
        &top_hex,
    );
    let pm = build_partition_manifest_json(&partition_names, has_bridge, &top_hex);

    fs::write(dir.join("ccm.manifest.json"), &top_manifest).expect("write top manifest");
    fs::write(dir.join("ccm.symbols.json"), top_union_symbols_bytes)
        .expect("write top symbols");
    fs::write(dir.join("partition-manifest.json"), &pm).expect("write partition-manifest");

    dir
}
