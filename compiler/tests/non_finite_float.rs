// SPDX-License-Identifier: BUSL-1.1

//! configflux-2yiq: a non-finite float must never reach a hash pre-image.
//!
//! TOML 1.0 admits `nan`, `+nan`, `-nan`, `inf`, `+inf` and `-inf` as float
//! literals, and `schema::Value::Float(f64)` accepts every one of them. Every
//! hash pre-image in this compiler is canonical JSON built with
//! `serde_json::to_value`, and `serde_json::Number::from_f64` returns `None`
//! for exactly those values — so the number is written as `null`. Three
//! distinct authored models then canonicalize to identical bytes, and the value
//! is destroyed inside `resolved_output`, the largest member of both
//! `resolve_hash` recipes.
//!
//! MEASURED before the fix, not reasoned: the three chunks below, differing
//! only in `nan` / `inf` / `-inf`, each compiled successfully and all three
//! reported the same `model_hash`,
//! `f168c450044a89ca9e3d7cc1db9d70a7e6e2232ce873edd4d5d574097a774c0f`.
//!
//! Black-box and end-to-end: everything runs through `compile_model`, the same
//! entry point `compiler compile` calls, so the codes asserted here are the
//! codes the binary reports.

use std::fs;
use std::path::{Path, PathBuf};

use compiler::product_api::{
    compile_model, CompileModelRequest, CompileResult, Diagnostic, OperationStatus,
    SourceManifestEntry, E_CATALOGUE_INVALID, E_COMPILE_INPUT_INVALID, PRODUCT_SCHEMA_VERSION,
};
use compiler::verify_ir_dir;

/// Every spelling TOML 1.0 gives a non-finite float. All six are one rule, and
/// a fix that caught only the unsigned pair would leave the collapse reachable.
const NON_FINITE_LITERALS: [&str; 6] = ["nan", "+nan", "-nan", "inf", "+inf", "-inf"];

/// A label safe to put in a filesystem path.
fn slug(literal: &str) -> String {
    literal.replace('+', "pos").replace('-', "neg")
}

/// One component parameter whose value is spelled `<literal>`.
fn param_chunk(literal: &str) -> String {
    format!(
        r#"
package = "p"
version = "1.0"

[components.pump.params.ratio]
type = "float"
value = {literal}
"#
    )
}

/// A definition — the other namespace a parameter lives in. Definitions are
/// merged into the same repository and reach the same encoder, so the rule has
/// to see them too.
fn definition_chunk(literal: &str) -> String {
    format!(
        r#"
package = "p"
version = "1.0"

[definitions.speed]
type = "float"
value = {literal}
"#
    )
}

/// A finite base value with a non-finite OVERRIDE payload. The override is what
/// the resolver ends up writing, so a rule that read only the base value would
/// pass this model and emit the collapse anyway.
fn override_chunk(literal: &str) -> String {
    format!(
        r#"
package = "p"
version = "1.0"

[facets.region]
values = ["eu", "us"]
default = "eu"

[components.pump.params.ratio]
type = "float"
value = 1.0

[[components.pump.params.ratio.overrides]]
condition = "region == 'us'"
value = {literal}
"#
    )
}

/// A finite value with a non-finite LIMIT. `resolver::resolve_parameter` moves
/// `limits` onto the resolved parameter verbatim, so a bound rides into
/// `resolved_output` beside the value and collapses the same way.
fn limits_chunk(bound: &str, literal: &str) -> String {
    format!(
        r#"
package = "p"
version = "1.0"

[components.pump.params.ratio]
type = "float"
value = 1.0

[components.pump.params.ratio.limits]
{bound} = {literal}
"#
    )
}

/// A typed catalogue whose `height_mm` column holds `<literal>`.
fn catalogue_chunk(literal: &str) -> String {
    format!(
        r#"
package = "p"
version = "1.0"

[catalogues.containers.fields.height_mm]
type = "float"

[catalogues.containers.entries.c1]
height_mm = {literal}

[components.pump.params.ratio]
type = "float"
value = 1.0
"#
    )
}

fn compile_chunk(chunk: &str, output_dir: &Path) -> CompileResult {
    compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "00_definitions.toml".to_string(),
            inline_content: chunk.to_string(),
        }],
        output_dir: Some(output_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    })
}

/// Assert the compile failed with exactly one diagnostic carrying
/// `expected_code`, that nothing was written, and hand the diagnostic back.
///
/// Both the flat `diagnostics` list and the per-check `diagnostic_codes` are
/// checked: they are two separate fields of the JSON the CLI prints, and a
/// consumer may read either one.
fn sole_error_diagnostic(
    result: &CompileResult,
    expected_code: &str,
    output_dir: &Path,
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
    // Nothing may reach the disk for a model that fails validation: a package
    // carrying a null where a number was authored is exactly what this rule
    // exists to prevent from ever being addressable by hash.
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
    diagnostics[0].clone()
}

