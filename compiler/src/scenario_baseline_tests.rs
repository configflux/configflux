// SPDX-License-Identifier: BUSL-1.1

use crate::conditions;
use crate::product_api::PRODUCT_SCHEMA_VERSION;
use crate::resolver::{resolve, ResolutionContext};
use crate::scenario_test_support::unique_temp_dir;
use crate::{verify_ir_dir, Compiler};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_SOURCE_MUTATION_UNKNOWN_DEP: &str =
    "scenarios/s1_water_pump/smoke/mutations/unknown_dependency.toml";
const S1_SOURCE_MUTATION_UNREACHABLE: &str =
    "scenarios/s1_water_pump/smoke/mutations/unreachable_branch.toml";

const S1_PROFILE: &str = include_str!("../scenarios/s1_water_pump/smoke/profile.toml");
const S1_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
const S1_MUTATION_UNKNOWN_DEP: &str =
    include_str!("../scenarios/s1_water_pump/smoke/mutations/unknown_dependency.toml");
const S1_MUTATION_UNREACHABLE: &str =
    include_str!("../scenarios/s1_water_pump/smoke/mutations/unreachable_branch.toml");

const S1_GOLDEN_VERIFY_OK: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/verify_report.ok.json");
const S1_GOLDEN_VERIFY_UNREACHABLE: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/verify_report.unreachable_warning.json");
const S1_GOLDEN_RESOLVED: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/resolved_output.json");
const S1_GOLDEN_EMITTED_MANIFEST: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/emitted_manifest.json");

const S5_SOURCE_DEFS: &str = "scenarios/s5_building_hvac/smoke/chunks/00_definitions.toml";
const S5_SOURCE_COMPONENTS: &str = "scenarios/s5_building_hvac/smoke/chunks/10_components.toml";
const S5_PROFILE: &str = include_str!("../scenarios/s5_building_hvac/smoke/profile.toml");
const S5_CHUNK_DEFS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/00_definitions.json");
const S5_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/10_components.json");
const S5_GOLDEN_VERIFY_OK: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/golden/verify_report.ok.json");
const S5_GOLDEN_RESOLVED: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/golden/resolved_output.json");
const S5_GOLDEN_EMITTED_MANIFEST: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/golden/emitted_manifest.json");

#[derive(Debug, Deserialize)]
struct ScenarioProfile {
    selection_domains: BTreeMap<String, Vec<String>>,
    default_context: BTreeMap<String, String>,
}

#[derive(Debug)]
struct BaselineMetrics {
    ingest_us: u128,
    verify_us: u128,
    resolve_us: u128,
    emit_us: u128,
    rss_kib: u64,
}

#[derive(Debug)]
struct ClosedLoopOutputs {
    verify_report: JsonValue,
    resolved_output: JsonValue,
    emitted_manifest: JsonValue,
    metrics: BaselineMetrics,
}

fn load_profile() -> Result<ScenarioProfile> {
    toml::from_str(S1_PROFILE).context("Failed to parse S1 smoke profile")
}

fn load_s5_profile() -> Result<ScenarioProfile> {
    toml::from_str(S5_PROFILE).context("Failed to parse S5 smoke profile")
}

fn load_json(content: &str) -> Result<JsonValue> {
    serde_json::from_str(content).context("Failed to parse JSON")
}

fn build_s1_compiler(extra_chunks: &[(&str, &str)]) -> Result<Compiler> {
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto(S1_SOURCE_DEFS, S1_CHUNK_DEFS)
        .context("Failed to add S1 definitions chunk")?;
    compiler
        .add_chunk_auto(S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS)
        .context("Failed to add S1 components chunk")?;

    for (source_id, content) in extra_chunks {
        compiler
            .add_chunk_auto(*source_id, content)
            .with_context(|| format!("Failed to add mutation chunk '{}'", source_id))?;
    }

    Ok(compiler)
}

fn build_s5_compiler(extra_chunks: &[(&str, &str)]) -> Result<Compiler> {
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto(S5_SOURCE_DEFS, S5_CHUNK_DEFS)
        .context("Failed to add S5 definitions chunk")?;
    compiler
        .add_chunk_auto(S5_SOURCE_COMPONENTS, S5_CHUNK_COMPONENTS)
        .context("Failed to add S5 components chunk")?;

    for (source_id, content) in extra_chunks {
        compiler
            .add_chunk_auto(*source_id, content)
            .with_context(|| format!("Failed to add mutation chunk '{}'", source_id))?;
    }

    Ok(compiler)
}

