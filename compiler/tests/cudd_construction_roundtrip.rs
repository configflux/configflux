// SPDX-License-Identifier: BUSL-1.1
//
// CUDD-side compiler construction round-trip — configflux-wbzw.
//
// Acceptance bar 4 from the wbzw task description:
// "Run --construction=cudd on the FAMA `requires` fixture (or one
//  loop fixture) and assert: produced .ccm decodes via solver,
//  valid_options returns a non-empty result, no panic."
//
// We use a hand-authored `ConditionModel` that exercises the four
// `ConditionExpr` shapes (Bool, Predicate, Not, And, Or). Running the
// CUDD path end-to-end through `emit_ccm_dir_with_construction`
// produces a three-file .ccm directory; loading it into `Session<CuddBackend>`
// + `Session<OxiddBackend>` must succeed; `valid_options` over a known
// facet must return a non-empty result without panic.
//
// This test does NOT run the gf2o-acceptance fixture (10k×50/50) — that
// belongs to `configflux-vfx4` per ADR-0011 §"Follow-on work".

use std::fs;
use std::path::PathBuf;

use compiler::ccm_emitter::{emit_ccm_dir_with_construction, ConditionModel};
use solver::{CuddBackend, OxiddBackend, Session, SolverBackend};

fn sample_model() -> ConditionModel {
    ConditionModel::from_clauses(
        "33".repeat(32),
        vec![
            // Boolean conjunction + predicate.
            "engine == 'v6' && gearbox == 'auto'".to_string(),
            // Predicate + negation.
            "!(region == 'restricted')".to_string(),
            // Disjunction.
            "gearbox == 'auto' || gearbox == 'manual'".to_string(),
        ],
    )
}

#[test]
fn cudd_path_emits_loadable_ccm_with_correct_manifest_tag() {
    let dir = tempdir_for("cudd_construction_manifest");
    let model = sample_model();
    emit_ccm_dir_with_construction(
        &model,
        &dir,
        "facet-name-ascending",
        "cudd",
    )
    .expect("CUDD-path emit must succeed on a small model");

    let manifest = fs::read_to_string(dir.join("ccm.manifest.json"))
        .expect("manifest must be written by CUDD path");
    assert!(
        manifest.contains("\"algorithm\":\"robdd-cudd-v1\""),
        "CUDD-path manifest must carry the robdd-cudd-v1 tag: {manifest}"
    );
    assert!(
        manifest.contains("\"apply_cache\":\"cudd-default\""),
        "CUDD-path algorithm_params must carry apply_cache=cudd-default: {manifest}"
    );
    assert!(
        manifest.contains("\"manager\":\"cudd-3.0\""),
        "CUDD-path algorithm_params must carry manager=cudd-3.0: {manifest}"
    );
    assert!(
        manifest.contains("\"reorder\":\"static\""),
        "CUDD-path algorithm_params must carry reorder=static: {manifest}"
    );
    assert!(
        manifest.contains("\"threads\":\"1\""),
        "CUDD-path algorithm_params must carry threads=1: {manifest}"
    );
}

#[test]
fn cudd_path_ccm_loads_into_solver_via_oxidd_backend() {
    let dir = tempdir_for("cudd_construction_oxidd_load");
    let model = sample_model();
    emit_ccm_dir_with_construction(&model, &dir, "facet-name-ascending", "cudd")
        .expect("CUDD-path emit");

    // OxiddBackend reads the .ccm bytes (post-order DFS canonical node
    // table per ADR-0005 §4) and rebuilds the BDD. Per ADR-0011 §3,
    // the CUDD-path and in-crate-path bytes are NOT byte-equal but
    // ARE semantically equivalent, so any conforming SolverBackend
    // can load either.
    let ccm = Session::<OxiddBackend>::load_ccm(&dir)
        .expect("OxiddBackend must load CUDD-emitted .ccm bytes");
    let session = Session::<OxiddBackend>::new(ccm)
        .expect("OxiddBackend must rebuild the BDD from CUDD-emitted bytes");

    // Non-empty valid_options over a known facet. `gearbox` has two
    // values in the model (`auto`, `manual`); valid_options(gearbox)
    // should enumerate both, neither, or whichever are still
    // satisfiable under the conjoined formula.
    let options = session
        .valid_options("gearbox")
        .expect("valid_options on a known facet must not error");
    assert!(
        options.count >= 1,
        "valid_options(gearbox) over the sample model must yield a non-empty result; got count={}",
        options.count
    );

    let current = session.current();
    assert!(
        !session.backend().is_false(current),
        "current formula must not be trivially unsat under the sample model"
    );
}