/// The message has to say what was refused and why, or the author is left
/// guessing which of their numbers the compiler dislikes.
fn assert_explains_the_refusal(diagnostic: &Diagnostic, needles: &[&str]) {
    let message = &diagnostic.message;
    for needle in needles {
        assert!(
            message.contains(needle),
            "message must contain {needle:?}: {message}"
        );
    }
    assert!(
        message.contains("is not a finite number"),
        "message must say the value is not finite: {message}"
    );
    assert!(
        message.contains("would record `null`"),
        "message must say what the silent outcome would have been: {message}"
    );
}

/// Compile `chunk`, expect the refusal, and check the message names the
/// offending place.
fn assert_rejected(chunk: &str, label: &str, code: &str, needles: &[&str]) {
    let output_dir = tempdir_for(label);
    let result = compile_chunk(chunk, &output_dir);
    let diagnostic = sole_error_diagnostic(&result, code, &output_dir);
    assert_explains_the_refusal(&diagnostic, needles);
    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn every_non_finite_spelling_in_a_component_parameter_is_rejected() {
    for literal in NON_FINITE_LITERALS {
        assert_rejected(
            &param_chunk(literal),
            &format!("param-{}", slug(literal)),
            E_COMPILE_INPUT_INVALID,
            &["Parameter 'components.pump.params.ratio'", "value"],
        );
    }
}

#[test]
fn every_non_finite_spelling_in_a_definition_is_rejected() {
    for literal in NON_FINITE_LITERALS {
        assert_rejected(
            &definition_chunk(literal),
            &format!("definition-{}", slug(literal)),
            E_COMPILE_INPUT_INVALID,
            &["Parameter 'definitions.speed'", "value"],
        );
    }
}

#[test]
fn a_non_finite_override_payload_is_rejected() {
    for literal in NON_FINITE_LITERALS {
        assert_rejected(
            &override_chunk(literal),
            &format!("override-{}", slug(literal)),
            E_COMPILE_INPUT_INVALID,
            &["Parameter 'components.pump.params.ratio.overrides[0]'"],
        );
    }
}

#[test]
fn a_non_finite_limit_is_rejected() {
    for bound in ["min", "max"] {
        assert_rejected(
            &limits_chunk(bound, "inf"),
            &format!("limits-{bound}"),
            E_COMPILE_INPUT_INVALID,
            &[
                "Parameter 'components.pump.params.ratio'",
                &format!("limits.{bound}"),
            ],
        );
    }
}

#[test]
fn every_non_finite_spelling_in_a_catalogue_entry_is_rejected() {
    for literal in NON_FINITE_LITERALS {
        assert_rejected(
            &catalogue_chunk(literal),
            &format!("catalogue-{}", slug(literal)),
            E_CATALOGUE_INVALID,
            &[
                "Catalogue 'containers'",
                "entry 'c1'",
                "field 'height_mm'",
            ],
        );
    }
}

/// The positive twin of the whole rule: finite floats still compile, and three
/// models that differ only in a finite float stay three distinguishable models.
/// This is precisely the property the collapse destroyed.
#[test]
fn distinct_finite_models_still_compile_to_distinct_model_hashes() {
    let mut hashes = Vec::new();
    for literal in ["1.0", "2.0", "-3.5"] {
        let output_dir = tempdir_for(&format!("finite-{}", slug(literal)));
        let result = compile_chunk(&param_chunk(literal), &output_dir);
        assert_eq!(
            result.status,
            OperationStatus::Ok,
            "a finite float must still compile: {:?}",
            result.verify_report
        );
        hashes.push(result.model_hash.clone());
        fs::remove_dir_all(&output_dir).ok();
    }
    hashes.sort();
    hashes.dedup();
    assert_eq!(
        hashes.len(),
        3,
        "three different finite values must produce three different model hashes"
    );
}

/// The catalogue rule must not over-correct into rejecting the ordinary case.
#[test]
fn a_finite_catalogue_entry_still_compiles() {
    let output_dir = tempdir_for("catalogue-finite");
    let result = compile_chunk(&catalogue_chunk("1000.0"), &output_dir);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "a finite catalogue field must still compile: {:?}",
        result.verify_report
    );
    fs::remove_dir_all(&output_dir).ok();
}

/// The one `chunk-<hash>.cfir` file a single-source package holds.
fn sole_chunk_path(output_dir: &Path) -> PathBuf {
    let mut chunks: Vec<PathBuf> = fs::read_dir(output_dir)
        .expect("read the package dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("chunk-") && name.ends_with(".cfir"))
        })
        .collect();
    assert_eq!(chunks.len(), 1, "expected exactly one chunk: {chunks:?}");
    chunks.pop().expect("the sole chunk")
}