fn run_s1_closed_loop(extra_chunks: &[(&str, &str)]) -> Result<ClosedLoopOutputs> {
    let profile = load_profile()?;

    let ingest_start = Instant::now();
    let compiler = build_s1_compiler(extra_chunks)?;
    let ingest_us = ingest_start.elapsed().as_micros();

    let verify_start = Instant::now();
    compiler.link_and_verify()?;
    let unreachable =
        unreachable_components_for_domains(compiler.get_repo(), &profile.selection_domains)?;
    let verify_report = build_verify_report(&unreachable);
    let verify_us = verify_start.elapsed().as_micros();

    let resolve_start = Instant::now();
    let context = ResolutionContext {
        tags: profile.default_context.into_iter().collect(),
    };
    let resolved = resolve(compiler.get_repo().clone(), &context)?;
    let resolved_output =
        serde_json::to_value(resolved).context("Failed to serialize resolved output")?;
    let resolve_us = resolve_start.elapsed().as_micros();

    let emit_start = Instant::now();
    let temp_dir = unique_temp_dir("cfx-baseline", "s1-smoke")?;

    compiler.emit_ir(&temp_dir.path)?;
    verify_ir_dir(&temp_dir.path)?;

    let index_path = temp_dir.path.join("index.cfir.json");
    let index_bytes = std::fs::read(&index_path)
        .with_context(|| format!("Failed to read emitted IR index '{}'", index_path.display()))?;
    let emitted_manifest = summarize_emitted_manifest(&index_bytes)?;
    let emit_us = emit_start.elapsed().as_micros();

    Ok(ClosedLoopOutputs {
        verify_report,
        resolved_output,
        emitted_manifest,
        metrics: BaselineMetrics {
            ingest_us,
            verify_us,
            resolve_us,
            emit_us,
            rss_kib: read_vm_rss_kib().unwrap_or(0),
        },
    })
}

fn run_s5_closed_loop(extra_chunks: &[(&str, &str)]) -> Result<ClosedLoopOutputs> {
    let profile = load_s5_profile()?;

    let ingest_start = Instant::now();
    let compiler = build_s5_compiler(extra_chunks)?;
    let ingest_us = ingest_start.elapsed().as_micros();

    let verify_start = Instant::now();
    compiler.link_and_verify()?;
    let unreachable =
        unreachable_components_for_domains(compiler.get_repo(), &profile.selection_domains)?;
    let verify_report = build_verify_report(&unreachable);
    let verify_us = verify_start.elapsed().as_micros();

    let resolve_start = Instant::now();
    let context = ResolutionContext {
        tags: profile.default_context.into_iter().collect(),
    };
    let resolved = resolve(compiler.get_repo().clone(), &context)?;
    let resolved_output =
        serde_json::to_value(resolved).context("Failed to serialize resolved output")?;
    let resolve_us = resolve_start.elapsed().as_micros();

    let emit_start = Instant::now();
    let temp_dir = unique_temp_dir("cfx-baseline", "s5-smoke")?;

    compiler.emit_ir(&temp_dir.path)?;
    verify_ir_dir(&temp_dir.path)?;

    let index_path = temp_dir.path.join("index.cfir.json");
    let index_bytes = std::fs::read(&index_path)
        .with_context(|| format!("Failed to read emitted IR index '{}'", index_path.display()))?;
    let emitted_manifest = summarize_emitted_manifest(&index_bytes)?;
    let emit_us = emit_start.elapsed().as_micros();

    Ok(ClosedLoopOutputs {
        verify_report,
        resolved_output,
        emitted_manifest,
        metrics: BaselineMetrics {
            ingest_us,
            verify_us,
            resolve_us,
            emit_us,
            rss_kib: read_vm_rss_kib().unwrap_or(0),
        },
    })
}

fn summarize_emitted_manifest(index_bytes: &[u8]) -> Result<JsonValue> {
    let index: crate::ir::IrIndex =
        serde_json::from_slice(index_bytes).context("Failed to parse emitted IR index")?;

    let mut source_ids: Vec<String> = index
        .chunks
        .iter()
        .map(|chunk| chunk.source_id.clone())
        .collect();
    source_ids.sort();

    let definition_ids: Vec<String> = index.definition_index.keys().cloned().collect();
    let component_ids: Vec<String> = index.component_index.keys().cloned().collect();
    let artifact_ids: Vec<String> = index.artifact_index.keys().cloned().collect();

    Ok(json!({
        "format_version": index.format_version,
        "chunk_count": index.chunks.len(),
        "source_ids": source_ids,
        "definition_ids": definition_ids,
        "component_ids": component_ids,
        "artifact_ids": artifact_ids,
    }))
}

fn build_verify_report(unreachable_components: &[String]) -> JsonValue {
    let warning_count = unreachable_components.len();
    let (reachability_status, reachability_summary, reachability_codes) =
        if unreachable_components.is_empty() {
            (
                "pass",
                "No unreachable components for scenario profile domains".to_string(),
                Vec::<String>::new(),
            )
        } else {
            (
                "warn",
                if warning_count == 1 {
                    "1 component is unreachable for scenario profile domains".to_string()
                } else {
                    format!(
                        "{} components are unreachable for scenario profile domains",
                        warning_count
                    )
                },
                vec!["W_UNREACHABLE_COMPONENT".to_string()],
            )
        };

    json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "status": "ok",
        "error_count": 0,
        "warning_count": warning_count,
        "checks": [
            {
                "check_id": "graph_integrity",
                "status": "pass",
                "summary": "All references and dependency constraints verified",
                "diagnostic_codes": []
            },
            {
                "check_id": "reachability",
                "status": reachability_status,
                "summary": reachability_summary,
                "diagnostic_codes": reachability_codes
            }
        ],
        "unreachable_components": unreachable_components,
    })
}

