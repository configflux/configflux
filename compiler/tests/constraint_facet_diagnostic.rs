// SPDX-License-Identifier: BUSL-1.1

//! configflux-6j91: the declared-facet constraint rule must reach the caller
//! as `E_FACET_VALUE_UNDECLARED`, not the generic ingest bucket.
//!
//! The rule itself (a constraint may name only DECLARED facets, ADR-0054 §5.2)
//! is enforced in `link_verify::validate_constraints` and its unit tests. What
//! this suite pins is the part the unit layer structurally cannot see: the
//! diagnostic CODE and remedy a caller receives. The unit tests see the message
//! only, so a change that leaves the message intact and the code wrong passes
//! every one of them — which is exactly how this defect arose. These tests fail
//! loudly in that case. (When they were written the code was recovered FROM the
//! message, so a reword alone could demote the violation; configflux-py7w moved
//! the code onto the refusal, and what is left to guard is the rule reporting
//! the code and remedy it is specified to carry.)
//!
//! Black-box and end-to-end: everything runs through `compile_model`, the same
//! entry point `compiler compile` calls. The CLI adds nothing between that call
//! and the JSON it prints, so the codes asserted here are the codes the binary
//! reports.

use std::fs;
use std::path::{Path, PathBuf};

use compiler::product_api::{
    compile_model, verify_model, CompileModelRequest, Diagnostic, OperationStatus,
    SourceManifestEntry, VerifyModelRequest, VerifyReport, E_COMPILE_INPUT_INVALID,
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

// ----------------------------------------------------------------------------
// configflux-secb.2 / ADR-0057 §D5: facet-to-facet comparison operands.
//
// Both sides of `a == b` must be DECLARED, by the same ADR-0054 §5.2 rule that
// governs a predicate tag. The two sides carry different messages on purpose,
// and different codes — which is exactly what this suite exists to pin. The
// left side is an ordinary tag position and carries `E_FACET_VALUE_UNDECLARED`
// with the declare-the-facet remedy. The right side is new: an author who writes
// `region == prod` most likely meant the literal and forgot the quotes, so the
// message says so and the rule carries no code of its own, landing in the
// generic input bucket rather than borrowing a facet-value code that would be
// false.
// ----------------------------------------------------------------------------

/// An unquoted right-hand side naming nothing declared. `line_container` is
/// declared; `sorter_container` is not.
const UNDECLARED_COMPARAND_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {"line_container": {"values": ["c1", "c2"], "default": "c1"}},
  "constraints": {"groups_equal": {"condition": "line_container == sorter_container"}},
  "components": {
    "agent": {"type": "service", "params": {}}
  }
}"#;

/// The mirror case: the LEFT side is undeclared. Same rule, existing message.
const UNDECLARED_COMPARISON_LEFT_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {"sorter_container": {"values": ["c1", "c2"], "default": "c1"}},
  "constraints": {"groups_equal": {"condition": "line_container == sorter_container"}},
  "components": {
    "agent": {"type": "service", "params": {}}
  }
}"#;

/// Both declared: the ordinary case must keep compiling. Guards against a fix
/// that over-corrects into rejecting every comparison.
const DECLARED_COMPARISON_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "line_container": {"values": ["c1", "c2"], "default": "c1"},
    "sorter_container": {"values": ["c1", "c2"], "default": "c1"}
  },
  "constraints": {"groups_equal": {"condition": "line_container == sorter_container"}},
  "components": {
    "agent": {"type": "service", "params": {}}
  }
}"#;

#[test]
fn an_undeclared_comparison_right_hand_side_is_coded_compile_input_invalid() {
    let output_dir = tempdir_for("undeclared-comparand");

    let result = compile_chunk(UNDECLARED_COMPARAND_CHUNK, &output_dir);
    let diagnostic = sole_error_diagnostic(&result, E_COMPILE_INPUT_INVALID);
    let msg = &diagnostic.message;
    assert!(
        msg.contains("constraint 'groups_equal'"),
        "message must name the constraint: {msg}"
    );
    assert!(
        msg.contains("right-hand side 'sorter_container' is not a declared facet or binding"),
        "message must name the offending identifier: {msg}"
    );
    assert!(
        msg.contains("quote it to compare against a literal"),
        "message must offer the quoting remedy: {msg}"
    );
    assert_nothing_emitted(&result, &output_dir);

    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn an_undeclared_comparison_left_hand_side_keeps_the_facet_value_undeclared_code() {
    let output_dir = tempdir_for("undeclared-comparison-left");

    let result = compile_chunk(UNDECLARED_COMPARISON_LEFT_CHUNK, &output_dir);
    let diagnostic = sole_error_diagnostic(&result, E_FACET_VALUE_UNDECLARED);
    assert_names_constraint_facet_and_remedy(&diagnostic, "groups_equal", "line_container");
    assert_nothing_emitted(&result, &output_dir);

    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn a_comparison_between_two_declared_facets_compiles() {
    let output_dir = tempdir_for("declared-comparison");

    let result = compile_chunk(DECLARED_COMPARISON_CHUNK, &output_dir);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "a comparison between two declared facets must compile: {:?}",
        result.verify_report
    );
    assert!(
        result.compiled_model_package_ref.is_some(),
        "a successful compile must emit a package"
    );

    fs::remove_dir_all(&output_dir).ok();
}

// ----------------------------------------------------------------------------
// configflux-xcrb: a facet `default` outside its own declared `values`.
//
// The third rule to reach a caller as `E_FACET_VALUE_UNDECLARED`, and the one
// that was landing in the generic bucket. The code's registry line already
// promised "a value that is not in a closed facet's exhaustively declared
// domain" — and a default IS such a value, read off the facet rather than off a
// condition — while the refusal carried no code at all, so the caller was told
// `E_COMPILE_INPUT_INVALID`.
//
// Driven through BOTH product entry points on purpose. `validate_facets` sits
// on the shared link/verify path, so `compiler verify` and `compiler compile`
// must report the identical code, remedy and text; a fix that reached only the
// compile mapper would leave `verify` promising something else for one model.
// ----------------------------------------------------------------------------

/// `mars` is a legal symbol and a legal value token — it is simply not one of
/// the two values `region` declares. Nothing else in the model is wrong, so the
/// facet-default rule is the sole refusal.
const FACET_DEFAULT_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {"region": {"values": ["eu", "us"], "default": "mars"}},
  "components": {
    "agent": {"type": "service", "params": {}}
  }
}"#;

