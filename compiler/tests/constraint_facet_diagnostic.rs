// SPDX-License-Identifier: BUSL-1.1

//! configflux-6j91: the declared-facet constraint rule must reach the caller
//! as `E_FACET_VALUE_UNDECLARED`, not the generic ingest bucket.
//!
//! The rule itself (a constraint may name only DECLARED facets, ADR-0054 §5.2)
//! is enforced in `link_verify::validate_constraints` and its unit tests. What
//! this suite pins is the part the unit layer structurally cannot see: the
//! diagnostic CODE a caller receives. `product_api::map_graph_error` classifies
//! link/verify failures by substring, so the message text and the code are
//! coupled through prose — a reword that keeps every unit test green silently
//! demotes the violation to `E_COMPILE_INPUT_INVALID`, which is exactly how
//! this defect arose. These tests fail loudly in that case.
//!
//! Black-box and end-to-end: everything runs through `compile_model`, the same
//! entry point `compiler compile` calls. The CLI adds nothing between that call
//! and the JSON it prints, so the codes asserted here are the codes the binary
//! reports.

use std::fs;
use std::path::{Path, PathBuf};

use compiler::product_api::{
    compile_model, CompileModelRequest, Diagnostic, OperationStatus, SourceManifestEntry,
    E_FACET_VALUE_UNDECLARED, PRODUCT_SCHEMA_VERSION,
};

/// The remedy `validate_constraints` puts in the message, verbatim. Declaring
/// the facet is the only fix that KEEPS the policy, so it has to be the one the
/// author is told about first.
const REMEDY: &str = "declare the facet with its value domain, or remove it from the constraint";

/// Shape (a): `arch` exists in the model only because a COMPONENT-level
/// condition mentions it. No `facets` block declares it.
const COMPONENT_CONDITION_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "constraints": {"pinned_arch": {"condition": "arch == 'x86'"}},
  "components": {
    "agent": {"type": "service", "condition": "arch == 'x86'", "params": {}}
  }
}"#;

/// Shape (b): `tier` exists only because a PARAMETER-OVERRIDE condition
/// mentions it. The two shapes are separated on purpose — they are the two
/// distinct ways the deleted declared-union-inferred name set used to admit an
/// undeclared facet, so each needs its own regression case even though
/// `validate_constraints` no longer reads conditions at all.
const PARAM_OVERRIDE_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "constraints": {"gold_tier_only": {"condition": "tier == 'gold'"}},
  "components": {
    "agent": {"type": "service", "params": {
      "timeout_ms": {"type": "integer", "value": 1000,
        "overrides": [{"condition": "tier == 'gold'", "value": 5000}]}
    }}
  }
}"#;

/// The sibling invariant in the same function: a value outside a CLOSED facet's
/// declared domain. It has always carried `E_FACET_VALUE_UNDECLARED`; asserting
/// it here through the identical compile path is what makes the two rules
/// provably equal at the surface rather than merely intended to be.
const CLOSED_DOMAIN_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {"region": {"values": ["eu", "us"], "default": "eu"}},
  "constraints": {"bad_region": {"condition": "region == 'mars'"}},
  "components": {
    "agent": {"type": "service", "params": {}}
  }
}"#;

/// Shape (a) with `arch` declared: byte-for-byte the same model otherwise.
/// Guards against a fix that over-corrects into rejecting the ordinary case,
/// where a facet is both declared and used as a selector — which is most real
/// models.
const DECLARED_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {"arch": {"values": ["x86", "arm"], "default": "x86"}},
  "constraints": {"pinned_arch": {"condition": "arch == 'x86'"}},
  "components": {
    "agent": {"type": "service", "condition": "arch == 'x86'", "params": {}}
  }
}"#;

/// Drive the real product compile path against one inline chunk, writing to a
/// fresh temp dir the caller owns.
fn compile_chunk(chunk: &str, output_dir: &Path) -> compiler::product_api::CompileResult {
    compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "00_definitions.json".to_string(),
            inline_content: chunk.to_string(),
        }],
        output_dir: Some(output_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    })
}

