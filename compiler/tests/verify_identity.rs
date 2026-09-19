// SPDX-License-Identifier: BUSL-1.1

//! `verify` and `compile` report ONE model identity (ADR-0056 Amendment 2,
//! configflux-3ukw).
//!
//! Before this suite, `VerifyReport.model_hash` meant two different things
//! depending on which command filled it: `compile`'s success path assigned the
//! CMP identity (`index.config_hash`), while standalone `verify` — and every
//! `compile` error path — published the source-manifest digest under the same
//! name. Two values, one name, chosen by the verb. Amendment 2 makes `verify`
//! build the index in memory and report the real identity, and makes the field
//! ABSENT wherever no index was built.
//!
//! Black box: everything goes through the public product API, and the absence
//! assertions read the SERIALIZED envelope rather than the Rust field, because
//! "the key is not in the JSON" is the property a consumer can observe and an
//! `Option` field alone does not witness it.
//!
//! **What this suite does NOT pin, and where that lives.** Path-spelling
//! invariance of the reported identity is `//compiler:source_digest_test` (T4,
//! over the ADR-0056 A9 pack); that the identity is still the value that
//! shipped is `//compiler:scenario_byte_stability_test`; that `compile` and
//! `link` write the same package is `//compiler:link_oracle_test`.

use compiler::product_api::{
    compile_model, verify_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    VerifyModelRequest, PRODUCT_SCHEMA_VERSION,
};
use serde_json::Value as JsonValue;

#[path = "temp_dirs.rs"]
mod temp_dirs;

/// One pack of the corpus: the `--source` chunks, under the ids the CLI would
/// give them.
struct Pack {
    label: &'static str,
    chunks: &'static [(&'static str, &'static str)],
}

/// A scenario pack's two chunks, under the paths the scenario tests use.
macro_rules! scenario {
    ($root:literal) => {
        &[
            (
                concat!("scenarios/", $root, "/cue/00_definitions.json"),
                include_str!(concat!("../scenarios/", $root, "/cue/00_definitions.json")),
            ),
            (
                concat!("scenarios/", $root, "/cue/10_components.json"),
                include_str!(concat!("../scenarios/", $root, "/cue/10_components.json")),
            ),
        ]
    };
}

/// One chunk of an example, named by its file.
macro_rules! example {
    ($dir:literal, $file:literal) => {
        (
            concat!("examples/", $dir, "/", $file),
            include_str!(concat!("../../examples/", $dir, "/", $file)),
        )
    };
}

/// An example laid out as the two-chunk pack most of them are.
macro_rules! example_pack {
    ($label:literal, $dir:literal) => {
        Pack {
            label: $label,
            chunks: &[
                example!($dir, "00_definitions.json"),
                example!($dir, "10_components.json"),
            ],
        }
    };
}

/// The corpus configflux-3ukw pins the identity over: every scenario smoke pack
/// and every shipped example. Deliberately the shapes an author meets — one
/// unit and many, a flat single-file pack, and the four-unit polyrepo whose
/// `--source` order is not unit-name order.
const CORPUS: &[Pack] = &[
    Pack { label: "s1-smoke", chunks: scenario!("s1_water_pump/smoke") },
    Pack { label: "s2-smoke", chunks: scenario!("s2_wind_turbine/smoke") },
    Pack { label: "s3-smoke", chunks: scenario!("s3_automation_cell/smoke") },
    Pack { label: "s4-smoke", chunks: scenario!("s4_mobile_robot/smoke") },
    Pack { label: "s5-smoke", chunks: scenario!("s5_building_hvac/smoke") },
    example_pack!("example-00", "00-service-multi-env"),
    Pack { label: "example-01", chunks: &[example!("01-hello-led", "config.json")] },
    example_pack!("example-02", "02-sensor-gateway"),
    example_pack!("example-03", "03-motor-controller"),
    example_pack!("example-04", "04-fleet-edge-node"),
    example_pack!("example-05", "05-compose-fleet"),
    Pack {
        label: "example-06",
        chunks: &[
            example!("06-catalogue-polyrepo", "repos/catalogue/00_catalogue.json"),
            example!("06-catalogue-polyrepo", "repos/vision/10_vision.json"),
            example!("06-catalogue-polyrepo", "repos/compute/10_compute.json"),
            example!("06-catalogue-polyrepo", "repos/sorter/20_sorter.json"),
        ],
    },
];

