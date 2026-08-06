// SPDX-License-Identifier: BUSL-1.1

use std::fs;
use std::path::PathBuf;

use compiler::ccm_emitter::{emit_ccm_dir, ConditionModel};
use solver::{OxiddBackend, Session, SolverBackend};

#[test]
fn emitted_ccm_loads_into_solver_with_symbols() {
    let base = tempdir_for("ccm_emitter_roundtrip");
    let ccm_dir = base.join("ccm");
    let model = ConditionModel::from_clauses(
        "22".repeat(32),
        vec!["a == 'on' && b == 'enabled'".to_string()],
    );

    emit_ccm_dir(&model, &ccm_dir).expect("emit solver-loadable CCM");

    let ccm = Session::<OxiddBackend>::load_ccm(&ccm_dir).expect("load emitted CCM");
    assert_eq!(ccm.bound_model_hash(), [0x22; 32]);

    let symbols = ccm.symbols().expect("symbols should be populated");
    assert_eq!(symbols.var_count(), 2);
    assert_eq!(
        symbols.variable_order().collect::<Vec<_>>(),
        vec!["a.on", "b.enabled"]
    );
    assert_eq!(
        symbols.labels().collect::<Vec<_>>(),
        vec!["a=on", "b=enabled"]
    );
    assert_eq!(symbols.var_for_facet("a.on"), Some(0));
    assert_eq!(symbols.var_for_facet("b.enabled"), Some(1));

    let session = Session::<OxiddBackend>::new(ccm).expect("deserialize emitted BDD");
    let current = session.current();
    assert!(!session.backend().is_false(current));
    assert!(!session.backend().is_true(current));
}

/// ADR-0054 §5.4: a model that DECLARES a constraint must round-trip into the
/// solver.
///
/// This is the pair test, and it earns its place by having caught a real break.
/// The roster lives inside the top-level `ccm_hash` pre-image, and the solver
/// does not trust the emitted `ccm_hash` — it RECONSTRUCTS that pre-image from
/// the parsed manifest and compares. So the emitter's `TopLevelPreimage` and the
/// solver's `TopLevelPreImage` are one contract expressed twice, and a field
/// added to either alone silently makes every constraint-bearing artifact
/// unloadable. The failure does not surface as a hash mismatch either: the
/// loader classifies an unloadable artifact as "no usable model", so the whole
/// selection surface degrades to `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` with
/// nothing pointing at the manifest.
///
/// The constraint-free sibling above cannot catch this, because the roster is
/// omitted entirely when empty — which is exactly what keeps the committed
/// FAMA/SPLOT/synthetic fixtures byte-stable, and exactly what makes their
/// coverage blind here.
#[test]
fn emitted_ccm_with_a_constraint_roster_loads_into_solver() {
    let base = tempdir_for("ccm_emitter_roundtrip_roster");
    let ccm_dir = base.join("ccm");
    let model = ConditionModel {
        bound_model_hash: "44".repeat(32),
        // Symbol-universe contributors: inert tautologies, as `compiler_core`
        // emits for a declared facet's values.
        clauses: vec![
            "environment == 'dev' || environment != 'dev'".to_string(),
            "environment == 'prod' || environment != 'prod'".to_string(),
            "log_level == 'info' || log_level != 'info'".to_string(),
            "log_level == 'debug' || log_level != 'debug'".to_string(),
        ],
        constraints: vec![(
            "prod_forbids_debug".to_string(),
            "environment != 'prod' || log_level != 'debug'".to_string(),
        )],
        cardinality: vec![
            "exactly_one_of(environment == 'dev', environment == 'prod')".to_string(),
            "exactly_one_of(log_level == 'info', log_level == 'debug')".to_string(),
        ],
    };

    emit_ccm_dir(&model, &ccm_dir).expect("emit solver-loadable CCM");

    // The load is the assertion: it recomputes the top-level ccm_hash over a
    // pre-image that must include the roster, and fails closed if it does not.
    let ccm = Session::<OxiddBackend>::load_ccm(&ccm_dir).expect("load emitted CCM with roster");
    assert_eq!(ccm.bound_model_hash(), [0x44; 32]);

    let symbols = ccm.symbols().expect("symbols should be populated");
    assert_eq!(symbols.var_count(), 4);

    // The root is a genuine constraint, not a tautology and not a
    // contradiction: the constraint plus cardinality leave `dev` completions
    // satisfiable while forbidding the `(prod, debug)` pair.
    let session = Session::<OxiddBackend>::new(ccm).expect("deserialize emitted BDD");
    let current = session.current();
    assert!(!session.backend().is_false(current));
    assert!(!session.backend().is_true(current));
}

/// ADR-0054 §5.4: the roster is model-global and belongs only to the top-level
/// manifest, so a per-partition manifest carrying one is rejected rather than
/// ignored — a `root_index` scoped to a partition is uninterpretable, and
/// silently dropping it would hand `cfx explain` a roster it could not trust.
#[test]
fn a_per_partition_manifest_carrying_a_roster_is_rejected() {
    let base = tempdir_for("ccm_emitter_roundtrip_partition_roster");
    let ccm_dir = base.join("ccm");
    let model = ConditionModel::from_clauses(
        "66".repeat(32),
        vec!["a == 'on' && b == 'enabled'".to_string()],
    );
    emit_ccm_dir(&model, &ccm_dir).expect("emit solver-loadable CCM");

    // Forge a roster onto the per-partition manifest, which the emitter never
    // writes there.
    let per_partition = ccm_dir.join("partition-0000").join("ccm.manifest.json");
    let raw = fs::read_to_string(&per_partition).expect("read per-partition manifest");
    let forged = raw.replacen(
        "\"construction_wall_time_us\"",
        "\"constraints\":[{\"condition\":\"a != 'on'\",\"id\":\"forged\",\"root_index\":0}],\
         \"construction_wall_time_us\"",
        1,
    );
    assert_ne!(forged, raw, "the forge must actually change the manifest");
    fs::write(&per_partition, forged).expect("write forged manifest");

    assert!(
        Session::<OxiddBackend>::load_ccm(&ccm_dir).is_err(),
        "a per-partition manifest carrying a constraint roster must be rejected"
    );
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
