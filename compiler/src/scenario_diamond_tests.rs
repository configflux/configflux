// SPDX-License-Identifier: BUSL-1.1

//! Verification-first coverage for ADR-0048 (allow diamond dependencies; DAG
//! component graphs). The pack mirrors the AtmosNet cold-evaluation topology
//! that finding F4 hit: a shared `platform_hal` (HAL) component reached from a
//! single root via several feature components — an honest diamond, and exactly
//! how real embedded/product stacks are shaped. A second root re-converges on
//! the same shared component to exercise cross-root convergence.
//!
//! Per ADR-0048 §5 the assertions are:
//!   1. the pack compiles and verifies (the diamond ban is gone),
//!   2. resolution is deterministic — byte-identical `resolve_hash` and
//!      `model_hash` across repeated runs (the concrete §1 determinism proof),
//!   3. the retained per-edge condition-implication check still rejects a
//!      diamond whose shared dependency is disabled where a dependent is
//!      enabled (the real hazard is still caught; retiring the ban opened no
//!      safety hole).
//!
//! Per ADR-0047 the pack declares its facets (closed, with defaults) like any
//! post-facet-train domain rather than relying on a removed default arm.

use crate::loader_api::{
    canonical_selection_state, open_model, resolve_from_selection, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest,
};
use crate::product_api::{
    compile_model, verify_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    VerifyModelRequest, PRODUCT_SCHEMA_VERSION,
};
use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};
use anyhow::{Context, Result};
use std::collections::BTreeMap;

const HONEST_DIAMOND_SOURCE: &str = "scenarios/atmosnet/honest_diamond.toml";

/// Honest diamond: `edge_gateway` reaches the shared `platform_hal` via three
/// feature components, and a second root `diagnostics_agent` re-converges on it
/// via two paths. All components are unconditional (the least-restrictive
/// shared dependency sits at the bottom), so the per-edge condition-implication
/// check passes trivially and the *only* thing that rejected this pack before
/// ADR-0048 was `detect_diamond`. Two closed facets are declared with defaults
/// per ADR-0047.
const HONEST_DIAMOND_CHUNK: &str = r#"
package = "atmosnet_edge"
version = "1.0.0"

[facets.deployment]
values = ["edge", "cloud"]
default = "edge"

[facets.radio]
values = ["lora", "nbiot"]
default = "lora"

[components.platform_hal]
type = "platform"

[components.sensor_driver]
type = "module"
depends_on = ["platform_hal"]

[components.power_manager]
type = "module"
depends_on = ["platform_hal"]

[components.fleet_ota]
type = "module"
depends_on = ["platform_hal"]

[components.edge_gateway]
type = "module"
depends_on = ["sensor_driver", "power_manager", "fleet_ota"]

[components.diagnostics_agent]
type = "module"
depends_on = ["power_manager", "sensor_driver"]
"#;

const UNSAFE_DIAMOND_SOURCE: &str = "scenarios/atmosnet/unsafe_diamond.toml";

/// Retained-safety fixture: a diamond whose shared dependency `platform_hal` is
/// only active in `cloud`, while its dependents are active in `edge`. The
/// per-edge condition-implication check (`link_verify.rs`) must still reject it
/// after the diamond ban is retired — `edge => cloud` does not hold.
const UNSAFE_DIAMOND_CHUNK: &str = r#"
package = "atmosnet_unsafe"
version = "1.0.0"

[facets.deployment]
values = ["edge", "cloud"]
default = "edge"

[components.platform_hal]
type = "platform"
condition = "deployment == 'cloud'"

[components.sensor_driver]
type = "module"
condition = "deployment == 'edge'"
depends_on = ["platform_hal"]

[components.power_manager]
type = "module"
condition = "deployment == 'edge'"
depends_on = ["platform_hal"]

[components.edge_gateway]
type = "module"
condition = "deployment == 'edge'"
depends_on = ["sensor_driver", "power_manager"]
"#;

fn manifest(source_id: &str, chunk: &str) -> Vec<SourceManifestEntry> {
    vec![SourceManifestEntry {
        source_id: source_id.to_string(),
        inline_content: chunk.to_string(),
    }]
}

fn honest_manifest() -> Vec<SourceManifestEntry> {
    manifest(HONEST_DIAMOND_SOURCE, HONEST_DIAMOND_CHUNK)
}

fn temp_output_dir(label: &str) -> Result<TempDirGuard> {
    unique_temp_dir("configflux-diamond", label)
}