fn manifest(chunks: &[(&str, &str)]) -> Vec<SourceManifestEntry> {
    chunks
        .iter()
        .map(|(source_id, content)| SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*content).to_string(),
        })
        .collect()
}

/// Every key in `value`, recursively, so a `model_hash` surviving inside a
/// nested check or the diagnostics report is caught too.
fn collect_keys(value: &JsonValue, out: &mut Vec<String>) {
    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                out.push(key.clone());
                collect_keys(child, out);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_keys(item, out);
            }
        }
        _ => {}
    }
}

fn assert_no_model_hash_key(label: &str, envelope: &JsonValue) {
    let mut keys = Vec::new();
    collect_keys(envelope, &mut keys);
    assert!(
        !keys.iter().any(|k| k == "model_hash"),
        "[{label}] a failing report still carries a model_hash key: {keys:?}"
    );
}

// ----------------------------------------------------------------------------
// T1 — verify reports the identity compile emits, over the whole corpus
// ----------------------------------------------------------------------------

/// R1. For every committed pack, `verify`'s `model_hash` is present and
/// bit-identical to the `model_hash` `compile` emits for the same sources.
///
/// This is also the corpus proof for R4's assumption: `verify` now runs the
/// link stages `compile` runs, and the ADR-0056 Amendment 2 D4 claim is that on
/// a complete model they cannot fail where the complete-model checks passed. A
/// pack that reached the `status` assertion below with an error is that claim
/// being false — the configflux-3ukw HARD STOP — and must be reported rather
/// than special-cased.
#[test]
fn t1_verify_reports_the_identity_compile_emits_for_every_pack() {
    for pack in CORPUS {
        let report = verify_model(VerifyModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: manifest(pack.chunks),
        });
        assert_eq!(
            report.status,
            OperationStatus::Ok,
            "[{}] verify failed on a committed pack: {:?}",
            pack.label,
            report.diagnostics.diagnostics
        );
        let verified = report.model_hash.clone().unwrap_or_else(|| {
            panic!("[{}] verify succeeded but reported no model_hash", pack.label)
        });

        let out = temp_dirs::unique_temp_dir("verify-identity", pack.label);
        let compiled = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: manifest(pack.chunks),
            output_dir: Some(out.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });
        assert_eq!(
            compiled.status,
            OperationStatus::Ok,
            "[{}] compile failed: {:?}",
            pack.label,
            compiled.verify_report.diagnostics.diagnostics
        );

        assert_eq!(
            verified, compiled.model_hash,
            "[{}] verify reports a different model identity than compile emits",
            pack.label
        );
        assert_eq!(
            compiled.verify_report.model_hash.as_deref(),
            Some(compiled.model_hash.as_str()),
            "[{}] compile's embedded verify report disagrees with its own envelope",
            pack.label
        );

        std::fs::remove_dir_all(&out).ok();
    }
}

/// The reported identity still discriminates. Without this, T1 would be
/// satisfied by any constant both commands happened to agree on.
///
/// The edit is a changed parameter VALUE, not reformatting: the chunk address
/// is over the chunk's canonical content (ADR-0056 Amendment 1), so respacing
/// the same model is correctly not a change and would not falsify anything.
#[test]
fn t1b_the_reported_identity_moves_when_the_model_changes() {
    let pack = CORPUS
        .iter()
        .find(|pack| pack.label == "example-01")
        .expect("example-01 is in the corpus");
    let baseline = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest(pack.chunks),
    });
    assert_eq!(baseline.status, OperationStatus::Ok, "baseline pack failed to verify");

    let edited_content = pack.chunks[0].1.replacen("\"value\": 500", "\"value\": 501", 1);
    assert_ne!(edited_content, pack.chunks[0].1, "edit did not change the chunk");
    let mut edited = manifest(pack.chunks);
    edited[0].inline_content = edited_content;

    let changed = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: edited,
    });

    assert_eq!(changed.status, OperationStatus::Ok, "edited pack failed to verify");
    assert_ne!(
        baseline.model_hash, changed.model_hash,
        "the reported identity did not move when a parameter value changed"
    );
}

// ----------------------------------------------------------------------------
// T2 — a failing verify publishes no identity
// ----------------------------------------------------------------------------

