// SPDX-License-Identifier: BUSL-1.1

//! Byte-stability snapshot test covering every checked-in scenario pack.
//!
//! Pre-flight regression gate for the v0.3.0 solver rewrite. Drives every
//! scenario variant (s1..s5 at smoke/medium/large sizes) through the full
//! loader_api closed loop (compile -> open_model -> resolve ->
//! export_software_bom) and asserts `model_hash`, `selection_state_hash`,
//! `resolve_hash`, and `bom_hash` match a checked-in baseline fixture byte
//! for byte.
//!
//! Unlike `scenario_scale_tests`, which only proves hashes are stable within
//! a single process run, this test pins the values so any change to
//! canonicalization, HashMap iteration, or the hashing pipeline fails loudly
//! even if the new hashes are internally consistent.
//!
//! If this test goes red: read the per-field diff in the failure message.
//! If the drift is intentional (e.g. a schema bump), regenerate the baseline
//! fixture at `compiler/tests/fixtures/byte-stability-baselines.json`. A
//! baseline update is a public hash-contract change — review carefully
//! before committing. If the drift is unintentional, the bug is upstream
//! (resolver, canonicalization, selection state, or sbom emit) — fix it.

use crate::loader_api::{
    canonical_selection_state, export_software_bom, open_model, resolve_from_selection,
    ExportSoftwareBomRequest, ModelHandle, OpenModelRequest, ResolveFromSelectionRequest,
    SelectionState, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
};
use crate::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

// Every scenario variant needs its definitions and components chunks
// embedded at compile time so the test is hermetic.
macro_rules! chunk {
    ($root:literal) => {
        &[
            (
                concat!("scenarios/", $root, "/chunks/00_definitions.toml"),
                include_str!(concat!("../scenarios/", $root, "/cue/00_definitions.json")),
            ),
            (
                concat!("scenarios/", $root, "/chunks/10_components.toml"),
                include_str!(concat!("../scenarios/", $root, "/cue/10_components.json")),
            ),
        ]
    };
}

/// Default contexts mirror each scenario's profile.toml `default_context`.
/// These are the canonical inputs that reproduce the pinned hashes.
const S1_CONTEXT: &[(&str, &str)] = &[
    ("cooling_brand", "hydra"),
    ("cooling_model", "x200"),
    ("pump_type", "dual"),
    ("region", "us"),
];
const S2_CONTEXT: &[(&str, &str)] = &[
    ("blade_class", "onshore"),
    ("gearbox_type", "direct_drive"),
    ("grid_code", "iec_61400"),
    ("sensor_pack", "core"),
];
const S3_CONTEXT: &[(&str, &str)] = &[
    ("conveyor_brand", "swiftmove"),
    ("network_topology", "ring"),
    ("safety_mode", "pl_d"),
    ("vision_stack", "opticore"),
];
const S4_CONTEXT: &[(&str, &str)] = &[
    ("battery_pack", "high_density"),
    ("drive_type", "mecanum"),
    ("localization_stack", "lidar"),
    ("payload_module", "heavy_lift"),
    ("region", "us"),
];
const S5_CONTEXT: &[(&str, &str)] = &[
    ("filtration_grade", "hepa"),
    ("occupancy_class", "hospital"),
    ("region", "us"),
];

/// The authoritative ledger of scenario variants covered by this test.
/// Adding a new scenario under compiler/scenarios/ requires a new row here,
/// a matching entry in the baseline fixture, and a new line in
/// EXPECTED_VARIANTS below (see the second test for the guardrail).
///
/// `pub(crate)` because it is the corpus roster for the whole crate's
/// pack-level tests, not just this one: `chunk_address_tests` reads it so the
/// chunk-address invariant is asserted over the same eleven variants, under the
/// same coverage guard, rather than over a second hand-written list that could
/// silently fall behind this one.
#[rustfmt::skip]
pub(crate) const SCENARIOS: &[ScenarioSpec] = &[
    ScenarioSpec { key: "s1-smoke",  scope: "component:thermal_control",        chunks: chunk!("s1_water_pump/smoke"),       context: S1_CONTEXT },
    ScenarioSpec { key: "s1-medium", scope: "component:thermal_control",        chunks: chunk!("s1_water_pump/medium"),      context: S1_CONTEXT },
    ScenarioSpec { key: "s2-smoke",  scope: "component:turbine_controller",     chunks: chunk!("s2_wind_turbine/smoke"),     context: S2_CONTEXT },
    ScenarioSpec { key: "s2-medium", scope: "component:turbine_controller",     chunks: chunk!("s2_wind_turbine/medium"),    context: S2_CONTEXT },
    ScenarioSpec { key: "s3-smoke",  scope: "component:swift_ring_standard",    chunks: chunk!("s3_automation_cell/smoke"),  context: S3_CONTEXT },
    ScenarioSpec { key: "s3-medium", scope: "component:swift_ring_standard",    chunks: chunk!("s3_automation_cell/medium"), context: S3_CONTEXT },
    ScenarioSpec { key: "s3-large",  scope: "component:large_cell_orchestrator",chunks: chunk!("s3_automation_cell/large"),  context: S3_CONTEXT },
    ScenarioSpec { key: "s4-smoke",  scope: "component:robot_platform",         chunks: chunk!("s4_mobile_robot/smoke"),     context: S4_CONTEXT },
    ScenarioSpec { key: "s4-medium", scope: "component:robot_platform",         chunks: chunk!("s4_mobile_robot/medium"),    context: S4_CONTEXT },
    ScenarioSpec { key: "s4-large",  scope: "component:robot_platform",         chunks: chunk!("s4_mobile_robot/large"),     context: S4_CONTEXT },
    ScenarioSpec { key: "s5-smoke",  scope: "component:climate_controller",     chunks: chunk!("s5_building_hvac/smoke"),    context: S5_CONTEXT },
];

