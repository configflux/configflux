// SPDX-License-Identifier: BUSL-1.1
//
// Hand-crafted CCM fixture round-trip — configflux-8dm.2.
//
// This integration test builds a three-file CCM directory (per ADR-0005
// §§1-5) byte-for-byte in a tempdir, then exercises `Session::load_ccm`
// and `Session::new` over both NullBackend and OxiddBackend to prove:
//
//   1. The §4 flat-binary BDD format is reconstructed correctly into
//      oxidd (acceptance criterion "non-trivial BDD with >=2 variables
//      loads correctly").
//   2. The `symbols` map is populated and queryable via `Ccm::symbols`
//      (acceptance criterion "symbols map populated").
//   3. `bound_model_hash` is stored on the `Ccm` handle and flows into
//      `Session::state_hash` so that two sessions loaded from distinct
//      CCMs produce distinct state hashes (acceptance criterion
//      "bound_model_hash stored and accessible via state_hash").
//   4. A `ccm.manifest.json.schema_version` over the compile-time
//      maximum returns a typed `CcmError::UnsupportedSchemaVersion`,
//      not a panic (acceptance criterion "schema_version rejection").
//
// The fixture is a 2-variable formula: f(a, b) = a ∧ b. Its BDD reduces
// to two non-terminal nodes plus two terminals — small enough to be
// hand-written, large enough to exercise the post-order and variable
// lookup paths.
//
// This file never imports `oxidd::*` directly; that is an architectural
// invariant (ADR-0003 §2 "no oxidd types leak"). If a refactor pushed
// oxidd through the public API this file would stop compiling.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use solver::{
    BddNode, Ccm, CcmError, Error, NullBackend, OxiddBackend, Session, SolverBackend, StateHash,
};

// Re-declared constants from ADR-0005 §4 — the solver crate keeps these
// as `pub(crate)`, so the external test redefines them rather than
// importing. A drift between these values and `ccm_format.rs` would
// surface here as a fixture that fails to parse.
const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

/// Build a `ccm.bdd.bin` for the formula f(a, b) = a ∧ b.
///
/// Reduced BDD structure (assuming variable order [a=0, b=1]):
///
/// ```text
///   a
///  / \
/// ⊥   b
///    / \
///   ⊥   ⊤
/// ```
///
/// Encoded post-order:
///
///   - index 0: terminal FALSE
///   - index 1: terminal TRUE
///   - index 2: non-terminal { var=1 (b), low=0 (⊥), high=1 (⊤) }
///   - index 3: non-terminal { var=0 (a), low=0 (⊥), high=2 (b-node) }
///
/// `root_table[0] = 3`.
fn build_and_bdd_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    // Header
    bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
    bytes.push(CCM_BDD_BIN_VERSION);
    bytes.extend_from_slice(&[0u8; 3]); // reserved
    bytes.extend_from_slice(&2u32.to_le_bytes()); // var_count
    bytes.extend_from_slice(&4u32.to_le_bytes()); // node_count
    bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count
    // Root table: root[0] = 3 (the a-node).
    bytes.extend_from_slice(&3u32.to_le_bytes());
    // Node 0: terminal FALSE
    bytes.extend_from_slice(&TERMINAL_VAR_INDEX.to_le_bytes());
    bytes.extend_from_slice(&TERMINAL_FALSE.to_le_bytes());
    bytes.extend_from_slice(&TERMINAL_FALSE.to_le_bytes());
    bytes.push(0u8); // flags
    bytes.extend_from_slice(&[0u8; 3]); // pad
    // Node 1: terminal TRUE
    bytes.extend_from_slice(&TERMINAL_VAR_INDEX.to_le_bytes());
    bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
    bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
    bytes.push(0u8);
    bytes.extend_from_slice(&[0u8; 3]);
    // Node 2: b-node { var=1, low=0, high=1 }
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.push(0u8);
    bytes.extend_from_slice(&[0u8; 3]);
    // Node 3: a-node { var=0, low=0, high=2 }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.push(0u8);
    bytes.extend_from_slice(&[0u8; 3]);
    bytes
}

