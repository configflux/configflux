// SPDX-License-Identifier: BUSL-1.1

//! configflux-pq2w.1 / ADR-0044 D1 (amended): the product compile path must
//! emit a deterministic, NON-hashed `provenance.json` sidecar next to each of
//! the two file-writing artifact sets — the CMP directory
//! (`<out>/provenance.json`) and its sibling CCM directory
//! (`<out>/ccm/provenance.json`).
//!
//! Black-box, driven entirely through the public product API. The invariants
//! pinned here are the acceptance bar for OBS-1:
//!  * both sidecars exist and carry the real workspace `tool_version`
//!    (from `/VERSION`, never the Bazel `0.0.0` default);
//!  * the recorded `artifacts` hashes recompute to the primary artifacts'
//!    actual SHA-256s;
//!  * WITHOUT `--stamp-time` the sidecars are BYTE-IDENTICAL across two runs
//!    (no wall-clock leaks in), as are the primary artifacts;
//!  * WITH `--stamp-time` only a `stamped_at` field appears — the primary
//!    artifacts stay byte-identical (identity never enters a hash preimage).

use std::fs;
use std::path::{Path, PathBuf};

use compiler::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use compiler::provenance_sidecar::{hash_file, tool_version};

const DEFS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const COMPONENTS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

fn compile_into(dir: &Path, stamp_time: bool) -> compiler::product_api::CompileResult {
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![
            SourceManifestEntry {
                source_id: "scenarios/s1/00_definitions.json".to_string(),
                inline_content: DEFS.to_string(),
            },
            SourceManifestEntry {
                source_id: "scenarios/s1/10_components.json".to_string(),
                inline_content: COMPONENTS.to_string(),
            },
        ],
        output_dir: Some(dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "compile must succeed: {:?}",
        result.verify_report
    );
    result
}

#[test]
fn compile_emits_provenance_sidecars_with_real_version() {
    let dir = tempdir_for("prov-basic");
    let result = compile_into(&dir, false);

    // The envelope carries the additive `tool_version`.
    assert_eq!(
        result.tool_version.as_deref(),
        Some(tool_version()),
        "CompileResult.tool_version must be the workspace version"
    );
    assert_ne!(tool_version(), "0.0.0", "version must come from /VERSION");

    let cmp_sidecar = dir.join("provenance.json");
    let ccm_sidecar = dir.join("ccm").join("provenance.json");
    assert!(cmp_sidecar.exists(), "CMP provenance.json must exist");
    assert!(ccm_sidecar.exists(), "CCM provenance.json must exist");

    // CMP sidecar records the manifest's actual content hash.
    let cmp: serde_json::Value =
        serde_json::from_slice(&fs::read(&cmp_sidecar).unwrap()).unwrap();
    assert_eq!(cmp["tool"], "configflux-compiler");
    assert_eq!(cmp["tool_version"], tool_version());
    assert!(cmp.get("stamped_at").is_none(), "no stamp without --stamp-time");
    let recorded_cmp = cmp["artifacts"]["cmp.manifest.json"].as_str().unwrap();
    assert_eq!(
        recorded_cmp,
        hash_file(&dir.join("cmp.manifest.json")).unwrap(),
        "recorded CMP hash must match the actual manifest bytes"
    );

    // CCM sidecar records the top-level ccm artifacts' actual hashes.
    let ccm: serde_json::Value =
        serde_json::from_slice(&fs::read(&ccm_sidecar).unwrap()).unwrap();
    assert_eq!(ccm["tool_version"], tool_version());
    let recorded_ccm = ccm["artifacts"]["ccm.manifest.json"].as_str().unwrap();
    assert_eq!(
        recorded_ccm,
        hash_file(&dir.join("ccm").join("ccm.manifest.json")).unwrap(),
        "recorded CCM hash must match the actual manifest bytes"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn sidecars_and_artifacts_are_byte_stable_without_stamp_time() {
    let a = tempdir_for("prov-det-a");
    let b = tempdir_for("prov-det-b");
    compile_into(&a, false);
    compile_into(&b, false);

    for rel in [
        "provenance.json",
        "ccm/provenance.json",
        "cmp.manifest.json",
        "ccm/ccm.manifest.json",
    ] {
        let ba = fs::read(a.join(rel)).unwrap();
        let bb = fs::read(b.join(rel)).unwrap();
        assert_eq!(ba, bb, "'{rel}' must be byte-identical across runs");
    }

    fs::remove_dir_all(&a).ok();
    fs::remove_dir_all(&b).ok();
}

#[test]
fn stamp_time_adds_only_stamped_at_and_leaves_artifacts_unchanged() {
    let plain = tempdir_for("prov-plain");
    let stamped = tempdir_for("prov-stamped");
    compile_into(&plain, false);
    compile_into(&stamped, true);

    // Primary artifacts are byte-identical regardless of the stamp flag —
    // the timestamp never enters a hash preimage.
    for rel in ["cmp.manifest.json", "ccm/ccm.manifest.json"] {
        assert_eq!(
            fs::read(plain.join(rel)).unwrap(),
            fs::read(stamped.join(rel)).unwrap(),
            "'{rel}' must not depend on --stamp-time"
        );
    }

    // The stamped sidecar carries a stamped_at; the plain one does not; every
    // other field (tool, tool_version, schema_versions, artifacts) matches.
    let plain_json: serde_json::Value =
        serde_json::from_slice(&fs::read(plain.join("provenance.json")).unwrap()).unwrap();
    let mut stamped_json: serde_json::Value =
        serde_json::from_slice(&fs::read(stamped.join("provenance.json")).unwrap()).unwrap();
    assert!(plain_json.get("stamped_at").is_none());
    assert!(
        stamped_json["stamped_at"].as_str().unwrap().ends_with('Z'),
        "stamped_at must be an RFC3339 UTC instant"
    );
    // Drop the stamp and the two records must be identical.
    stamped_json.as_object_mut().unwrap().remove("stamped_at");
    assert_eq!(
        plain_json, stamped_json,
        "only stamped_at may differ between plain and stamped sidecars"
    );

    fs::remove_dir_all(&plain).ok();
    fs::remove_dir_all(&stamped).ok();
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
