// SPDX-License-Identifier: BUSL-1.1

//! configflux-9hi2: the product compile path (`compile_model` ->
//! `Compiler::emit_ir`) must emit the v2 multi-part `.ccm` directory
//! alongside the CMP package, `ModelHandle` must surface its path, and the
//! emitted `.ccm` must be bound to the same `model_hash` as the CMP package
//! (ADR-0005 §9 `bound_model_hash` linkage).
//!
//! This is a black-box regression test driven entirely through the public
//! product/loader API plus the public solver loader: it never reaches into
//! `ccm_emitter` internals. The key signal is that the solver can construct a
//! NON-empty `Ccm` from the directory the product compile emitted — i.e. the
//! `g3f.2` blocker (`load_ccm` falling through to `Ccm::empty()`) is cleared.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use compiler::loader_api::{open_model, OpenModelRequest};
use compiler::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use solver::{OxiddBackend, Session, SolverBackend};

/// The `s1_water_pump/smoke` fixture carries three component/override
/// `condition`s, so the compiled model lowers to a non-trivial BDD with
/// several boolean facet variables — exactly what we need to prove the
/// emitted `.ccm` is non-empty.
const DEFS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const COMPONENTS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

#[test]
fn product_compile_emits_loadable_ccm_bound_to_model_hash() {
    let output_dir = tempdir_for("product-compile-ccm");

    // (1) Drive the REAL product compile path.
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
        output_dir: Some(output_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "compile must succeed: {:?}",
        result.verify_report
    );
    let model_hash = result.model_hash.clone();

    // The CMP package is still emitted exactly as before.
    let cmp_manifest_ref = result
        .compiled_model_package_ref
        .clone()
        .expect("cmp manifest ref present");
    assert!(Path::new(&cmp_manifest_ref).exists(), "CMP manifest exists");

    // (a) The v2 multi-part `.ccm/` directory was emitted alongside the CMP.
    let ccm_dir = output_dir.join("ccm");
    assert!(
        ccm_dir.join("ccm.manifest.json").exists(),
        "top-level ccm.manifest.json must exist at {}",
        ccm_dir.display()
    );
    assert!(
        ccm_dir.join("partition-manifest.json").exists(),
        "v2 multi-part sentinel partition-manifest.json must exist"
    );

    // (a)+key-signal: the solver constructs a NON-empty Ccm from the emitted
    // directory (the g3f.2 blocker: load_ccm previously fell through to
    // Ccm::empty()).
    let ccm = Session::<OxiddBackend>::load_ccm(&ccm_dir)
        .expect("solver loads the product-emitted .ccm dir");
    let symbols = ccm
        .symbols()
        .expect("emitted .ccm carries a symbol table (non-empty Ccm)");
    assert!(
        symbols.var_count() > 0,
        "the compiled smoke model has conditions, so the BDD must have variables"
    );
    assert_ne!(
        ccm.bound_model_hash(),
        [0u8; 32],
        "a real load must carry a non-zero bound_model_hash, not the empty stub"
    );

    // The loaded Ccm must actually deserialize into a usable session.
    let session = Session::<OxiddBackend>::new(ccm).expect("deserialize emitted BDD into a session");
    let current = session.current();
    assert!(
        !session.backend().is_false(current),
        "compiled smoke model must be satisfiable (root != FALSE)"
    );

    // (c) ADR-0005 §9: the .ccm's bound_model_hash byte-equals the CMP model_hash.
    let expected_bytes = decode_hex32(&model_hash).expect("model_hash is 64-char hex");
    // Re-load to assert the linkage on a fresh handle (the prior `ccm` was
    // consumed by `Session::new`).
    let ccm_for_link = Session::<OxiddBackend>::load_ccm(&ccm_dir).expect("reload .ccm");
    assert_eq!(
        ccm_for_link.bound_model_hash(),
        expected_bytes,
        "bound_model_hash must byte-equal the CMP model_hash (ADR-0005 §9)"
    );

    // (b) ModelHandle surfaces the .ccm path.
    let open_result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref,
    });
    assert_eq!(open_result.status, OperationStatus::Ok, "open_model ok");
    let handle = open_result.model_handle.expect("model handle present");
    assert_eq!(handle.model_hash, model_hash, "handle model_hash matches");
    assert!(
        !handle.ccm_ref.is_empty(),
        "ModelHandle must surface a non-empty ccm_ref"
    );
    assert_eq!(
        PathBuf::from(&handle.ccm_ref),
        ccm_dir,
        "handle.ccm_ref points at the sibling ccm/ directory"
    );
    assert!(
        Path::new(&handle.ccm_ref).is_dir(),
        "handle.ccm_ref resolves to a real directory"
    );

    fs::remove_dir_all(&output_dir).ok();
}

/// Decode a 64-char lowercase hex string into 32 raw bytes. Mirrors the
/// `decode_hex32` the solver uses to compare `bound_model_hash`.
fn decode_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn tempdir_for(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "configflux-compiler-{test_name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}
