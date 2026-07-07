// SPDX-License-Identifier: BUSL-1.1

use crate::product_api::{
    inspect_model, InspectModelRequest, InspectQuery, InspectionItem, InspectionResult,
    OperationStatus, SourceManifestEntry, E_INSPECT_QUERY_INVALID, E_INSPECT_UNKNOWN_PARAMETER,
    E_INSPECT_UNKNOWN_SCOPE, PRODUCT_SCHEMA_VERSION,
};
use anyhow::{Context, Result};
use serde_json::Value as JsonValue;

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
const S1_GOLDEN_INSPECT_PARAMETER: &str = include_str!(
    "../scenarios/s1_water_pump/smoke/golden/inspect.parameter.thermal_control.control_driver.json"
);

const S3_SOURCE_DEFS: &str = "scenarios/s3_automation_cell/smoke/chunks/00_definitions.toml";
const S3_SOURCE_COMPONENTS: &str = "scenarios/s3_automation_cell/smoke/chunks/10_components.toml";
const S3_CHUNK_DEFS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/00_definitions.json");
const S3_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/10_components.json");
const S3_GOLDEN_INSPECT_SCOPED_STATS: &str = include_str!(
    "../scenarios/s3_automation_cell/smoke/golden/inspect.scoped_stats.component_swift_ring_standard.json"
);

fn inspect_with_chunks(query: InspectQuery, chunks: &[(&str, &str)]) -> InspectionResult {
    inspect_model(InspectModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: chunks
            .iter()
            .map(|(source_id, inline_content)| SourceManifestEntry {
                source_id: (*source_id).to_string(),
                inline_content: (*inline_content).to_string(),
            })
            .collect(),
        query,
    })
}

fn with_dynamic_model_hash(template: &str, model_hash: &str) -> String {
    template.replace("__DYNAMIC_MODEL_HASH__", model_hash)
}

fn parse_json(content: &str) -> Result<JsonValue> {
    serde_json::from_str(content).context("Failed to parse JSON")
}

#[test]
fn loop9_contract_inspect_parameter_envelope_has_required_fields() {
    let result = inspect_with_chunks(
        InspectQuery::Parameter {
            component_id: "thermal_control".to_string(),
            param_key: "control_driver".to_string(),
        },
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
    );

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert!(!result.model_hash.is_empty());
    assert_eq!(result.error_count, 0);
    assert_eq!(result.warning_count, 0);
    assert_eq!(result.diagnostics.error_count, 0);
    assert_eq!(result.diagnostics.warning_count, 0);
    assert!(matches!(
        result.item,
        Some(InspectionItem::Parameter {
            ref component_id,
            ref param_key,
            ..
        }) if component_id == "thermal_control" && param_key == "control_driver"
    ));
}

#[test]
fn loop9_contract_inspect_scoped_stats_envelope_has_required_fields() {
    let result = inspect_with_chunks(
        InspectQuery::ScopedStats {
            scope: "component:swift_ring_standard".to_string(),
        },
        &[
            (S3_SOURCE_DEFS, S3_CHUNK_DEFS),
            (S3_SOURCE_COMPONENTS, S3_CHUNK_COMPONENTS),
        ],
    );

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert!(!result.model_hash.is_empty());
    assert_eq!(result.error_count, 0);
    assert_eq!(result.warning_count, 0);
    assert_eq!(result.diagnostics.error_count, 0);
    assert_eq!(result.diagnostics.warning_count, 0);
    assert!(matches!(
        result.item,
        Some(InspectionItem::ScopedStats {
            ref scope,
            component_count,
            parameter_count,
            ..
        }) if scope == "component:swift_ring_standard" && component_count == 2 && parameter_count == 2
    ));
}

#[test]
fn loop9_golden_s1_parameter_query_matches() -> Result<()> {
    let result = inspect_with_chunks(
        InspectQuery::Parameter {
            component_id: "thermal_control".to_string(),
            param_key: "control_driver".to_string(),
        },
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
    );
    assert_eq!(result.status, OperationStatus::Ok);

    let expected = with_dynamic_model_hash(S1_GOLDEN_INSPECT_PARAMETER, &result.model_hash);
    let expected_json = parse_json(&expected)?;
    let actual_json = serde_json::to_value(result).context("Failed to serialize result")?;
    assert_eq!(actual_json, expected_json);
    Ok(())
}