/// The DEFAULT remedy of `E_FACET_VALUE_UNDECLARED` (`product_api::hint_for`),
/// which is the one this rule is specified to carry: a default outside the
/// domain is fixed exactly the way any other undeclared value is. Written out
/// as a literal rather than borrowed from the compiler, so this agrees with the
/// shipped text rather than with any rewording of itself.
const FACET_VALUE_UNDECLARED_HINT: &str = "Add the value to the facet's `values`, mark the facet \
                                           `open: true`, or fix the condition to use a declared \
                                           value";

/// Drive the real product verify path against one inline chunk. The compile
/// twin above writes a package; this one never touches the disk, which is part
/// of why both are driven.
fn verify_chunk(chunk: &str) -> VerifyReport {
    verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "00_definitions.json".to_string(),
            inline_content: chunk.to_string(),
        }],
    })
}

/// Assert the verify failed with exactly one diagnostic and hand it back.
///
/// Mirrors `sole_error_diagnostic` above over the verify report, including the
/// per-check code list: they are two separate fields of the JSON the CLI
/// prints, and a consumer may branch on either one.
fn sole_verify_diagnostic(report: &VerifyReport, expected_code: &str) -> Diagnostic {
    assert_eq!(
        report.status,
        OperationStatus::Error,
        "verify must fail: {report:?}"
    );
    let diagnostics = &report.diagnostics.diagnostics;
    assert_eq!(
        diagnostics.len(),
        1,
        "expected exactly one diagnostic, got {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].code, expected_code,
        "diagnostic code, full report: {report:?}"
    );
    assert_eq!(
        report.checks[0].diagnostic_codes,
        vec![expected_code.to_string()],
        "the per-check code list must agree with the diagnostic"
    );
    diagnostics[0].clone()
}

/// The message has to name the facet, the rejected default and the domain it is
/// missing from; the remedy has to be the code's own. A code is only actionable
/// alongside text that says which declaration to go and fix.
fn assert_names_facet_default_and_domain(diagnostic: &Diagnostic, case: &str) {
    let msg = &diagnostic.message;
    assert!(
        msg.contains("Facet 'region'"),
        "{case}: message must name the facet: {msg}"
    );
    assert!(
        msg.contains("default 'mars'"),
        "{case}: message must name the rejected default: {msg}"
    );
    assert!(
        msg.contains("eu, us"),
        "{case}: message must name the declared domain: {msg}"
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some(FACET_VALUE_UNDECLARED_HINT),
        "{case}: the rule carries its code's default remedy"
    );
}

#[test]
fn a_facet_default_outside_its_declared_values_is_coded_facet_value_undeclared() {
    let report = verify_chunk(FACET_DEFAULT_CHUNK);
    let verified = sole_verify_diagnostic(&report, E_FACET_VALUE_UNDECLARED);
    assert_names_facet_default_and_domain(&verified, "verify");

    let output_dir = tempdir_for("facet-default");
    let result = compile_chunk(FACET_DEFAULT_CHUNK, &output_dir);
    let compiled = sole_error_diagnostic(&result, E_FACET_VALUE_UNDECLARED);
    assert_names_facet_default_and_domain(&compiled, "compile");
    assert_eq!(
        compiled.message, verified.message,
        "verify and compile must report the same refusal, not two spellings of it"
    );
    assert_nothing_emitted(&result, &output_dir);

    fs::remove_dir_all(&output_dir).ok();
}

// Collision-proof temp-dir naming shared with the other `compiler/tests/`
// crates; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