#[test]
fn cudd_path_ccm_loads_into_solver_via_cudd_backend() {
    let dir = tempdir_for("cudd_construction_cudd_load");
    let model = sample_model();
    emit_ccm_dir_with_construction(&model, &dir, "facet-name-ascending", "cudd")
        .expect("CUDD-path emit");

    // Cross-check: same .ccm, loaded under CuddBackend. Catches the
    // case where the CUDD-emitted byte stream happens to satisfy the
    // OxiddBackend reader but breaks the CUDD-side `canonical_to_cudd`
    // re-rebuild (e.g. if the post-order constraint were violated).
    let ccm = Session::<CuddBackend>::load_ccm(&dir)
        .expect("CuddBackend must load CUDD-emitted .ccm bytes");
    let session = Session::<CuddBackend>::new(ccm)
        .expect("CuddBackend must rebuild the BDD from CUDD-emitted bytes");

    let options = session
        .valid_options("gearbox")
        .expect("valid_options(gearbox) under CuddBackend must not error");
    assert!(
        options.count >= 1,
        "valid_options(gearbox) under CuddBackend must yield a non-empty result; got count={}",
        options.count
    );
}

#[test]
fn cudd_and_in_crate_paths_are_semantically_equivalent() {
    // ADR-0011 §3: "Cross-path byte equality is not provided ... they
    // are interchangeable only at the `Session` API level (same valid
    // options, same resolve results, same explain-rejection outputs)."
    //
    // This test pins the semantic-parity half of that contract on a
    // small model: building the same `ConditionModel` under both
    // paths must produce the same `valid_options` per facet.
    //
    // configflux-mwyp / ADR-0012: under the v2 wire format the
    // single-file `build_ccm_artifact_with_construction` byte stream
    // is no longer round-trip-loadable in isolation — the solver only
    // accepts the v2 multi-part directory layout. We therefore route
    // this semantic-parity check through the dir-emitting entrypoint
    // (`emit_ccm_dir_with_construction`), which produces the v2
    // multi-part shape on disk. The manifest-tag invariants still
    // apply (top-level manifest carries the algorithm tag).
    let model = sample_model();

    // Build via in-crate (default).
    let in_crate_dir = tempdir_for("cudd_vs_in_crate_in_crate");
    emit_ccm_dir_with_construction(
        &model,
        &in_crate_dir,
        "facet-name-ascending",
        "in-crate",
    )
    .expect("in-crate build");
    // Build via CUDD.
    let cudd_dir = tempdir_for("cudd_vs_in_crate_cudd");
    emit_ccm_dir_with_construction(&model, &cudd_dir, "facet-name-ascending", "cudd")
        .expect("CUDD-path build");

    // Algorithm tag invariant carried by the top-level v2 manifest.
    let in_crate_manifest =
        fs::read_to_string(in_crate_dir.join("ccm.manifest.json")).expect("read in-crate top");
    let cudd_manifest =
        fs::read_to_string(cudd_dir.join("ccm.manifest.json")).expect("read cudd top");
    assert!(in_crate_manifest.contains("robdd-handrolled-v1"));
    assert!(cudd_manifest.contains("robdd-cudd-v1"));

    let session_in_crate = load(&in_crate_dir);
    let session_cudd = load(&cudd_dir);

    for facet in &["engine", "gearbox", "region"] {
        let a = session_in_crate
            .valid_options(facet)
            .expect("in-crate valid_options");
        let b = session_cudd
            .valid_options(facet)
            .expect("cudd valid_options");
        // Compare option-name sets (order is not part of the contract).
        let mut a_names: Vec<&str> = a.options.iter().map(String::as_str).collect();
        let mut b_names: Vec<&str> = b.options.iter().map(String::as_str).collect();
        a_names.sort_unstable();
        b_names.sort_unstable();
        assert_eq!(
            a_names, b_names,
            "valid_options({facet}) must match across in-crate and CUDD paths"
        );
    }
}

fn load(dir: &PathBuf) -> Session<OxiddBackend> {
    let ccm = Session::<OxiddBackend>::load_ccm(dir).expect("load_ccm");
    Session::<OxiddBackend>::new(ccm).expect("session")
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-wbzw", test_name)
}