/// Build a `ccm.symbols.json` with two variables named "a" and "b".
fn build_symbols_json() -> Vec<u8> {
    let mut facet_to_var = BTreeMap::new();
    facet_to_var.insert("a".to_string(), 0u32);
    facet_to_var.insert("b".to_string(), 1u32);
    let sym = SymbolsOut {
        schema_version: 2,
        variable_order: vec!["a".to_string(), "b".to_string()],
        facet_to_var,
        var_to_label: vec!["A".to_string(), "B".to_string()],
    };
    let mut v = serde_json::to_vec(&sym).expect("symbols serialize");
    v.push(b'\n');
    v
}

#[derive(Serialize)]
struct SymbolsOut {
    schema_version: u32,
    variable_order: Vec<String>,
    facet_to_var: BTreeMap<String, u32>,
    var_to_label: Vec<String>,
}

// v2 multi-part CCM fixture builders are shared across the solver
// integration tests; see `fixture_v2.rs`.
#[path = "fixture_v2.rs"]
mod fixture_v2;

/// Build the v2 single-partition (N=1, no bridge) CCM directory for
/// `f(a, b) = a ∧ b`. The `schema_version` parameter overrides the
/// top-level manifest's schema; for the happy-path tests pass `2`. The
/// `load_ccm_rejects_future_manifest_schema_version` test uses `999`
/// to drive the schema-overflow reject path; in that case the
/// recomputed top-level hash will not match (the loader rejects the
/// schema first, so the test still asserts the schema variant).
fn materialize_and_ccm(
    base: &Path,
    bound_model_hash_hex: &str,
    schema_version: u32,
) -> PathBuf {
    let bdd_bytes = build_and_bdd_bin();
    let symbols_bytes = build_symbols_json();
    if schema_version == 2 {
        return fixture_v2::materialize_single_partition_ccm(
            base,
            &bdd_bytes,
            &symbols_bytes,
            bound_model_hash_hex,
            2,
            4,
        );
    }
    // Schema-overflow reject path: emit a top-level manifest with the
    // requested schema_version (e.g. 999). The hash chain will be
    // self-consistent at every level, but the solver rejects the
    // schema before the chain check. We piggy-back on the shared
    // fixture helper for the partition tree, then overwrite the
    // top-level `ccm.manifest.json` with a future-schema variant.
    let dir = fixture_v2::materialize_single_partition_ccm(
        base,
        &bdd_bytes,
        &symbols_bytes,
        bound_model_hash_hex,
        2,
        4,
    );
    write_future_schema_top_manifest(&dir, bound_model_hash_hex, schema_version, 2, 4);
    dir
}

/// Overwrite the top-level `ccm.manifest.json` with a future schema
/// version. The other files (partition manifest, per-partition triple)
/// stay at schema 2; the solver rejects the top-level schema first.
fn write_future_schema_top_manifest(
    dir: &Path,
    bound_model_hash_hex: &str,
    schema_version: u32,
    var_count: u32,
    node_count: u64,
) {
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
    // Any nonzero hex string is fine — the solver rejects the schema
    // before reaching the hash chain.
    let top = TopOut {
        algorithm: "robdd-handrolled-v1",
        algorithm_params: &params,
        bound_model_hash: bound_model_hash_hex,
        ccm_hash: &"f".repeat(64),
        construction_wall_time_us: 0,
        emitted_at: "1970-01-01T00:00:00Z",
        node_count,
        partition_manifest: "partition-manifest.json",
        schema_version,
        var_count,
    };
    let mut v = serde_json::to_vec(&top).expect("future schema manifest");
    v.push(b'\n');
    fs::write(dir.join("ccm.manifest.json"), v).expect("overwrite top manifest");
}

/// Create a unique tempdir under the system temp dir for one test
/// invocation. We avoid the `tempfile` crate because it is not in the
/// solver's Bazel dep set today; see `fixture_v2::unique_temp_dir` for why
/// the discriminator is an atomic counter and not the pid or the clock.
fn tempdir_for(test_name: &str) -> PathBuf {
    fixture_v2::unique_temp_dir("configflux-solver", test_name)
}

