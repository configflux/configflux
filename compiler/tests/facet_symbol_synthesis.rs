// SPDX-License-Identifier: BUSL-1.1

//! configflux-3j84 / ADR-0047 §4: first-class facet clause synthesis + loader
//! domain seeding. Black-box, end-to-end through the public product/loader API
//! plus the public solver loader — never reaching into `ccm_emitter` internals.
//!
//! The load-bearing case is **F2**: a facet's DEFAULT arm — the declared value
//! that no condition names — must become a first-class, selectable value.
//! Before this change it had no `{facet}.{value}` symbol, so it was absent from
//! `ccm.symbols.json`, invisible to the solver's `valid_options`, unlisted by
//! `list_selection_facets`, and rejected by the loader's option-validity check.
//!
//! After the change the compiler synthesizes a cardinality clause per declared
//! facet, so every declared value (default arm included) enters the symbol
//! universe with ZERO solver-crate changes; the loader seeds `facet_domains`
//! and a `declared_values` set from the declarations, so a declaration-only
//! facet is listed and its declared values (including the default) are accepted
//! by `get_selection_options`.
//!
//! Fixtures are DECLARATION-ONLY (no condition references the declared facet).
//! That is ADR §3's canonical F2 shape ("a declared facet referenced by no
//! condition is valid — it contributes its domain to the symbol universe") and
//! keeps the solver's feasibility formula (the hard conjunction of all authored
//! clauses) from forcing a non-default arm, which is what makes an unconstrained
//! `valid_options` offer every declared value. The exact synthesized clause
//! bytes for the condition-interacting cases (open effective domain = declared
//! ∪ inferred, single-value degeneracies) are pinned by the `compiler_core`
//! unit tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use compiler::loader_api::{
    get_selection_options, initialize_selection_state, list_selection_facets, open_model,
    GetSelectionOptionsRequest, InitializeSelectionStateRequest, ModelHandle, OpenModelRequest,
    SelectionState,
};
use compiler::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use solver::{OxiddBackend, Session, SolverBackend};