/// Compile the honest-diamond pack and open a model handle. Fails loudly if the
/// compile does not succeed so the RED phase (before `detect_diamond` is
/// removed) surfaces the real diamond diagnostic. The returned `TempDirGuard`
/// owns the on-disk CMP package the handle points at; the caller must keep it
/// alive until every resolve against the handle is done.
fn compile_and_open(label: &str) -> Result<(ModelHandle, String, TempDirGuard)> {
    let output_dir = temp_output_dir(label)?;
    let compile_result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: honest_manifest(),
        output_dir: Some(output_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    if compile_result.status != OperationStatus::Ok {
        anyhow::bail!(
            "compile_model rejected the honest diamond: {:?}",
            compile_result.verify_report.diagnostics.diagnostics
        );
    }
    let cmp_manifest_ref = compile_result
        .compiled_model_package_ref
        .as_deref()
        .context("missing compiled_model_package_ref")?;
    let handle = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_manifest_ref.to_string(),
    });
    if handle.status != OperationStatus::Ok {
        anyhow::bail!("open_model failed: {:?}", handle.diagnostics.diagnostics);
    }
    let model_handle = handle.model_handle.context("missing model_handle")?;
    Ok((model_handle, compile_result.model_hash, output_dir))
}

/// Resolve the given scope from an empty selection and return its `resolve_hash`.
fn resolve_hash_for_scope(handle: &ModelHandle, scope: &str) -> Result<String> {
    let selection_state = canonical_selection_state(
        handle.model_hash.clone(),
        scope.to_string(),
        BTreeMap::new(),
        BTreeMap::new(),
    )?;
    let resolve_result = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        selection_state,
    });
    if resolve_result.status != OperationStatus::Ok {
        anyhow::bail!(
            "resolve_from_selection failed for scope '{}': {:?}",
            scope,
            resolve_result.diagnostics.diagnostics
        );
    }
    resolve_result
        .resolve_hash
        .context("missing resolve_hash on a successful resolve")
}

#[test]
fn honest_diamond_compiles_and_resolves_deterministically() -> Result<()> {
    // (1) The pack — a real diamond (`platform_hal` reached via three paths from
    // `edge_gateway`) — verifies cleanly now that the ban is retired.
    let verify_report = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: honest_manifest(),
    });
    assert_eq!(
        verify_report.status,
        OperationStatus::Ok,
        "honest diamond should verify: {:?}",
        verify_report.diagnostics.diagnostics
    );
    assert_eq!(verify_report.error_count, 0);

    // (2) Determinism: compile twice and resolve the diamond root's closure
    // twice; `model_hash` and `resolve_hash` must be byte-identical across runs.
    let (handle_a, model_hash_a, _guard_a) = compile_and_open("determinism-a")?;
    let (handle_b, model_hash_b, _guard_b) = compile_and_open("determinism-b")?;
    assert_eq!(
        model_hash_a, model_hash_b,
        "model_hash must be stable across repeated compiles of a diamond pack"
    );
    assert_eq!(handle_a.model_hash, model_hash_a);

    let scope = "component:edge_gateway";
    let resolve_hash_a = resolve_hash_for_scope(&handle_a, scope)?;
    let resolve_hash_b = resolve_hash_for_scope(&handle_a, scope)?;
    let resolve_hash_c = resolve_hash_for_scope(&handle_b, scope)?;
    assert_eq!(
        resolve_hash_a, resolve_hash_b,
        "resolve_hash must be identical across repeated resolves (same handle)"
    );
    assert_eq!(
        resolve_hash_a, resolve_hash_c,
        "resolve_hash must be identical across repeated compiles + resolves"
    );

    Ok(())
}

#[test]
fn honest_diamond_second_root_resolves() -> Result<()> {
    // The second root `diagnostics_agent` re-converges on `platform_hal` via two
    // paths; its closure resolves deterministically too, exercising cross-root
    // convergence on the shared component.
    let (handle, _model_hash, _guard) = compile_and_open("second-root")?;
    let scope = "component:diagnostics_agent";
    let first = resolve_hash_for_scope(&handle, scope)?;
    let second = resolve_hash_for_scope(&handle, scope)?;
    assert_eq!(
        first, second,
        "second-root resolve_hash must be deterministic across runs"
    );
    Ok(())
}

#[test]
fn diamond_with_disabled_shared_dep_still_rejected() {
    // Retained safety: a diamond is not a free pass. If the shared dependency is
    // disabled in a context where a dependent is enabled, the per-edge
    // condition-implication check must still reject the pack after ADR-0048.
    let report = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest(UNSAFE_DIAMOND_SOURCE, UNSAFE_DIAMOND_CHUNK),
    });
    assert_eq!(
        report.status,
        OperationStatus::Error,
        "a diamond with a disabled shared dependency must still be rejected"
    );
    let messages: Vec<&str> = report
        .diagnostics
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("Condition incompatibility")),
        "expected the condition-implication check to reject it, got: {:?}",
        messages
    );
}