#[test]
fn loop9_golden_s3_scoped_stats_query_matches() -> Result<()> {
    let result = inspect_with_chunks(
        InspectQuery::ScopedStats {
            scope: "component:swift_ring_standard".to_string(),
        },
        &[
            (S3_SOURCE_DEFS, S3_CHUNK_DEFS),
            (S3_SOURCE_COMPONENTS, S3_CHUNK_COMPONENTS),
        ],
    );
    assert_eq!(result.status, OperationStatus::Ok);

    let expected = with_dynamic_model_hash(S3_GOLDEN_INSPECT_SCOPED_STATS, &result.model_hash);
    let expected_json = parse_json(&expected)?;
    let actual_json = serde_json::to_value(result).context("Failed to serialize result")?;
    assert_eq!(actual_json, expected_json);
    Ok(())
}

#[test]
fn loop9_mutation_unknown_parameter_scope_and_invalid_payload_emit_stable_codes() {
    let unknown_parameter = inspect_with_chunks(
        InspectQuery::Parameter {
            component_id: "thermal_control".to_string(),
            param_key: "missing_parameter".to_string(),
        },
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
    );
    assert_eq!(unknown_parameter.status, OperationStatus::Error);
    assert_eq!(
        unknown_parameter.diagnostics.diagnostics[0].code,
        E_INSPECT_UNKNOWN_PARAMETER.to_string()
    );

    let unknown_scope = inspect_with_chunks(
        InspectQuery::ScopedStats {
            scope: "component:missing_component".to_string(),
        },
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
    );
    assert_eq!(unknown_scope.status, OperationStatus::Error);
    assert_eq!(
        unknown_scope.diagnostics.diagnostics[0].code,
        E_INSPECT_UNKNOWN_SCOPE.to_string()
    );

    let invalid_payload = inspect_with_chunks(
        InspectQuery::ScopedStats {
            scope: "component:Bad_Name".to_string(),
        },
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
    );
    assert_eq!(invalid_payload.status, OperationStatus::Error);
    assert_eq!(
        invalid_payload.diagnostics.diagnostics[0].code,
        E_INSPECT_QUERY_INVALID.to_string()
    );

    let invalid_parameter_payload = inspect_with_chunks(
        InspectQuery::Parameter {
            component_id: "thermal_control".to_string(),
            param_key: "BadParam".to_string(),
        },
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
    );
    assert_eq!(invalid_parameter_payload.status, OperationStatus::Error);
    assert_eq!(
        invalid_parameter_payload.diagnostics.diagnostics[0].code,
        E_INSPECT_QUERY_INVALID.to_string()
    );
}

#[test]
fn loop9_determinism_inspect_payloads_are_byte_stable() -> Result<()> {
    let first = inspect_with_chunks(
        InspectQuery::ScopedStats {
            scope: "component:swift_ring_standard".to_string(),
        },
        &[
            (S3_SOURCE_DEFS, S3_CHUNK_DEFS),
            (S3_SOURCE_COMPONENTS, S3_CHUNK_COMPONENTS),
        ],
    );
    let second = inspect_with_chunks(
        InspectQuery::ScopedStats {
            scope: "component:swift_ring_standard".to_string(),
        },
        &[
            (S3_SOURCE_DEFS, S3_CHUNK_DEFS),
            (S3_SOURCE_COMPONENTS, S3_CHUNK_COMPONENTS),
        ],
    );

    assert_eq!(first.status, OperationStatus::Ok);
    assert_eq!(second.status, OperationStatus::Ok);
    let first_json = serde_json::to_string(&first).context("Failed to serialize first result")?;
    let second_json =
        serde_json::to_string(&second).context("Failed to serialize second result")?;
    assert_eq!(first_json, second_json);

    Ok(())
}