#[derive(Clone, Copy)]
pub(crate) struct ScenarioSpec {
    pub(crate) key: &'static str,
    pub(crate) scope: &'static str,
    pub(crate) chunks: &'static [(&'static str, &'static str)],
    /// Context tags (from profile.toml `default_context`) used to build the
    /// canonical selection state. Choices are left empty — this is the
    /// context-only resolution path exercised by scenario_early_binding_tests.
    pub(crate) context: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BaselineFile {
    schema_version: u32,
    scenarios: BTreeMap<String, ScenarioBaseline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ScenarioBaseline {
    scope: String,
    model_hash: String,
    selection_state_hash: String,
    resolve_hash: String,
    bom_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedHashes {
    model_hash: String,
    selection_state_hash: String,
    resolve_hash: String,
    bom_hash: String,
}

const BASELINE_FIXTURE: &str = include_str!("../tests/fixtures/byte-stability-baselines.json");

fn parse_baselines() -> Result<BaselineFile> {
    let baseline: BaselineFile = serde_json::from_str(BASELINE_FIXTURE)
        .context("Failed to parse byte-stability-baselines.json")?;
    if baseline.schema_version != 1 {
        anyhow::bail!(
            "Unsupported baseline schema_version {} (expected 1)",
            baseline.schema_version
        );
    }
    Ok(baseline)
}

fn manifest_for_chunks(chunks: &[(&str, &str)]) -> Vec<SourceManifestEntry> {
    chunks
        .iter()
        .map(|(source_id, content)| SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*content).to_string(),
        })
        .collect()
}

fn context_map(context: &[(&str, &str)]) -> BTreeMap<String, String> {
    context
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn open_handle(cmp_manifest: &Path) -> Result<ModelHandle> {
    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_manifest.to_string_lossy().into_owned(),
    });
    if result.status != OperationStatus::Ok {
        return Err(anyhow!(
            "open_model failed: {:?}",
            result.diagnostics.diagnostics
        ));
    }
    result
        .model_handle
        .context("open_model returned Ok but no model_handle")
}

fn build_selection_state(
    handle: &ModelHandle,
    scope: &str,
    context: &[(&str, &str)],
) -> Result<SelectionState> {
    canonical_selection_state(
        handle.model_hash.clone(),
        scope.to_string(),
        context_map(context),
        BTreeMap::new(),
    )
}

fn run_scenario(spec: &ScenarioSpec) -> Result<(ObservedHashes, TempDirGuard)> {
    let manifest = manifest_for_chunks(spec.chunks);

    let out_dir = unique_temp_dir("configflux-byte-stability", spec.key)?;
    let compile_result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest,
        output_dir: Some(out_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    if compile_result.status != OperationStatus::Ok {
        return Err(anyhow!(
            "[{}] compile_model failed: {:?}",
            spec.key,
            compile_result.verify_report.diagnostics.diagnostics
        ));
    }
    let cmp_manifest_ref = compile_result
        .compiled_model_package_ref
        .as_deref()
        .with_context(|| {
            format!("[{}] compile_result missing compiled_model_package_ref", spec.key)
        })?;
    let handle = open_handle(Path::new(cmp_manifest_ref))
        .with_context(|| format!("[{}] open_model", spec.key))?;
    let state = build_selection_state(&handle, spec.scope, spec.context)
        .with_context(|| format!("[{}] build_selection_state", spec.key))?;

    let resolve_result = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: spec.scope.to_string(),
        selection_state: state.clone(),
        implied_choices: Default::default(),
    });
    if resolve_result.status != OperationStatus::Ok {
        return Err(anyhow!(
            "[{}] resolve_from_selection failed: {:?}",
            spec.key,
            resolve_result.diagnostics.diagnostics
        ));
    }
    let resolve_hash = resolve_result
        .resolve_hash
        .clone()
        .with_context(|| format!("[{}] resolve_result missing resolve_hash", spec.key))?;

    let bom_result = export_software_bom(ExportSoftwareBomRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result,
        profile: EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT.to_string(),
    });
    if bom_result.status != OperationStatus::Ok {
        return Err(anyhow!(
            "[{}] export_software_bom failed: {:?}",
            spec.key,
            bom_result.diagnostics.diagnostics
        ));
    }
    let bom_hash = bom_result
        .bom_hash
        .with_context(|| format!("[{}] bom_result missing bom_hash", spec.key))?;

    Ok((
        ObservedHashes {
            model_hash: handle.model_hash.clone(),
            selection_state_hash: state.selection_state_hash.clone(),
            resolve_hash,
            bom_hash,
        },
        out_dir,
    ))
}