/// R2. On every failing path of `verify` the `model_hash` key is absent from
/// the serialized report — not empty, not a placeholder, not the source digest
/// it used to carry.
#[test]
fn t2_a_failing_verify_omits_the_model_hash_key() {
    for (label, request) in failing_verify_requests() {
        let report = verify_model(request);
        assert_eq!(
            report.status,
            OperationStatus::Error,
            "[{label}] expected this model to fail verification"
        );
        assert_eq!(report.model_hash, None, "[{label}] a failed verify reported an identity");

        let envelope: JsonValue =
            serde_json::to_value(&report).expect("serialize verify report");
        assert_no_model_hash_key(label, &envelope);
    }
}

// ----------------------------------------------------------------------------
// T3 — a failing compile publishes no identity in its embedded verify report
// ----------------------------------------------------------------------------

/// R3. `compile`'s embedded `verify_report` omits `model_hash` on every error
/// path. `CompileResult.model_hash` (the envelope's own field) is untouched by
/// this change — ADR-0056 Amendment 2 D3 — so the assertion is scoped to the
/// nested report rather than to the whole envelope.
#[test]
fn t3_a_failing_compile_omits_model_hash_from_its_verify_report() {
    for (label, sources) in failing_source_manifests() {
        let out = temp_dirs::unique_temp_dir("verify-identity-fail", label);
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: sources,
            output_dir: Some(out.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });
        assert_eq!(
            result.status,
            OperationStatus::Error,
            "[{label}] expected this model to fail compilation"
        );
        assert_eq!(
            result.verify_report.model_hash, None,
            "[{label}] a failed compile reported an identity in its verify report"
        );

        let envelope: JsonValue = serde_json::to_value(&result).expect("serialize compile result");
        let report = envelope
            .get("verify_report")
            .expect("compile result carries a verify_report");
        assert_no_model_hash_key(label, report);

        std::fs::remove_dir_all(&out).ok();
    }
}

/// The same for the schema-version refusal, which `compile` answers before it
/// builds anything at all.
#[test]
fn t3b_a_schema_version_refusal_omits_model_hash_from_its_verify_report() {
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION - 1,
        source_manifest: manifest(CORPUS[0].chunks),
        output_dir: None,
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(result.status, OperationStatus::Error, "an old schema version was accepted");
    assert_eq!(result.verify_report.model_hash, None);

    let envelope: JsonValue = serde_json::to_value(&result).expect("serialize compile result");
    assert_no_model_hash_key(
        "compile/schema-version",
        envelope.get("verify_report").expect("verify_report"),
    );
}

// ----------------------------------------------------------------------------
// Failing fixtures
// ----------------------------------------------------------------------------

/// A model whose sole component depends on a component nothing declares: valid
/// JSON, ingested without complaint, refused by the complete-model checks.
const DANGLING_DEPENDENCY: &str = r#"{
  "package": "verify_identity_probe",
  "version": "1.0.0",
  "definitions": {},
  "components": {
    "led_driver": {
      "type": "module",
      "depends_on": ["no_such_component"],
      "params": {}
    }
  }
}"#;

/// The three classes of failure `verify` can report, one per arm of
/// `verify_model`: the schema-version refusal, an ingest refusal, and a
/// complete-model refusal.
fn failing_verify_requests() -> Vec<(&'static str, VerifyModelRequest)> {
    let mut out = vec![(
        "verify/schema-version",
        VerifyModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION - 1,
            source_manifest: manifest(CORPUS[0].chunks),
        },
    )];
    for (label, source_manifest) in failing_source_manifests() {
        out.push((
            label,
            VerifyModelRequest { schema_version: PRODUCT_SCHEMA_VERSION, source_manifest },
        ));
    }
    out
}

/// The two failing manifests both verbs share: one that cannot be ingested and
/// one that ingests but cannot pass the complete-model checks.
fn failing_source_manifests() -> Vec<(&'static str, Vec<SourceManifestEntry>)> {
    vec![
        (
            "ingest",
            vec![SourceManifestEntry {
                source_id: "broken.json".to_string(),
                inline_content: "{ this is not json".to_string(),
            }],
        ),
        (
            "graph",
            vec![SourceManifestEntry {
                source_id: "dangling.json".to_string(),
                inline_content: DANGLING_DEPENDENCY.to_string(),
            }],
        ),
    ]
}
