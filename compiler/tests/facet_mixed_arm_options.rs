// SPDX-License-Identifier: BUSL-1.1

//! ADR-0047 §4/§6 — mixed-arm regression (configflux-5zqr).
//!
//! Coverage gap left by `facet_symbol_synthesis.rs`, whose fixtures are
//! DECLARATION-ONLY (no condition references the declared facet). The AtmosNet
//! F2 shape is the *mixed* arm: a declared closed facet whose DEFAULT value is
//! named by no condition, while a SIBLING value IS condition-referenced (it
//! gates a component). QA confirmed the defect black-box on the adopted
//! examples 02/03/04.
//!
//! This test drives the exact path `cfx options` takes — `session_compose::
//! options`, the solver-authoritative composition (`cfx/src/options.rs`) — over
//! a model compiled through the REAL product pipeline (`compile_model` ->
//! `open_model`), NOT the declaration-only `.ccm`/loader helpers the T3 test
//! used. It asserts the cfx-visible contract:
//!
//!   (1) single-guard closed facet: `valid_options` contains BOTH the
//!       condition-referenced sibling AND the never-referenced default arm.
//!   (2) multi-guard closed facet (>= 2 referenced arms + a default): the facet
//!       has a NON-EMPTY domain — the shape that today collapses the whole facet
//!       to `valid_options: []` (over-constrained BDD, exactly_one_of AMO under
//!       forced guards).
//!
//! It is spec-independent: it encodes the F2 user-facing guarantee, not the
//! synthesized-clause mechanism, so it stays valid across the ADR-0047 §4
//! amendment (symbol-introduction-only synthesis; exclusivity at the
//! selection/tag layer).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use compiler::loader_api::{
    initialize_selection_state, open_model, GetSelectionOptionsRequest,
    InitializeSelectionStateRequest, ModelHandle, OpenModelRequest, SelectionState,
};
use compiler::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};

/// Example-02 shape: a closed facet `bus_type` over `[serial, ethernet]` with
/// `serial` as the default arm, and a component gated on the SIBLING value
/// `ethernet`. `serial` is named by no condition (the invisible-before-ADR-0047
/// default); `ethernet` is forced true by the component guard.
const SINGLE_GUARD_FIXTURE: &str = r#"{
    "package": "facet_mixed_single",
    "version": "1.0.0",
    "facets": {
        "bus_type": { "values": ["serial", "ethernet"], "default": "serial", "open": false }
    },
    "components": {
        "sensor_bus": { "type": "service" },
        "network_monitor": { "type": "service", "condition": "bus_type == 'ethernet'" }
    }
}"#;

/// The multi-arm AtmosNet shape: a closed facet `color` over `[red, green,
/// blue]` with `blue` the default, and TWO components each gated on a DIFFERENT
/// sibling (`red`, `green`). Under the pre-amendment synthesis both siblings are
/// forced true and the closed `exactly_one_of` AMO makes the whole facet UNSAT,
/// collapsing `valid_options` to `[]`.
const MULTI_GUARD_FIXTURE: &str = r#"{
    "package": "facet_mixed_multi",
    "version": "1.0.0",
    "facets": {
        "color": { "values": ["red", "green", "blue"], "default": "blue", "open": false }
    },
    "components": {
        "a": { "type": "service", "condition": "color == 'red'" },
        "b": { "type": "service", "condition": "color == 'green'" }
    }
}"#;

struct Compiled {
    output_dir: PathBuf,
    handle: ModelHandle,
}

fn compile_fixture(label: &str, source: &str) -> Compiled {
    let output_dir = tempdir_for(label);
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "scenarios/facet/00_mixed.json".to_string(),
            inline_content: source.to_string(),
        }],
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
    let cmp_manifest_ref = result
        .compiled_model_package_ref
        .clone()
        .expect("cmp manifest ref present");
    let open_result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref,
    });
    assert_eq!(open_result.status, OperationStatus::Ok, "open_model ok");
    Compiled {
        handle: open_result.model_handle.expect("model handle present"),
        output_dir,
    }
}

fn empty_selection_state(handle: &ModelHandle) -> SelectionState {
    let init = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        context_tags: BTreeMap::new(),
    });
    assert_eq!(init.status, OperationStatus::Ok, "init selection state ok");
    init.selection_state.expect("selection_state present")
}

/// The `valid_options` a `cfx options` invocation would print for `facet` under
/// an empty selection — routed through the SAME solver-authoritative
/// `session_compose::options` seam `cfx/src/options.rs` uses.
fn cfx_valid_options(handle: &ModelHandle, facet: &str) -> Vec<String> {
    let state = empty_selection_state(handle);
    let result = session_compose::options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        facet: facet.to_string(),
        include_pruned_reasons: false,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "options must succeed for a modeled facet: {:?}",
        result.diagnostics
    );
    let mut opts = result.valid_options;
    opts.sort();
    opts
}

#[test]
fn single_guard_closed_facet_lists_default_and_referenced_sibling() {
    let compiled = compile_fixture("facet-mixed-single", SINGLE_GUARD_FIXTURE);

    // The cfx-visible contract (ADR-0047 §4/§6): the closed facet's FULL declared
    // domain is offered — the condition-referenced sibling `ethernet` AND the
    // never-referenced default arm `serial`. Today `serial` is pruned (forced
    // `ethernet` + exactly_one_of AMO make it UNSAT), so this is RED.
    assert_eq!(
        cfx_valid_options(&compiled.handle, "bus_type"),
        vec!["ethernet".to_string(), "serial".to_string()],
        "cfx options(bus_type) must offer BOTH the referenced sibling and the default arm"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

#[test]
fn multi_guard_closed_facet_has_non_empty_domain() {
    let compiled = compile_fixture("facet-mixed-multi", MULTI_GUARD_FIXTURE);

    let opts = cfx_valid_options(&compiled.handle, "color");

    // The load-bearing regression: >= 2 condition-referenced arms must NOT
    // collapse the facet. Today the exactly_one_of AMO under two forced guards is
    // UNSAT and this list is empty.
    assert!(
        !opts.is_empty(),
        "multi-guarded closed facet must keep a non-empty domain, got {opts:?}"
    );
    // Stronger form of the same guarantee: every declared value stays offered,
    // including the two referenced siblings and the default arm.
    assert_eq!(
        opts,
        vec!["blue".to_string(), "green".to_string(), "red".to_string()],
        "multi-guarded closed facet must still offer every declared value"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