#[test]
fn load_ccm_reconstructs_two_variable_bdd_into_oxidd() {
    // Acceptance #1: hand-crafted ccm.bdd.bin with >=2 variables loads
    // into oxidd and produces a Session with a non-⊤, non-⊥ root.
    let base = tempdir_for("and2");
    let bound = "11".repeat(32); // 64-char lowercase hex, all 0x11
    let dir = materialize_and_ccm(&base, &bound, 2);

    let ccm: Ccm =
        Session::<OxiddBackend>::load_ccm(&dir).expect("load_ccm on AND-2 fixture must succeed");

    // Ccm must report the real (nonzero) hash and bound_model_hash.
    assert_ne!(ccm.ccm_hash(), [0u8; 32], "ccm_hash must be nonzero");
    assert_eq!(
        ccm.bound_model_hash(),
        [0x11u8; 32],
        "bound_model_hash must round-trip through the manifest"
    );
    // After configflux-mwyp / ADR-0012 the v2 multi-part loader is the
    // only accepted shape; the top-level manifest reports
    // schema_version = 2. `CCM_SCHEMA_VERSION_V1 == 1` is kept as the
    // empty-Ccm sentinel for the round-trip-empty contract.
    assert_eq!(ccm.schema_version(), 2);

    // Symbols must be populated and queryable.
    let symbols = ccm.symbols().expect("symbols must be populated");
    assert_eq!(symbols.var_count(), 2);
    assert_eq!(symbols.var_for_facet("a"), Some(0));
    assert_eq!(symbols.var_for_facet("b"), Some(1));
    assert_eq!(symbols.var_for_facet("nope"), None);

    // BDD payload must be populated with the expected shape.
    let bdd = ccm.bdd().expect("bdd must be populated");
    assert_eq!(bdd.var_count(), 2);
    assert_eq!(bdd.node_count(), 4);
    assert_eq!(bdd.primary_root(), 3);

    // Session::new must deserialize into oxidd without error.
    let session: Session<OxiddBackend> =
        Session::new(ccm).expect("Session::new must succeed on AND-2 fixture");

    // The reconstructed root is neither ⊤ nor ⊥ — it is the formula
    // `a ∧ b`, which is contingent.
    let current = session.current();
    assert!(!session.backend().is_true(current), "AND-2 is not ⊤");
    assert!(!session.backend().is_false(current), "AND-2 is not ⊥");
}

#[test]
fn load_ccm_state_hash_incorporates_bound_model_hash() {
    // Acceptance #3: bound_model_hash flows into state_hash so that two
    // sessions loaded from distinct CCMs — differing only by
    // bound_model_hash — produce distinct state hashes. This is what
    // pins "bound_model_hash is stored and accessible via state_hash".
    let base_a = tempdir_for("sh_a");
    let base_b = tempdir_for("sh_b");
    let dir_a = materialize_and_ccm(&base_a, &"11".repeat(32), 2);
    let dir_b = materialize_and_ccm(&base_b, &"22".repeat(32), 2);

    let ccm_a = Session::<OxiddBackend>::load_ccm(&dir_a).expect("load a");
    let ccm_b = Session::<OxiddBackend>::load_ccm(&dir_b).expect("load b");

    // The two Ccms have different bound_model_hash by construction but
    // the same BDD + symbols bytes, so their ccm_hash values also
    // differ (bound_model_hash is in the pre-image).
    assert_ne!(ccm_a.ccm_hash(), ccm_b.ccm_hash());
    assert_ne!(ccm_a.bound_model_hash(), ccm_b.bound_model_hash());

    let session_a = Session::<OxiddBackend>::new(ccm_a).expect("session a");
    let session_b = Session::<OxiddBackend>::new(ccm_b).expect("session b");

    let hash_a: StateHash = session_a.state_hash();
    let hash_b: StateHash = session_b.state_hash();
    assert_ne!(
        hash_a, hash_b,
        "state_hash must differ when bound_model_hash differs"
    );
    assert_ne!(
        hash_a,
        StateHash::zero(),
        "state_hash must be nonzero for a real-Ccm session"
    );
}