/// Closed facet `region` over `[eu, us, apac]` with `apac` as the default arm.
/// NO condition references `region`, so `apac` is the invisible-before-ADR-0047
/// default (F2), and the closed `exactly_one_of` synthesized clause is the only
/// clause that mentions `region`.
const CLOSED_FIXTURE: &str = r#"{
    "package": "facet_f2_closed",
    "version": "1.0.0",
    "facets": {
        "region": { "values": ["eu", "us", "apac"], "default": "apac", "open": false }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// Open facet `env` over `[prod, staging]` with `staging` as the default arm.
/// Declaration-only; the synthesized clause is at-most-one (no at-least-one),
/// so "none selected" stays satisfiable while both known values are offered.
const OPEN_FIXTURE: &str = r#"{
    "package": "facet_open",
    "version": "1.0.0",
    "facets": {
        "env": { "values": ["prod", "staging"], "default": "staging", "open": true }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

struct Compiled {
    output_dir: PathBuf,
    model_hash: String,
    ccm_dir: PathBuf,
    handle: ModelHandle,
}

fn compile_fixture(label: &str, source: &str) -> Compiled {
    let output_dir = tempdir_for(label);
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        // Fixed source_id: `source_id` is folded into `model_hash` (chunks are
        // sorted by and hashed with it), so the determinism test — which
        // compiles the SAME content twice — must present it under an IDENTICAL
        // source_id. Only the output directory varies per compile.
        source_manifest: vec![SourceManifestEntry {
            source_id: "scenarios/facet/00_facets.json".to_string(),
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
        model_hash: result.model_hash.clone(),
        ccm_dir: output_dir.join("ccm"),
        handle: open_result.model_handle.expect("model handle present"),
        output_dir,
    }
}

/// Every `{facet}.{value}` label the emitted `.ccm` symbol table carries.
fn ccm_symbol_labels(ccm_dir: &std::path::Path) -> Vec<String> {
    let ccm = Session::<OxiddBackend>::load_ccm(ccm_dir).expect("solver loads emitted .ccm");
    ccm.symbols()
        .expect("emitted .ccm carries a symbol table")
        .labels()
        .map(str::to_string)
        .collect()
}

/// The solver's `valid_options` for a facet, read over the emitted `.ccm`
/// (BDD path — the mechanism the synthesized clauses feed, zero solver change).
fn solver_valid_options(ccm_dir: &std::path::Path, facet: &str) -> Vec<String> {
    let ccm = Session::<OxiddBackend>::load_ccm(ccm_dir).expect("solver loads emitted .ccm");
    let session = Session::<OxiddBackend>::new(ccm).expect("session from emitted BDD");
    let current = session.current();
    assert!(
        !session.backend().is_false(current),
        "declaration-only model must be satisfiable (root != FALSE)"
    );
    let mut opts = session
        .valid_options(facet)
        .unwrap_or_else(|e| panic!("valid_options({facet}) failed: {e:?}"))
        .options;
    opts.sort();
    opts
}

/// The loader's `get_selection_options` valid-option list for a facet under an
/// empty selection — exercises the `declared_values`-aware option-validity
/// check, distinct from the solver `.ccm` path above.
fn loader_valid_options(handle: &ModelHandle, facet: &str) -> Vec<String> {
    let state = empty_selection_state(handle);
    let result = get_selection_options(GetSelectionOptionsRequest {
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
        "get_selection_options must succeed (facet must be known): {:?}",
        result.diagnostics
    );
    let mut opts = result.valid_options;
    opts.sort();
    opts
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

#[test]
fn closed_facet_default_arm_becomes_a_first_class_selectable_value() {
    let compiled = compile_fixture("facet-closed", CLOSED_FIXTURE);

    // (a) The default-only arm now has a `{facet}.{value}` symbol, alongside
    //     the other declared arms.
    // The `.ccm` symbol table's human-readable labels use the `{tag}={value}`
    // form (the BDD variable order that `valid_options` prefix-strips uses the
    // `{tag}.{value}` form internally).
    let labels = ccm_symbol_labels(&compiled.ccm_dir);
    for want in ["region=eu", "region=us", "region=apac"] {
        assert!(
            labels.iter().any(|l| l == want),
            "ccm.symbols must contain '{want}'; got {labels:?}"
        );
    }

    // (b) The solver's valid_options offers the default arm — TOGETHER with the
    //     other declared values, proving the synthesized exactly_one_of clause
    //     co-located them in one partition (partition sanity). Achieved with
    //     ZERO solver-crate changes.
    assert_eq!(
        solver_valid_options(&compiled.ccm_dir, "region"),
        vec!["apac".to_string(), "eu".to_string(), "us".to_string()],
        "solver valid_options(region) must offer all declared values incl. the default arm"
    );

    // (c) The declaration-only facet is enumerated by the loader.
    let facets = list_selection_facets(&compiled.handle).expect("list_selection_facets");
    assert!(
        facets.iter().any(|f| f == "region"),
        "list_selection_facets must include the declared facet; got {facets:?}"
    );

    // (d) The loader's option-validity path also accepts every declared value,
    //     including the default arm that appears in no condition.
    assert_eq!(
        loader_valid_options(&compiled.handle, "region"),
        vec!["apac".to_string(), "eu".to_string(), "us".to_string()],
        "get_selection_options(region) must accept the declared values incl. 'apac'"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

#[test]
fn open_facet_offers_all_declared_values_via_at_most_one() {
    let compiled = compile_fixture("facet-open", OPEN_FIXTURE);

    let labels = ccm_symbol_labels(&compiled.ccm_dir);
    for want in ["env=prod", "env=staging"] {
        assert!(
            labels.iter().any(|l| l == want),
            "ccm.symbols must contain '{want}'; got {labels:?}"
        );
    }

    // At-most-one (not exactly-one): every declared value is individually
    // selectable, so both survive as valid options — including the default arm.
    assert_eq!(
        solver_valid_options(&compiled.ccm_dir, "env"),
        vec!["prod".to_string(), "staging".to_string()],
        "open facet valid_options must offer every declared value"
    );

    let facets = list_selection_facets(&compiled.handle).expect("list_selection_facets");
    assert!(facets.iter().any(|f| f == "env"), "env facet listed");
    assert_eq!(
        loader_valid_options(&compiled.handle, "env"),
        vec!["prod".to_string(), "staging".to_string()],
        "get_selection_options(env) must accept the open facet's declared values"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

#[test]
fn synthesized_clause_emission_is_byte_stable_across_compiles() {
    // Two independent compiles of the same declaring fixture must produce the
    // same model_hash and the same symbol table — the synthesized clauses are
    // deterministic (facet-name-ascending, declared-order, pinned left-fold).
    let first = compile_fixture("facet-determinism-a", CLOSED_FIXTURE);
    let second = compile_fixture("facet-determinism-b", CLOSED_FIXTURE);

    assert_eq!(
        first.model_hash, second.model_hash,
        "model_hash must be identical across repeat compiles of a declaring model"
    );
    assert_eq!(
        ccm_symbol_labels(&first.ccm_dir),
        ccm_symbol_labels(&second.ccm_dir),
        "ccm symbol table must be byte-order identical across repeat compiles"
    );

    fs::remove_dir_all(&first.output_dir).ok();
    fs::remove_dir_all(&second.output_dir).ok();
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