/// Assert the compile failed with exactly one diagnostic and hand it back.
///
/// Both the flat `diagnostics` list and the per-check `diagnostic_codes` are
/// checked: they are two separate fields of the JSON the CLI prints, and a
/// consumer may read either one.
fn sole_error_diagnostic(
    result: &compiler::product_api::CompileResult,
    expected_code: &str,
) -> Diagnostic {
    assert_eq!(
        result.status,
        OperationStatus::Error,
        "compile must fail: {:?}",
        result.verify_report
    );
    let diagnostics = &result.verify_report.diagnostics.diagnostics;
    assert_eq!(
        diagnostics.len(),
        1,
        "expected exactly one diagnostic, got {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].code, expected_code,
        "diagnostic code, full report: {:?}",
        result.verify_report
    );
    assert_eq!(
        result.verify_report.checks[0].diagnostic_codes,
        vec![expected_code.to_string()],
        "the per-check code list must agree with the diagnostic"
    );
    diagnostics[0].clone()
}

/// Nothing may reach the disk for a model that fails validation: the package
/// would carry a policy the options surface cannot enforce.
fn assert_nothing_emitted(result: &compiler::product_api::CompileResult, output_dir: &Path) {
    assert!(
        result.compiled_model_package_ref.is_none(),
        "no package ref may be returned for a failed compile"
    );
    let entries: Vec<PathBuf> = fs::read_dir(output_dir)
        .expect("read output dir")
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    assert!(
        entries.is_empty(),
        "output dir must be untouched, found: {entries:?}"
    );
}

/// The message has to name the constraint, the offending facet, and the remedy.
/// Any one of the three missing leaves the author without a next step.
fn assert_names_constraint_facet_and_remedy(
    diagnostic: &Diagnostic,
    constraint_id: &str,
    facet: &str,
) {
    let msg = &diagnostic.message;
    assert!(
        msg.contains(&format!("Constraint '{constraint_id}'")),
        "message must name the constraint: {msg}"
    );
    assert!(
        msg.contains(&format!("facet '{facet}'")),
        "message must name the offending facet: {msg}"
    );
    assert!(
        msg.contains("not declared"),
        "message must say the facet is not declared: {msg}"
    );
    assert!(msg.contains(REMEDY), "message must carry the remedy: {msg}");
    // Not a closed-domain violation, so it must not claim to be one — that
    // phrase is the sibling rule's, and borrowing it would make the message
    // false even though the code is now shared.
    assert!(
        !msg.contains("closed facet"),
        "message must not claim a closed-domain violation: {msg}"
    );
}

#[test]
fn a_constraint_over_a_component_condition_facet_is_coded_facet_value_undeclared() {
    let output_dir = tempdir_for("component-condition");

    let result = compile_chunk(COMPONENT_CONDITION_CHUNK, &output_dir);
    let diagnostic = sole_error_diagnostic(&result, E_FACET_VALUE_UNDECLARED);
    assert_names_constraint_facet_and_remedy(&diagnostic, "pinned_arch", "arch");
    assert_nothing_emitted(&result, &output_dir);

    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn a_constraint_over_a_param_override_facet_is_coded_facet_value_undeclared() {
    let output_dir = tempdir_for("param-override");

    let result = compile_chunk(PARAM_OVERRIDE_CHUNK, &output_dir);
    let diagnostic = sole_error_diagnostic(&result, E_FACET_VALUE_UNDECLARED);
    assert_names_constraint_facet_and_remedy(&diagnostic, "gold_tier_only", "tier");
    assert_nothing_emitted(&result, &output_dir);

    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn the_sibling_closed_domain_rule_carries_the_same_code_through_the_same_path() {
    let output_dir = tempdir_for("closed-domain");

    let result = compile_chunk(CLOSED_DOMAIN_CHUNK, &output_dir);
    let diagnostic = sole_error_diagnostic(&result, E_FACET_VALUE_UNDECLARED);
    let msg = &diagnostic.message;
    assert!(msg.contains("Constraint 'bad_region'"), "msg: {msg}");
    assert!(msg.contains("closed facet 'region'"), "msg: {msg}");
    assert_nothing_emitted(&result, &output_dir);

    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn declaring_the_facet_makes_the_same_model_compile() {
    let output_dir = tempdir_for("declared");

    let result = compile_chunk(DECLARED_CHUNK, &output_dir);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "a constraint over a declared facet must compile: {:?}",
        result.verify_report
    );
    assert!(
        result.compiled_model_package_ref.is_some(),
        "a successful compile must emit a package"
    );

    fs::remove_dir_all(&output_dir).ok();
}

// Collision-proof temp-dir naming shared with the other `compiler/tests/`
// crates; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