#[test]
fn load_ccm_rejects_future_manifest_schema_version() {
    // Acceptance #4: schema_version higher than CCM_MANIFEST_SCHEMA_MAX
    // must return a typed Err (not panic) per ADR-0005 §7 "loud
    // rejection on version overflow".
    let base = tempdir_for("schema");
    let dir = materialize_and_ccm(&base, &"33".repeat(32), 999);

    let err: Error = Session::<OxiddBackend>::load_ccm(&dir)
        .expect_err("schema_version = 999 must be rejected");
    assert!(
        matches!(err, Error::Ccm(CcmError::UnsupportedSchemaVersion)),
        "schema overflow must map to CcmError::UnsupportedSchemaVersion, got {err:?}"
    );
}

#[test]
fn load_ccm_rejects_corrupted_ccm_hash() {
    // Red-path: if we write a manifest whose claimed ccm_hash does not
    // match the recomputed digest, the loader must refuse.
    let base = tempdir_for("ccmhash");
    let dir = materialize_and_ccm(&base, &"44".repeat(32), 2);

    // Poison the ccm_hash field by overwriting with 64 zeros.
    let manifest_path = dir.join("ccm.manifest.json");
    let original = fs::read_to_string(&manifest_path).expect("read manifest");
    let original_hash = find_ccm_hash_value(&original);
    assert_ne!(original_hash, "0".repeat(64), "starting hash must be nonzero");
    let corrupted = original.replace(&original_hash, &"00".repeat(32));
    fs::write(&manifest_path, corrupted).expect("write corrupted manifest");

    let err =
        Session::<OxiddBackend>::load_ccm(&dir).expect_err("corrupted ccm_hash must be rejected");
    assert!(
        matches!(err, Error::Ccm(CcmError::CcmHashMismatch)),
        "expected CcmHashMismatch, got {err:?}"
    );
}

/// Extract the 64-char hex value of the `ccm_hash` field from a
/// compact-JSON manifest string. The fixture above emits compact JSON
/// so the literal `"ccm_hash":"..."` substring is always present.
fn find_ccm_hash_value(manifest: &str) -> String {
    let needle = "\"ccm_hash\":\"";
    let start = manifest.find(needle).expect("ccm_hash field present") + needle.len();
    let end = manifest[start..].find('"').expect("closing quote") + start;
    manifest[start..end].to_string()
}

#[test]
fn bdd_node_re_export_is_stable() {
    // The `BddNode` type is part of the public API because
    // `SolverBackend::deserialize_bdd` takes `&[BddNode]`. Pinning its
    // field layout here catches any accidental rename of the public
    // fields that would otherwise silently break downstream backends.
    let n = BddNode {
        var_index: 7,
        low_id: 0,
        high_id: 1,
        flags: 0,
    };
    assert_eq!(n.var_index, 7);
    assert_eq!(n.low_id, 0);
    assert_eq!(n.high_id, 1);
    assert_eq!(n.flags, 0);
}

#[test]
fn null_backend_rejects_non_trivial_root_via_default_deserialize_bdd() {
    // NullBackend inherits the trait's default `deserialize_bdd` impl
    // (see `backend.rs`), which only handles ⊤ / ⊥ sentinels. For a
    // real 2-variable BDD the root is a non-sentinel index, so the
    // default impl must return a `Serialization` backend error that
    // Session::new surfaces as Error::Backend.
    //
    // This test pins the behavior so a future refactor that silently
    // drops the trait's default impl surfaces as a failure here
    // rather than a wrong result.
    let base = tempdir_for("null_deser");
    let dir = materialize_and_ccm(&base, &"55".repeat(32), 2);
    let ccm = Session::<NullBackend>::load_ccm(&dir).expect("load_ccm with null backend");
    let err = Session::<NullBackend>::new(ccm)
        .expect_err("null backend must not reconstruct a variable-valued BDD");
    assert!(
        matches!(err, Error::Backend(_)),
        "expected Error::Backend from null deserialize_bdd, got {err:?}"
    );
}