/// The re-verification path is the check a caller applies to a package THIS
/// TOOLCHAIN DID NOT PRODUCE, so no package carrying a parameter value that is
/// not a finite number may pass it. `verify_ir_dir` keeps a second, hand-written
/// spelling of the compile path's whole-model check list, and the parameter half
/// of this rule was missing from it while the catalogue half — which rides along
/// with `validate_catalogues` — was not.
///
/// The layer that refuses the package TODAY is the decoder, not the finiteness
/// rule: chunk files are read with `serde_json`, JSON has no `inf` token, and an
/// overflowing literal (the nearest it can come to one) is refused as `number
/// out of range` before a value is built. That guard belongs to the decoder
/// rather than to this contract, so the rule now in the list stands behind it.
/// This test pins the property; the next pins the guard currently enforcing it.
#[test]
fn the_re_verification_path_refuses_a_non_finite_parameter_value() {
    let output_dir = tempdir_for("verify-ir-dir-non-finite");
    let result = compile_chunk(&param_chunk("2.5"), &output_dir);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "the finite model must compile: {:?}",
        result.verify_report
    );
    verify_ir_dir(&output_dir).expect("the package this compiler just wrote must verify");

    // Rewrite the sole chunk so its parameter carries the overflowing literal.
    // Nothing else moves — the file keeps its name, its `chunk_hash` field and
    // its `source_id` — so this is exactly the shape an outside producer could
    // hand over, and the package-integrity checks have nothing else to catch.
    let chunk_path = sole_chunk_path(&output_dir);
    let authored = fs::read_to_string(&chunk_path).expect("read the chunk");
    let forged = authored.replace("2.5", "1e400");
    assert_ne!(
        forged, authored,
        "the authored value must appear in the chunk: {authored}"
    );
    fs::write(&chunk_path, &forged).expect("write the forged chunk");

    let error = verify_ir_dir(&output_dir)
        .expect_err("a parameter value that is not a finite number must be refused");
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("Failed to parse IR chunk"),
        "the refusal must name the chunk it was reading: {rendered}"
    );
    assert!(
        rendered.contains("number out of range"),
        "the refusal must say the value is outside what a float can hold: {rendered}"
    );
    fs::remove_dir_all(&output_dir).ok();
}

/// The decoder guard that keeps the rule above out of reach through a package,
/// pinned rather than assumed. `1e400` is the case the two JSON-token spellings
/// miss: it is well-formed JSON, so only the range check refuses it.
#[test]
fn a_chunk_file_cannot_spell_a_non_finite_float() {
    for literal in ["nan", "inf", "NaN", "Infinity", "-Infinity", "1e400", "-1e400"] {
        assert!(
            serde_json::from_str::<serde_json::Value>(literal).is_err(),
            "JSON must not decode {literal} to a float"
        );
    }
}

/// The format asymmetry, asserted rather than assumed: JSON has no non-finite
/// literal, so the JSON ingest path refuses one at parse and never reaches the
/// rule above. CUE reaches this compiler as exported JSON, so the CUE authoring
/// front end is covered by the same refusal — TOML is the only authored format
/// that can carry the value far enough to need the rule.
#[test]
fn json_cannot_express_a_non_finite_float() {
    for literal in ["NaN", "Infinity", "-Infinity"] {
        let chunk = format!(
            r#"{{"package":"p","version":"1.0","components":{{"pump":{{"type":"actuator",
               "params":{{"ratio":{{"type":"float","value":{literal}}}}}}}}}}}"#
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(&chunk).is_err(),
            "JSON must have no literal for {literal}"
        );

        let output_dir = tempdir_for(&format!("json-{}", slug(literal)));
        let result = compile_chunk(&chunk, &output_dir);
        assert_eq!(
            result.status,
            OperationStatus::Error,
            "a JSON chunk spelling {literal} must be refused: {:?}",
            result.verify_report
        );
        assert_eq!(
            result.verify_report.diagnostics.diagnostics[0].code, E_COMPILE_INPUT_INVALID,
            "JSON refuses it at parse, one stage before the finiteness rule"
        );
        fs::remove_dir_all(&output_dir).ok();
    }
}

/// The mechanism behind the collapse, pinned directly so the reason the rule
/// exists cannot quietly stop being true.
#[test]
fn serde_json_flattens_every_non_finite_float_to_null() {
    for value in [f64::NAN, -f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            serde_json::to_value(value).expect("f64 always serializes"),
            serde_json::Value::Null,
            "serde_json must map {value} to null"
        );
    }
}

// Collision-proof temp-dir naming shared with the other `compiler/tests/`
// crates; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