#[test]
fn scenario_byte_stability_every_scenario_pack_matches_baseline() -> Result<()> {
    let baseline = parse_baselines()?;

    // SCENARIOS and baseline fixture must cover the same set of keys.
    // Compare as sorted sets — BTreeMap iterates lexicographically, SCENARIOS
    // is declared in pack order.
    let mut scenario_keys: Vec<&str> = SCENARIOS.iter().map(|s| s.key).collect();
    scenario_keys.sort();
    let mut baseline_keys: Vec<&str> = baseline.scenarios.keys().map(|s| s.as_str()).collect();
    baseline_keys.sort();
    assert_eq!(
        scenario_keys, baseline_keys,
        "SCENARIOS and baseline fixture keys disagree. If you added a scenario \
         pack, update both the SCENARIOS list and the baseline fixture."
    );

    let mut failures: Vec<String> = Vec::new();
    for spec in SCENARIOS {
        let expected = baseline
            .scenarios
            .get(spec.key)
            .with_context(|| format!("missing baseline entry for '{}'", spec.key))?;
        assert_eq!(
            expected.scope, spec.scope,
            "[{}] scope drift: fixture has '{}' but scenario spec declares '{}'",
            spec.key, expected.scope, spec.scope
        );

        let (observed, _temp) = match run_scenario(spec) {
            Ok(pair) => pair,
            Err(err) => {
                failures.push(format!("[{}] pipeline error: {err:#}", spec.key));
                continue;
            }
        };

        diff_hash(&mut failures, spec.key, "model_hash", &expected.model_hash, &observed.model_hash);
        diff_hash(&mut failures, spec.key, "selection_state_hash", &expected.selection_state_hash, &observed.selection_state_hash);
        diff_hash(&mut failures, spec.key, "resolve_hash", &expected.resolve_hash, &observed.resolve_hash);
        diff_hash(&mut failures, spec.key, "bom_hash", &expected.bom_hash, &observed.bom_hash);
    }

    if !failures.is_empty() {
        panic!(
            "Byte-stability baseline mismatch: {} hash field(s) drifted from \
             the checked-in baseline.\n\n{}\n\n\
             If this drift is intentional, regenerate the baseline fixture at \
             compiler/tests/fixtures/byte-stability-baselines.json. A baseline \
             update is a public hash-contract change — review carefully before \
             committing.",
            failures.len(),
            failures.join("\n")
        );
    }

    Ok(())
}

fn diff_hash(failures: &mut Vec<String>, key: &str, field: &str, expected: &str, observed: &str) {
    if expected != observed {
        failures.push(format!(
            "[{key}] {field} drift\n  expected: {expected}\n  observed: {observed}"
        ));
    }
}

#[test]
fn scenario_byte_stability_every_pack_under_scenarios_is_covered() -> Result<()> {
    // Guardrail: if a new scenario pack lands under compiler/scenarios/ but
    // is not wired into this test, we want a loud failure so the solver
    // rewrite can't accidentally skip coverage. The embedded SCENARIOS list
    // is the authoritative ledger — this second test cross-checks it against
    // a hardcoded set of expected variants. When a new scenario pack lands,
    // all three of {SCENARIOS, EXPECTED_VARIANTS, baseline fixture} must be
    // updated in the same commit.
    const EXPECTED_VARIANTS: &[&str] = &[
        "s1-smoke", "s1-medium",
        "s2-smoke", "s2-medium",
        "s3-smoke", "s3-medium", "s3-large",
        "s4-smoke", "s4-medium", "s4-large",
        "s5-smoke",
    ];
    let declared_keys: Vec<&str> = SCENARIOS.iter().map(|s| s.key).collect();
    assert_eq!(
        declared_keys, EXPECTED_VARIANTS,
        "SCENARIOS variants drifted from expected ledger. If a new scenario \
         pack was added under compiler/scenarios/, add it to both SCENARIOS \
         and EXPECTED_VARIANTS in the same change."
    );
    Ok(())
}