fn unreachable_components_for_domains(
    config: &crate::schema::Config,
    selection_domains: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>> {
    let contexts = enumerate_contexts(selection_domains);
    let mut unreachable = Vec::new();

    for (component_id, component) in &config.components {
        let Some(condition) = component.condition.as_deref() else {
            continue;
        };

        let mut reachable = false;
        for tags in &contexts {
            if conditions::eval_condition(condition, tags).with_context(|| {
                format!(
                    "Failed to evaluate component '{}' condition '{}' during reachability analysis",
                    component_id, condition
                )
            })? {
                reachable = true;
                break;
            }
        }

        if !reachable {
            unreachable.push(component_id.clone());
        }
    }

    unreachable.sort();
    Ok(unreachable)
}

fn enumerate_contexts(
    selection_domains: &BTreeMap<String, Vec<String>>,
) -> Vec<HashMap<String, String>> {
    let mut contexts = vec![HashMap::new()];

    for (tag, values) in selection_domains {
        let mut next = Vec::new();
        for context in &contexts {
            for value in values {
                let mut candidate = context.clone();
                candidate.insert(tag.clone(), value.clone());
                next.push(candidate);
            }
        }
        contexts = next;
    }

    contexts
}

fn read_vm_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let value = rest.split_whitespace().next()?;
            return value.parse::<u64>().ok();
        }
    }
    None
}

#[test]
fn baseline_s1_smoke_closed_loop_matches_goldens() -> Result<()> {
    let outputs = run_s1_closed_loop(&[])?;

    let expected_verify = load_json(S1_GOLDEN_VERIFY_OK)?;
    let expected_resolved = load_json(S1_GOLDEN_RESOLVED)?;
    let expected_manifest = load_json(S1_GOLDEN_EMITTED_MANIFEST)?;

    assert_eq!(outputs.verify_report, expected_verify);
    assert_eq!(outputs.resolved_output, expected_resolved);
    assert_eq!(outputs.emitted_manifest, expected_manifest);

    eprintln!(
        "baseline_s1_smoke_metrics ingest_us={} verify_us={} resolve_us={} emit_us={} rss_kib={}",
        outputs.metrics.ingest_us,
        outputs.metrics.verify_us,
        outputs.metrics.resolve_us,
        outputs.metrics.emit_us,
        outputs.metrics.rss_kib
    );

    Ok(())
}

#[test]
fn baseline_s5_non_robotics_smoke_closed_loop_matches_goldens() -> Result<()> {
    let outputs = run_s5_closed_loop(&[])?;

    let expected_verify = load_json(S5_GOLDEN_VERIFY_OK)?;
    let expected_resolved = load_json(S5_GOLDEN_RESOLVED)?;
    let expected_manifest = load_json(S5_GOLDEN_EMITTED_MANIFEST)?;

    assert_eq!(outputs.verify_report, expected_verify);
    assert_eq!(outputs.resolved_output, expected_resolved);
    assert_eq!(outputs.emitted_manifest, expected_manifest);

    eprintln!(
        "baseline_s5_smoke_metrics ingest_us={} verify_us={} resolve_us={} emit_us={} rss_kib={}",
        outputs.metrics.ingest_us,
        outputs.metrics.verify_us,
        outputs.metrics.resolve_us,
        outputs.metrics.emit_us,
        outputs.metrics.rss_kib
    );

    Ok(())
}

#[test]
fn baseline_mutation_unknown_dependency_fails_verify() -> Result<()> {
    let compiler = build_s1_compiler(&[(S1_SOURCE_MUTATION_UNKNOWN_DEP, S1_MUTATION_UNKNOWN_DEP)])?;

    let err = compiler
        .link_and_verify()
        .expect_err("Expected unknown dependency mutation to fail verification");

    assert!(
        format!("{err}").contains("depends_on unknown component"),
        "err: {err}"
    );
    assert!(format!("{err}").contains("missing_power_bus"), "err: {err}");

    Ok(())
}

#[test]
fn baseline_mutation_unreachable_branch_warns() -> Result<()> {
    let outputs = run_s1_closed_loop(&[(S1_SOURCE_MUTATION_UNREACHABLE, S1_MUTATION_UNREACHABLE)])?;
    let expected_verify = load_json(S1_GOLDEN_VERIFY_UNREACHABLE)?;

    assert_eq!(outputs.verify_report, expected_verify);

    Ok(())
}
