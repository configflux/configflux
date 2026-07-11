// SPDX-License-Identifier: BUSL-1.1

// configflux-9pjy.3 / ADR-0039 §7 + ADR-0005 Amendment 2: the compile-time
// progress signal. `compile_model_with_progress` accepts a `ProgressSink`;
// the summary rides on `CompileResult`. Progress is a SEPARATE stream and
// never enters the byte-stable artifact — the default `compile_model` path
// wires no sink and is byte-identical to today.
use crate::progress::{ProgressSink, ProgressSummary, ProgressTracker, Phase};
use crate::resource_budget::ResourceBudget;
use crate::{ir, Compiler};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

// Bumped 1 -> 2 (configflux-ts7z, ADR-0038 amendment Decision A.5): threading
// `override_intent` into the signed device-report payload is a wire-format change,
// so the report envelope (`SIGNED_REPORT_SCHEMA_VERSION`, which aliases this)
// advances. The version is the tamper-protected discriminator that selects the
// canonical signed shape: a report stamped below this version canonicalizes
// WITHOUT `override_intent` (so old reports still verify byte-identically), a
// report at this version canonicalizes WITH it. One monotonic schema number is
// kept across the contract surface rather than forking a report-local counter.
// Bumped 2 -> 3 (ADR-0047 §2): first-class facet declarations add `facet_index`
// to the `model_hash` preimage (via `IR_FORMAT_VERSION` 1 -> 2) and the `facets`
// namespace to the authored model, rotating `model_hash` globally this release.
// The product-contract discriminator advances in lockstep so a consumer can tell
// a facet-aware model package from a pre-facet one.
pub const PRODUCT_SCHEMA_VERSION: u32 = 3;

pub const E_UNKNOWN_COMPONENT_DEP: &str = "E_UNKNOWN_COMPONENT_DEP";
// ADR-0047: a facet key declared by more than one chunk (the pack-global
// at-most-one-declarer invariant), raised at ingest merge.
pub const E_INGEST_DUPLICATE_FACET: &str = "E_INGEST_DUPLICATE_FACET";
// ADR-0047 §3: a closed facet's declared domain is exhaustive, but a condition
// equality predicate names a value outside it.
pub const E_FACET_VALUE_UNDECLARED: &str = "E_FACET_VALUE_UNDECLARED";
pub const E_COMPONENT_DEP_CYCLE: &str = "E_COMPONENT_DEP_CYCLE";
// RETIRED by ADR-0048: diamond dependencies are permitted (the component graph
// may be any DAG). This code is reserved and never reused — it is kept as a
// visible, never-emitted marker so the frozen diagnostic registry
// (docs/interface-contracts.md §3.4) stays a stable contract. No code path
// emits it; the constant exists only to burn the identifier under its old
// meaning.
#[allow(dead_code)]
pub const E_COMPONENT_DEP_DIAMOND: &str = "E_COMPONENT_DEP_DIAMOND";
pub const E_COMPILE_INPUT_INVALID: &str = "E_COMPILE_INPUT_INVALID";
pub const E_COMPILE_EMIT_FAILED: &str = "E_COMPILE_EMIT_FAILED";
pub const E_UNSUPPORTED_SCHEMA_VERSION: &str = "E_UNSUPPORTED_SCHEMA_VERSION";
pub const E_INSPECT_UNKNOWN_COMPONENT: &str = "E_INSPECT_UNKNOWN_COMPONENT";
pub const E_INSPECT_UNKNOWN_DEFINITION: &str = "E_INSPECT_UNKNOWN_DEFINITION";
pub const E_INSPECT_UNKNOWN_ARTIFACT: &str = "E_INSPECT_UNKNOWN_ARTIFACT";
pub const E_INSPECT_UNKNOWN_PARAMETER: &str = "E_INSPECT_UNKNOWN_PARAMETER";
pub const E_INSPECT_UNKNOWN_SCOPE: &str = "E_INSPECT_UNKNOWN_SCOPE";
pub const E_INSPECT_QUERY_INVALID: &str = "E_INSPECT_QUERY_INVALID";

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Ok,
    Error,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyCheckStatus {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceManifestEntry {
    pub source_id: String,
    pub inline_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyModelRequest {
    pub schema_version: u32,
    pub source_manifest: Vec<SourceManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileModelRequest {
    pub schema_version: u32,
    pub source_manifest: Vec<SourceManifestEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    /// Target maximum number of distinct variables per BDD partition —
    /// configflux-vmlb / ADR-0012 §2. When `None`, the partitioner
    /// collapses to a single partition (`usize::MAX` default,
    /// preserving FAMA/SPLOT fixtures' single-partition layout). The
    /// CLI surfaces this via `--cluster-size`; the field is plumbed
    /// here so the compile API surface is self-describing for
    /// downstream callers (`configflux-dpst` 4 GB bench,
    /// `configflux-4fu0` fixture rotation). As of configflux-9hi2 the
    /// `compile_model` product path emits the `.ccm` artifact itself when
    /// `output_dir` is set, and this field is the partitioning input for
    /// that emission (it also remains the canonical wiring point for
    /// out-of-band emitters such as `tools/gen_synthetic`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_size: Option<usize>,
    /// Soft resource budget for the compile — configflux-9pjy.2 / ADR-0039.
    /// `None` (the default) is byte-for-byte identical to today: no memo
    /// cap is applied and no `cluster_size` is derived. When set, the
    /// budget derives an apply-memo cap (byte-neutral cache lever) and,
    /// if the projected unique table would exceed the budget, a
    /// `cluster_size` (explicit `cluster_size` above always wins, per
    /// ADR-0012 Amendment 1). `serde(default)` keeps requests serialized
    /// before this field existed deserializing unchanged, and
    /// `skip_serializing_if` keeps an unset budget out of the wire form
    /// entirely (mirrors `cluster_size`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub budget: Option<ResourceBudget>,
    /// ADR-0044 D1 (`configflux-pq2w.1`): when `true`, the emitted
    /// provenance sidecars carry a wall-clock `stamped_at`. Default `false`
    /// keeps compile output byte-stable (same inputs → same bytes, sidecar
    /// included). `serde(default)` keeps pre-field requests deserializing
    /// unchanged; `skip_serializing_if` keeps the clean default out of the
    /// canonical wire form (mirrors `cluster_size`/`budget`). The sidecar is
    /// non-hashed, so this never touches any artifact hash preimage.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stamp_time: bool,
}

/// `skip_serializing_if` predicate: keep a `false` flag out of the canonical
/// wire form (serde needs a `&bool -> bool` fn path).
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    pub schema_version: u32,
    pub diagnostics: Vec<Diagnostic>,
    pub error_count: u32,
    pub warning_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCheckResult {
    pub check_id: String,
    pub status: VerifyCheckStatus,
    pub summary: String,
    pub diagnostic_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub schema_version: u32,
    pub model_hash: String,
    pub status: OperationStatus,
    pub error_count: u32,
    pub warning_count: u32,
    pub checks: Vec<VerifyCheckResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileStats {
    pub source_count: u32,
    pub chunk_count: u32,
    pub definition_count: u32,
    pub component_count: u32,
    pub artifact_count: u32,
}

/// Next-run advisory that the effective `cluster_size` was too large for
/// the soft budget (configflux-9pjy.4 / ADR-0039 §5). Recorded only when a
/// partition's post-build RSS approached the budget. The compiler does
/// **not** re-partition mid-run — the partition layout is part of the
/// byte-stable pre-image (ADR-0012 determinism) — so this is purely a hint
/// for the operator's next invocation (lower `--cluster-size` or raise
/// `--max-rss-mb`). Metadata only; never enters the artifact bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterSizeAdvisory {
    /// The effective `cluster_size` this run used (the value to reduce next
    /// time). `u64::MAX` denotes single-partition collapse (no
    /// `--cluster-size`), i.e. partitioning should be introduced.
    pub effective_cluster_size: u64,
    /// Human-readable next-run guidance.
    pub message: String,
}

/// Soft-budget adaptation report for a compile (configflux-9pjy.4 /
/// ADR-0039 §5). Populated on [`CompileResult`] only when a
/// [`ResourceBudget`] with an `max_rss_mb` drove the adaptive path;
/// `None`/absent otherwise so the default wire form is unchanged
/// (`skip_serializing_if`). Surfaces how far the byte-neutral live
/// memo-cap shrink pushed the cache and the cross-partition advisory.
/// This is a SEPARATE summary on the API result — it never enters
/// `ccm.manifest.json`, `ccm.symbols.json`, `ccm.bdd.bin`, or
/// `partition-manifest.json` (ADR-0005 §6 / Amendment 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReport {
    /// Total live memo-cap halvings across all partitions (`0` when the
    /// budget was never approached).
    pub memo_shrink_count: u32,
    /// The smallest final memo cap any partition reached (the deepest the
    /// budget pushed the cache), or `None` when no shrink fired.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub final_memo_cap: Option<u64>,
    /// The cross-partition "cluster_size too large" advisory, present only
    /// when triggered (ADR-0039 §5). Absent by default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cluster_size_advisory: Option<ClusterSizeAdvisory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled_model_package_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub stats: CompileStats,
    pub verify_report: VerifyReport,
    /// Compile-time progress summary (configflux-9pjy.3 / ADR-0039 §7).
    /// Populated only when a [`ProgressSink`] is wired
    /// (`compile_model_with_progress`); `None` for the default
    /// `compile_model` path, which keeps the wire form byte-identical to
    /// today (the `skip_serializing_if` drops the absent field entirely).
    /// Progress is a SEPARATE stream and never enters the byte-stable
    /// artifact (ADR-0005 Amendment 2); this summary lives on the API
    /// result, not in any `ccm.*` file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub progress_summary: Option<ProgressSummary>,
    /// Soft-budget adaptation report (configflux-9pjy.4 / ADR-0039 §5).
    /// Populated only when a [`ResourceBudget`] with `max_rss_mb` drove the
    /// adaptive path; `None`/absent otherwise so the default (unbudgeted)
    /// wire form is byte-identical to today. Carries the live memo-cap
    /// shrink count / final cap and the cross-partition advisory. A
    /// SEPARATE summary on the result — never in any `ccm.*` artifact file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub budget_report: Option<BudgetReport>,
    /// Tool-identity stamp (ADR-0044 D1 / `configflux-pq2w.1`). The
    /// workspace `tool_version` (from `/VERSION`), populated by the product
    /// compile path so any consumer holding this envelope can answer "which
    /// tool version produced this". Additive + optional (serde
    /// `skip_serializing_if`), mirroring `budget_report`: it carries
    /// identity on the side-channel and never enters a hashed artifact byte,
    /// and adding it does NOT bump `schema_version`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "query_type", rename_all = "snake_case")]
pub enum InspectQuery {
    Summary,
    Component {
        component_id: String,
    },
    Definition {
        definition_id: String,
    },
    Artifact {
        artifact_id: String,
    },
    Parameter {
        component_id: String,
        param_key: String,
    },
    ScopedStats {
        scope: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectModelRequest {
    pub schema_version: u32,
    pub source_manifest: Vec<SourceManifestEntry>,
    pub query: InspectQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectionSummary {
    pub source_count: u32,
    pub definition_count: u32,
    pub component_count: u32,
    pub artifact_count: u32,
    pub definition_ids: Vec<String>,
    pub component_ids: Vec<String>,
    pub artifact_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "entity_kind", rename_all = "snake_case")]
pub enum InspectionItem {
    Component {
        component_id: String,
        component_type: Option<String>,
        condition: Option<String>,
        depends_on: Vec<String>,
        param_count: u32,
        param_keys: Vec<String>,
    },
    Definition {
        definition_id: String,
        param_type: Option<String>,
        inherits: Option<String>,
        has_value: bool,
        override_count: u32,
    },
    Artifact {
        artifact_id: String,
        name: String,
        version: Option<String>,
        hash: Option<String>,
        source: Option<String>,
        target: Option<String>,
    },
    Parameter {
        component_id: String,
        param_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        inherits: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<crate::schema::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lifecycle: Option<crate::schema::Lifecycle>,
        #[serde(skip_serializing_if = "Option::is_none")]
        safety: Option<crate::schema::SafetyLevel>,
        #[serde(skip_serializing_if = "Option::is_none")]
        access: Option<crate::schema::Role>,
        #[serde(skip_serializing_if = "Option::is_none")]
        req_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        doc: Option<String>,
        override_count: u32,
        override_conditions: Vec<String>,
        candidate_artifact_ids: Vec<String>,
    },
    ScopedStats {
        scope: String,
        scope_roots: Vec<String>,
        component_count: u32,
        parameter_count: u32,
        artifact_count: u32,
        component_ids: Vec<String>,
        artifact_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectionResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub query: InspectQuery,
    pub summary: InspectionSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<InspectionItem>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

pub fn verify_model(request: VerifyModelRequest) -> VerifyReport {
    let model_hash = hash_sources(&request.source_manifest);
    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        let diagnostic = Diagnostic {
            code: E_UNSUPPORTED_SCHEMA_VERSION.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unsupported schema_version {} (expected {})",
                request.schema_version, PRODUCT_SCHEMA_VERSION
            ),
            source_id: None,
            entity_path: None,
            hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
        };
        return verify_report_with_failures(
            model_hash,
            "graph_integrity",
            "Unsupported schema version",
            vec![diagnostic],
        );
    }

    let mut compiler = Compiler::new();
    for source in &request.source_manifest {
        if let Err(err) =
            compiler.add_chunk_auto(source.source_id.clone(), &source.inline_content)
        {
            return verify_report_with_failures(
                model_hash,
                "graph_integrity",
                "Model ingestion failed",
                vec![map_compile_input_error(
                    &err.to_string(),
                    Some(source.source_id.clone()),
                )],
            );
        }
    }

    verify_compiler(&compiler, model_hash)
}

pub fn compile_model(request: CompileModelRequest) -> CompileResult {
    // The default path wires no progress sink: byte-identical to today
    // and `progress_summary` stays absent from the wire form (ADR-0005
    // Amendment 2).
    compile_model_with_progress(request, None)
}

/// As [`compile_model`], plus an optional compile-time progress sink
/// (configflux-9pjy.3 / ADR-0039 §7). When `sink` is `Some`, the compile
/// emits a weighted-phase progress stream to it (ingest → merge → link →
/// var-order → BDD apply loop → serialize) and the returned
/// [`CompileResult::progress_summary`] is populated; when `None`, this is
/// the exact default compile — byte-for-byte identical output, no summary.
///
/// Progress is a SEPARATE stream (ADR-0005 Amendment 2): the sink and the
/// summary never enter `ccm.manifest.json`, `ccm.symbols.json`,
/// `ccm.bdd.bin`, or `partition-manifest.json`.
pub fn compile_model_with_progress(
    request: CompileModelRequest,
    sink: Option<&dyn ProgressSink>,
) -> CompileResult {
    let source_count = request.source_manifest.len() as u32;
    let mut compiler = Compiler::new();
    let mut chunk_count = 0_u32;
    let mut model_hash = hash_sources(&request.source_manifest);
    // ADR-0044 D1: stamp every result envelope with the workspace version so
    // identity rides the side-channel (never a hashed byte). Cheap `&str`.
    let tool_version_stamp = crate::provenance_sidecar::tool_version().to_string();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        let diagnostic = Diagnostic {
            code: E_UNSUPPORTED_SCHEMA_VERSION.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unsupported schema_version {} (expected {})",
                request.schema_version, PRODUCT_SCHEMA_VERSION
            ),
            source_id: None,
            entity_path: None,
            hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
        };
        let verify_report = verify_report_with_failures(
            model_hash.clone(),
            "graph_integrity",
            "Unsupported schema version",
            vec![diagnostic],
        );
        return CompileResult {
            schema_version: PRODUCT_SCHEMA_VERSION,
            status: OperationStatus::Error,
            model_hash,
            compiled_model_package_ref: None,
            diagnostics_ref: None,
            stats: compile_stats(source_count, chunk_count, &compiler),
            verify_report,
            progress_summary: None,
            budget_report: None,
            tool_version: Some(tool_version_stamp.clone()),
        };
    }

    for source in &request.source_manifest {
        match compiler.add_chunk_auto(source.source_id.clone(), &source.inline_content) {
            Ok(()) => {
                chunk_count += 1;
            }
            Err(err) => {
                let verify_report = verify_report_with_failures(
                    model_hash.clone(),
                    "graph_integrity",
                    "Model ingestion failed",
                    vec![map_compile_input_error(
                        &err.to_string(),
                        Some(source.source_id.clone()),
                    )],
                );
                return CompileResult {
                    schema_version: PRODUCT_SCHEMA_VERSION,
                    status: OperationStatus::Error,
                    model_hash,
                    compiled_model_package_ref: None,
                    diagnostics_ref: None,
                    stats: compile_stats(source_count, chunk_count, &compiler),
                    verify_report,
                    progress_summary: None,
                    budget_report: None,
                    tool_version: Some(tool_version_stamp.clone()),
                };
            }
        }
    }

    let mut verify_report = verify_compiler(&compiler, model_hash.clone());
    if verify_report.status == OperationStatus::Error {
        return CompileResult {
            schema_version: PRODUCT_SCHEMA_VERSION,
            status: OperationStatus::Error,
            model_hash,
            compiled_model_package_ref: None,
            diagnostics_ref: None,
            stats: compile_stats(source_count, chunk_count, &compiler),
            verify_report,
            progress_summary: None,
            budget_report: None,
            tool_version: Some(tool_version_stamp.clone()),
        };
    }

    let mut compiled_model_package_ref = None;
    let mut progress_summary = None;
    // configflux-9pjy.4 / ADR-0039 §5: the soft-budget adaptation report,
    // populated from the emitter's `EmitBudgetOutcome` when a budget with
    // an RSS target drove the adaptive path. Stays `None` (absent from the
    // wire form) on the unbudgeted default path.
    let mut budget_report = None;
    if let Some(output_dir) = request.output_dir {
        match compiler.emit_ir(&output_dir) {
            Ok(index) => {
                model_hash = index.config_hash;
                compiled_model_package_ref = Some(
                    Path::new(&output_dir)
                        .join(ir::CMP_DEFAULT_MANIFEST_FILENAME)
                        .to_string_lossy()
                        .into_owned(),
                );
                verify_report.model_hash = model_hash.clone();

                // configflux-9pjy.3: ingest, merge, and link are complete by
                // the time we reach the .ccm emit, so mark those phase
                // boundaries on the tracker before the emitter drives the
                // var-order → apply → serialize band. The tracker is built
                // only when a sink is wired; with no sink the emit takes the
                // exact byte-identical default path.
                let mut tracker = sink.map(ProgressTracker::new);
                if let Some(t) = tracker.as_mut() {
                    t.emit_phase_complete(Phase::Ingest, 0);
                    t.emit_phase_complete(Phase::Merge, 0);
                    t.emit_phase_complete(Phase::Link, 0);
                }

                // configflux-9hi2: additively emit the v2 multi-part `.ccm`
                // sibling so the solver has a real artifact to load (ADR-0005
                // Amendment 1, ADR-0017 §2); bound to the CMP model_hash (§9).
                // configflux-9pjy.3: thread the progress tracker through —
                // observational only, output bytes unchanged.
                // configflux-9pjy.4: capture the emitter's soft-budget
                // outcome (live memo-cap shrink + cross-partition advisory)
                // and fold it into the compile summary. The outcome is
                // metadata only — output bytes are unaffected.
                match compiler.emit_ccm_sibling_with_progress(
                    &output_dir,
                    &model_hash,
                    request.cluster_size,
                    request.budget.as_ref(),
                    tracker.as_mut(),
                ) {
                    Ok((ccm_dir, outcome)) => {
                        budget_report =
                            budget_report_from_outcome(request.budget.as_ref(), &outcome);
                        // ADR-0044 D1 (configflux-pq2w.1): write the
                        // deterministic, non-hashed provenance sidecars next
                        // to the two file-writing artifact sets — the CMP dir
                        // and its sibling CCM dir. Atomic temp+rename; no
                        // wall-clock unless `stamp_time`. This runs after both
                        // emits succeeded, so the recorded content hashes are
                        // over the final artifact bytes.
                        if let Err(err) = write_compile_provenance(
                            Path::new(&output_dir),
                            &ccm_dir,
                            request.stamp_time,
                        ) {
                            let diagnostic = Diagnostic {
                                code: E_COMPILE_EMIT_FAILED.to_string(),
                                severity: DiagnosticSeverity::Error,
                                message: err.to_string(),
                                source_id: None,
                                entity_path: None,
                                hint: Some(emit_failure_hint(&err)),
                            };
                            verify_report = verify_report_with_failures(
                                model_hash.clone(),
                                "graph_integrity",
                                "Failed to write provenance sidecar",
                                vec![diagnostic],
                            );
                            return CompileResult {
                                schema_version: PRODUCT_SCHEMA_VERSION,
                                status: OperationStatus::Error,
                                model_hash,
                                compiled_model_package_ref: None,
                                diagnostics_ref: None,
                                stats: compile_stats(source_count, chunk_count, &compiler),
                                verify_report,
                                progress_summary: None,
                                budget_report: None,
                                tool_version: Some(tool_version_stamp.clone()),
                            };
                        }
                    }
                    Err(err) => {
                        let diagnostic = Diagnostic {
                            code: E_COMPILE_EMIT_FAILED.to_string(),
                            severity: DiagnosticSeverity::Error,
                            message: err.to_string(),
                            source_id: None,
                            entity_path: None,
                            hint: Some(emit_failure_hint(&err)),
                        };
                        verify_report = verify_report_with_failures(
                            model_hash.clone(),
                            "graph_integrity",
                            "Failed to emit compiled constraint model (.ccm) artifact",
                            vec![diagnostic],
                        );
                        return CompileResult {
                            schema_version: PRODUCT_SCHEMA_VERSION,
                            status: OperationStatus::Error,
                            model_hash,
                            compiled_model_package_ref: None,
                            diagnostics_ref: None,
                            stats: compile_stats(source_count, chunk_count, &compiler),
                            verify_report,
                            progress_summary: None,
                            budget_report: None,
                            tool_version: Some(tool_version_stamp.clone()),
                        };
                    }
                }

                // configflux-9pjy.3: emit the terminal Serialize completion
                // (overall pct == 1.0) and capture the summary. `total_clauses`
                // is unknown here (it lives in the emitter), but the emitter
                // already reported the apply band; `finish` only needs it to
                // stamp the final event's clause counters, which are
                // observational. Pass 0 — the pct still terminates at exactly
                // 1.0 because the phase weights are fixed.
                progress_summary = tracker.map(|t| t.finish(0));
            }
            Err(err) => {
                let diagnostic = Diagnostic {
                    code: E_COMPILE_EMIT_FAILED.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: None,
                    entity_path: None,
                    hint: Some(emit_failure_hint(&err)),
                };
                verify_report = verify_report_with_failures(
                    model_hash.clone(),
                    "graph_integrity",
                    "Failed to emit compiled model package",
                    vec![diagnostic],
                );
                return CompileResult {
                    schema_version: PRODUCT_SCHEMA_VERSION,
                    status: OperationStatus::Error,
                    model_hash,
                    compiled_model_package_ref: None,
                    diagnostics_ref: None,
                    stats: compile_stats(source_count, chunk_count, &compiler),
                    verify_report,
                    progress_summary: None,
                    budget_report: None,
                    tool_version: Some(tool_version_stamp.clone()),
                };
            }
        }
    }

    CompileResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        compiled_model_package_ref,
        diagnostics_ref: None,
        stats: compile_stats(source_count, chunk_count, &compiler),
        verify_report,
        progress_summary,
        budget_report,
        tool_version: Some(tool_version_stamp.clone()),
    }
}

/// Build the [`BudgetReport`] from the emitter's [`EmitBudgetOutcome`]
/// (configflux-9pjy.4 / ADR-0039 §5). Returns `None` — keeping the report
/// absent from the wire form — on the unbudgeted path (no budget, or a
/// budget with no `max_rss_mb`), so the default compile is byte-identical
/// to today. When the adaptive path ran, surfaces the live memo-cap shrink
/// count / final cap and the cross-partition advisory (present only when
/// the budget was approached).
fn budget_report_from_outcome(
    budget: Option<&ResourceBudget>,
    outcome: &crate::ccm_emitter::EmitBudgetOutcome,
) -> Option<BudgetReport> {
    // The report exists only when an RSS budget actually drove the path.
    // A `max_threads`-only budget does not touch the in-crate RSS lever, so
    // it produces no report (mirrors how an unbudgeted compile is silent).
    budget?.max_rss_mb?;

    let cluster_size_advisory = if outcome.cluster_size_too_large {
        let effective = outcome.effective_cluster_size.unwrap_or(usize::MAX) as u64;
        let message = if effective == u64::MAX {
            "A partition's peak RSS approached the soft budget. Consider \
             partitioning the model with --cluster-size, or raising \
             --max-rss-mb, on the next run."
                .to_string()
        } else {
            format!(
                "A partition's peak RSS approached the soft budget at the \
                 effective cluster_size of {effective}. Consider lowering \
                 --cluster-size, or raising --max-rss-mb, on the next run."
            )
        };
        Some(ClusterSizeAdvisory {
            effective_cluster_size: effective,
            message,
        })
    } else {
        None
    };

    Some(BudgetReport {
        memo_shrink_count: outcome.memo_shrink_count,
        final_memo_cap: outcome.min_final_memo_cap.map(|c| c as u64),
        cluster_size_advisory,
    })
}

/// ADR-0044 D1 (`configflux-pq2w.1`): write the deterministic, non-hashed
/// provenance sidecars for the two file-writing artifact sets emitted by a
/// compile — the CMP directory (`<out>/provenance.json`) and its sibling CCM
/// directory (`<out>/ccm/provenance.json`). Each sidecar records the tool
/// version (from `/VERSION`), the relevant schema/format versions, and the
/// SHA-256 content hashes of the primary artifacts it accompanies. The
/// sidecars are NEVER part of any hash preimage; `stamp_time` opts into a
/// wall-clock `stamped_at` (default off keeps the sidecar byte-stable).
fn write_compile_provenance(cmp_dir: &Path, ccm_dir: &Path, stamp_time: bool) -> Result<()> {
    use crate::provenance_sidecar::{hash_file, now_rfc3339_utc, ProvenanceSidecar};

    let stamped_at = if stamp_time {
        Some(now_rfc3339_utc())
    } else {
        None
    };

    // CMP sidecar: sibling of `cmp.manifest.json`.
    let cmp_manifest_name = ir::CMP_DEFAULT_MANIFEST_FILENAME;
    let mut cmp_schema_versions = BTreeMap::new();
    cmp_schema_versions.insert("cmp_manifest".to_string(), ir::CMP_MANIFEST_SCHEMA_VERSION);
    cmp_schema_versions.insert("product".to_string(), PRODUCT_SCHEMA_VERSION);
    let mut cmp_artifacts = BTreeMap::new();
    cmp_artifacts.insert(
        cmp_manifest_name.to_string(),
        hash_file(&cmp_dir.join(cmp_manifest_name))?,
    );
    ProvenanceSidecar::new(cmp_schema_versions, cmp_artifacts, stamped_at.clone())
        .write_to_dir(cmp_dir)?;

    // CCM sidecar: sibling of the top-level `ccm.manifest.json`. Record the
    // three top-level files the multi-part emitter always writes.
    let mut ccm_schema_versions = BTreeMap::new();
    ccm_schema_versions.insert(
        "ccm".to_string(),
        crate::ccm_emitter::CCM_SCHEMA_VERSION,
    );
    ccm_schema_versions.insert(
        "ccm_partition_manifest".to_string(),
        crate::ccm_emitter::CCM_PARTITION_MANIFEST_SCHEMA_VERSION,
    );
    let mut ccm_artifacts = BTreeMap::new();
    for name in ["ccm.manifest.json", "ccm.symbols.json", "partition-manifest.json"] {
        let path = ccm_dir.join(name);
        if path.exists() {
            ccm_artifacts.insert(name.to_string(), hash_file(&path)?);
        }
    }
    ProvenanceSidecar::new(ccm_schema_versions, ccm_artifacts, stamped_at)
        .write_to_dir(ccm_dir)?;

    Ok(())
}

pub fn inspect_model(request: InspectModelRequest) -> InspectionResult {
    let InspectModelRequest {
        schema_version,
        source_manifest,
        query,
    } = request;

    let source_count = source_manifest.len() as u32;
    let model_hash = hash_sources(&source_manifest);
    if schema_version != PRODUCT_SCHEMA_VERSION {
        let diagnostic = Diagnostic {
            code: E_UNSUPPORTED_SCHEMA_VERSION.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unsupported schema_version {} (expected {})",
                schema_version, PRODUCT_SCHEMA_VERSION
            ),
            source_id: None,
            entity_path: None,
            hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
        };
        return inspect_result_with_failures(
            model_hash,
            query,
            empty_inspection_summary(source_count),
            vec![diagnostic],
        );
    }

    let mut compiler = Compiler::new();
    for source in &source_manifest {
        if let Err(err) =
            compiler.add_chunk_auto(source.source_id.clone(), &source.inline_content)
        {
            return inspect_result_with_failures(
                model_hash,
                query,
                empty_inspection_summary(source_count),
                vec![map_compile_input_error(
                    &err.to_string(),
                    Some(source.source_id.clone()),
                )],
            );
        }
    }

    if let Err(err) = compiler.link_and_verify() {
        return inspect_result_with_failures(
            model_hash,
            query,
            inspection_summary(compiler.get_repo(), source_count),
            vec![map_graph_error(&err.to_string())],
        );
    }

    let summary = inspection_summary(compiler.get_repo(), source_count);
    let item = match &query {
        InspectQuery::Summary => None,
        InspectQuery::Component { component_id } => {
            let Some(component) = compiler.get_repo().components.get(component_id) else {
                return inspect_result_with_failures(
                    model_hash,
                    query.clone(),
                    summary,
                    vec![inspect_unknown_component_diagnostic(component_id)],
                );
            };
            Some(inspection_item_component(component_id, component))
        }
        InspectQuery::Definition { definition_id } => {
            let Some(definition) = compiler.get_repo().definitions.get(definition_id) else {
                return inspect_result_with_failures(
                    model_hash,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_UNKNOWN_DEFINITION.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown definition '{}'", definition_id),
                        source_id: None,
                        entity_path: Some(format!("definition.{}", definition_id)),
                        hint: Some(
                            "Use `inspect summary` to list available definition IDs".to_string(),
                        ),
                    }],
                );
            };
            Some(InspectionItem::Definition {
                definition_id: definition_id.clone(),
                param_type: definition.r#type.clone(),
                inherits: definition.inherits.clone(),
                has_value: definition.value.is_some(),
                override_count: definition.overrides.len() as u32,
            })
        }
        InspectQuery::Artifact { artifact_id } => {
            let Some(artifact) = compiler.get_repo().artifacts.get(artifact_id) else {
                return inspect_result_with_failures(
                    model_hash,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_UNKNOWN_ARTIFACT.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown artifact '{}'", artifact_id),
                        source_id: None,
                        entity_path: Some(format!("artifact.{}", artifact_id)),
                        hint: Some(
                            "Use `inspect summary` to list available artifact IDs".to_string(),
                        ),
                    }],
                );
            };
            Some(InspectionItem::Artifact {
                artifact_id: artifact_id.clone(),
                name: artifact.name.clone(),
                version: artifact.version.clone(),
                hash: artifact.hash.clone(),
                source: artifact.source.clone(),
                target: artifact.target.clone(),
            })
        }
        InspectQuery::Parameter {
            component_id,
            param_key,
        } => {
            if !is_valid_snake_case_ident(component_id) || !is_valid_snake_case_ident(param_key) {
                return inspect_result_with_failures(
                    model_hash,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_QUERY_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Invalid parameter query payload (component_id='{}', param_key='{}')",
                            component_id, param_key
                        ),
                        source_id: None,
                        entity_path: None,
                        hint: Some(
                            "Use snake_case component_id/param_key values in inspect parameter queries"
                                .to_string(),
                        ),
                    }],
                );
            }
            let Some(component) = compiler.get_repo().components.get(component_id) else {
                return inspect_result_with_failures(
                    model_hash,
                    query.clone(),
                    summary,
                    vec![inspect_unknown_component_diagnostic(component_id)],
                );
            };
            let Some(parameter) = component.params.get(param_key) else {
                return inspect_result_with_failures(
                    model_hash,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_UNKNOWN_PARAMETER.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown parameter '{}.{}'", component_id, param_key),
                        source_id: None,
                        entity_path: Some(format!(
                            "component.{}.param.{}",
                            component_id, param_key
                        )),
                        hint: Some(
                            "Use `inspect component <component_id>` to list available param_keys"
                                .to_string(),
                        ),
                    }],
                );
            };
            match inspection_item_parameter(
                component_id,
                param_key,
                parameter,
                &compiler.get_repo().definitions,
            ) {
                Ok(item) => Some(item),
                Err(diagnostic) => {
                    return inspect_result_with_failures(
                        model_hash,
                        query.clone(),
                        summary,
                        vec![diagnostic],
                    );
                }
            }
        }
        InspectQuery::ScopedStats { scope } => {
            match inspection_item_scoped_stats(scope, compiler.get_repo()) {
                Ok(item) => Some(item),
                Err(diagnostic) => {
                    return inspect_result_with_failures(
                        model_hash,
                        query.clone(),
                        summary,
                        vec![diagnostic],
                    );
                }
            }
        }
    };

    inspect_result_ok(model_hash, query, summary, item)
}

fn compile_stats(source_count: u32, chunk_count: u32, compiler: &Compiler) -> CompileStats {
    let repo = compiler.get_repo();
    CompileStats {
        source_count,
        chunk_count,
        definition_count: repo.definitions.len() as u32,
        component_count: repo.components.len() as u32,
        artifact_count: repo.artifacts.len() as u32,
    }
}

fn inspection_summary(config: &crate::schema::Config, source_count: u32) -> InspectionSummary {
    let mut definition_ids: Vec<String> = config.definitions.keys().cloned().collect();
    definition_ids.sort();
    let mut component_ids: Vec<String> = config.components.keys().cloned().collect();
    component_ids.sort();
    let mut artifact_ids: Vec<String> = config.artifacts.keys().cloned().collect();
    artifact_ids.sort();

    InspectionSummary {
        source_count,
        definition_count: definition_ids.len() as u32,
        component_count: component_ids.len() as u32,
        artifact_count: artifact_ids.len() as u32,
        definition_ids,
        component_ids,
        artifact_ids,
    }
}

fn empty_inspection_summary(source_count: u32) -> InspectionSummary {
    InspectionSummary {
        source_count,
        definition_count: 0,
        component_count: 0,
        artifact_count: 0,
        definition_ids: Vec::new(),
        component_ids: Vec::new(),
        artifact_ids: Vec::new(),
    }
}

fn inspection_item_component(
    component_id: &str,
    component: &crate::schema::Component,
) -> InspectionItem {
    let mut depends_on = component.depends_on.clone();
    depends_on.sort();
    let mut param_keys: Vec<String> = component.params.keys().cloned().collect();
    param_keys.sort();
    InspectionItem::Component {
        component_id: component_id.to_string(),
        component_type: component.r#type.clone(),
        condition: component.condition.clone(),
        depends_on,
        param_count: param_keys.len() as u32,
        param_keys,
    }
}

fn inspect_unknown_component_diagnostic(component_id: &str) -> Diagnostic {
    Diagnostic {
        code: E_INSPECT_UNKNOWN_COMPONENT.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Unknown component '{}'", component_id),
        source_id: None,
        entity_path: Some(format!("component.{}", component_id)),
        hint: Some("Use `inspect summary` to list available component IDs".to_string()),
    }
}

fn inspection_item_parameter(
    component_id: &str,
    param_key: &str,
    parameter: &crate::schema::Parameter,
    definitions: &HashMap<String, crate::schema::Parameter>,
) -> std::result::Result<InspectionItem, Diagnostic> {
    let mut effective_parameter = parameter.clone();
    if let Err(err) = apply_definition_chain_to_parameter(
        &mut effective_parameter,
        definitions,
        &mut HashSet::new(),
    ) {
        return Err(Diagnostic {
            code: E_INSPECT_QUERY_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Failed to materialize parameter metadata for '{}.{}': {}",
                component_id, param_key, err
            ),
            source_id: None,
            entity_path: Some(format!("component.{}.param.{}", component_id, param_key)),
            hint: Some("Verify definition inheritance for this parameter".to_string()),
        });
    }

    let mut override_conditions = Vec::new();
    collect_override_conditions(parameter, &mut override_conditions);
    override_conditions.sort();
    override_conditions.dedup();

    let mut candidate_artifact_ids = BTreeSet::new();
    if effective_parameter.r#type.as_deref() == Some("artifact") {
        collect_candidate_artifact_values(parameter, &mut candidate_artifact_ids);
    }

    Ok(InspectionItem::Parameter {
        component_id: component_id.to_string(),
        param_key: param_key.to_string(),
        inherits: parameter.inherits.clone(),
        r#type: effective_parameter.r#type.clone(),
        value: parameter.value.clone(),
        unit: effective_parameter.unit.clone(),
        lifecycle: effective_parameter.lifecycle.clone(),
        safety: effective_parameter.safety.clone(),
        access: effective_parameter.access.clone(),
        req_id: effective_parameter.req_id.clone(),
        doc: effective_parameter.doc.clone(),
        override_count: count_overrides(parameter) as u32,
        override_conditions,
        candidate_artifact_ids: candidate_artifact_ids.into_iter().collect(),
    })
}

fn inspection_item_scoped_stats(
    scope: &str,
    config: &crate::schema::Config,
) -> std::result::Result<InspectionItem, Diagnostic> {
    let scope = scope.trim();
    if scope.is_empty() {
        return Err(Diagnostic {
            code: E_INSPECT_QUERY_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "Inspect scoped_stats requires a non-empty scope".to_string(),
            source_id: None,
            entity_path: Some("query.scope".to_string()),
            hint: Some(
                "Use scope values like component:<id>, platform:<id>, platform:all, or all"
                    .to_string(),
            ),
        });
    }

    let scope_roots = scope_roots_for_inspection(config, scope)?;
    let mut component_ids = BTreeSet::new();
    for root in &scope_roots {
        let closure = match dependency_closure_for_inspection(&config.components, root) {
            Ok(closure) => closure,
            Err(err) => {
                return Err(Diagnostic {
                    code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Scope '{}' failed to resolve dependency closure: {}",
                        scope, err
                    ),
                    source_id: None,
                    entity_path: Some(format!("scope.{}", scope)),
                    hint: Some(
                        "Use `inspect summary` to validate scope roots and dependencies"
                            .to_string(),
                    ),
                })
            }
        };
        component_ids.extend(closure);
    }

    let component_ids: Vec<String> = component_ids.into_iter().collect();
    let component_count = component_ids.len() as u32;

    let mut parameter_count = 0_u32;
    let mut artifact_ids = BTreeSet::new();
    for component_id in &component_ids {
        let Some(component) = config.components.get(component_id) else {
            continue;
        };
        parameter_count += component.params.len() as u32;
        for parameter in component.params.values() {
            collect_parameter_artifacts(parameter, &config.definitions, &mut artifact_ids);
        }
    }

    Ok(InspectionItem::ScopedStats {
        scope: scope.to_string(),
        scope_roots,
        component_count,
        parameter_count,
        artifact_count: artifact_ids.len() as u32,
        component_ids,
        artifact_ids: artifact_ids.into_iter().collect(),
    })
}

fn scope_roots_for_inspection(
    config: &crate::schema::Config,
    scope: &str,
) -> std::result::Result<Vec<String>, Diagnostic> {
    let selectors = match crate::resolver::parse_scope_selectors(scope) {
        Ok(selectors) => selectors,
        Err(err) => {
            return Err(Diagnostic {
                code: E_INSPECT_QUERY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!("Invalid inspect scope '{}': {}", scope, err),
                source_id: None,
                entity_path: Some("query.scope".to_string()),
                hint: Some(
                    "Use scope values like component:<id>, platform:<id>, platform:all, or all"
                        .to_string(),
                ),
            });
        }
    };

    if selectors.len() == 1 && matches!(selectors[0], crate::resolver::ScopeSelector::All) {
        let mut roots: Vec<String> = config.components.keys().cloned().collect();
        roots.sort();
        return Ok(roots);
    }

    let mut roots = BTreeSet::new();
    for selector in selectors {
        match selector {
            crate::resolver::ScopeSelector::Component(component_id) => {
                if !config.components.contains_key(&component_id) {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown scope component '{}'", component_id),
                        source_id: None,
                        entity_path: Some(format!("scope.component.{}", component_id)),
                        hint: Some(
                            "Use `inspect summary` to list available component IDs".to_string(),
                        ),
                    });
                }
                roots.insert(component_id);
            }
            crate::resolver::ScopeSelector::Platform(component_id) => {
                let Some(component) = config.components.get(&component_id) else {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown scope platform '{}'", component_id),
                        source_id: None,
                        entity_path: Some(format!("scope.platform.{}", component_id)),
                        hint: Some(
                            "Use `inspect summary` to list available component IDs".to_string(),
                        ),
                    });
                };
                if component.r#type.as_deref() != Some("platform") {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Scope selector 'platform:{}' does not match a platform component",
                            component_id
                        ),
                        source_id: None,
                        entity_path: Some(format!("scope.platform.{}", component_id)),
                        hint: Some(
                            "Use `component:<id>` for non-platform roots or choose a platform component"
                                .to_string(),
                        ),
                    });
                }
                roots.insert(component_id);
            }
            crate::resolver::ScopeSelector::PlatformAll => {
                for (component_id, component) in &config.components {
                    if component.r#type.as_deref() == Some("platform") {
                        roots.insert(component_id.clone());
                    }
                }
                if roots.is_empty() {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: "Scope selector 'platform:all' matched no components".to_string(),
                        source_id: None,
                        entity_path: Some("scope.platform:all".to_string()),
                        hint: Some(
                            "Define at least one component with type = 'platform'".to_string(),
                        ),
                    });
                }
            }
            crate::resolver::ScopeSelector::All => {}
        }
    }

    Ok(roots.into_iter().collect())
}

fn dependency_closure_for_inspection(
    components: &HashMap<String, crate::schema::Component>,
    root: &str,
) -> Result<BTreeSet<String>> {
    let mut closure = BTreeSet::new();
    let mut stack = vec![root.to_string()];

    while let Some(component_id) = stack.pop() {
        if !closure.insert(component_id.clone()) {
            continue;
        }
        let component = components
            .get(&component_id)
            .with_context(|| format!("Missing component '{}'", component_id))?;
        for dependency in &component.depends_on {
            stack.push(dependency.clone());
        }
    }

    Ok(closure)
}

fn collect_parameter_artifacts(
    parameter: &crate::schema::Parameter,
    definitions: &HashMap<String, crate::schema::Parameter>,
    out: &mut BTreeSet<String>,
) {
    let mut effective_parameter = parameter.clone();
    if apply_definition_chain_to_parameter(
        &mut effective_parameter,
        definitions,
        &mut HashSet::new(),
    )
    .is_err()
    {
        return;
    }
    if effective_parameter.r#type.as_deref() != Some("artifact") {
        return;
    }
    collect_candidate_artifact_values(parameter, out);
}

fn collect_candidate_artifact_values(
    parameter: &crate::schema::Parameter,
    out: &mut BTreeSet<String>,
) {
    if let Some(crate::schema::Value::String(value)) = &parameter.value {
        if !value.is_empty() {
            out.insert(value.clone());
        }
    }
    for override_block in &parameter.overrides {
        collect_candidate_artifact_values(override_block.payload.as_ref(), out);
    }
}

fn collect_override_conditions(parameter: &crate::schema::Parameter, out: &mut Vec<String>) {
    for override_block in &parameter.overrides {
        out.push(override_block.condition.clone());
        collect_override_conditions(override_block.payload.as_ref(), out);
    }
}

fn count_overrides(parameter: &crate::schema::Parameter) -> usize {
    parameter
        .overrides
        .iter()
        .map(|override_block| 1 + count_overrides(override_block.payload.as_ref()))
        .sum()
}

fn apply_definition_chain_to_parameter(
    parameter: &mut crate::schema::Parameter,
    definitions: &HashMap<String, crate::schema::Parameter>,
    visiting: &mut HashSet<String>,
) -> Result<()> {
    let Some(definition_id) = parameter.inherits.clone() else {
        return Ok(());
    };
    apply_definition_to_parameter(parameter, &definition_id, definitions, visiting)
}

fn apply_definition_to_parameter(
    parameter: &mut crate::schema::Parameter,
    definition_id: &str,
    definitions: &HashMap<String, crate::schema::Parameter>,
    visiting: &mut HashSet<String>,
) -> Result<()> {
    if !visiting.insert(definition_id.to_string()) {
        bail!(
            "Definition inheritance cycle detected at '{}'",
            definition_id
        );
    }

    let definition = definitions
        .get(definition_id)
        .with_context(|| format!("Unknown definition '{}'", definition_id))?;
    if let Some(parent_id) = definition.inherits.as_deref() {
        apply_definition_to_parameter(parameter, parent_id, definitions, visiting)?;
    }

    if parameter.r#type.is_none() {
        parameter.r#type = definition.r#type.clone();
    }
    if parameter.unit.is_none() {
        parameter.unit = definition.unit.clone();
    }
    if parameter.lifecycle.is_none() {
        parameter.lifecycle = definition.lifecycle.clone();
    }
    if parameter.safety.is_none() {
        parameter.safety = definition.safety.clone();
    }
    if parameter.access.is_none() {
        parameter.access = definition.access.clone();
    }
    if parameter.doc.is_none() {
        parameter.doc = definition.doc.clone();
    }
    // limits gap-fill removed in B-5 (ADR-0027 Decision 5): CUE resolves
    // `limits` into the emitted chunk, so re-filling it here was redundant.
    if parameter.req_id.is_none() {
        parameter.req_id = definition.req_id.clone();
    }

    visiting.remove(definition_id);
    Ok(())
}

fn is_valid_snake_case_ident(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
        return false;
    }

    let mut prev_underscore = false;
    for &b in bytes {
        if b.is_ascii_lowercase() || b.is_ascii_digit() {
            prev_underscore = false;
            continue;
        }
        if b == b'_' {
            if prev_underscore {
                return false;
            }
            prev_underscore = true;
            continue;
        }
        return false;
    }

    true
}

fn inspect_result_ok(
    model_hash: String,
    query: InspectQuery,
    summary: InspectionSummary,
    item: Option<InspectionItem>,
) -> InspectionResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    InspectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        query,
        summary,
        item,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn inspect_result_with_failures(
    model_hash: String,
    query: InspectQuery,
    summary: InspectionSummary,
    diagnostics: Vec<Diagnostic>,
) -> InspectionResult {
    let diagnostics = diagnostics_report(diagnostics);
    InspectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        query,
        summary,
        item: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn verify_compiler(compiler: &Compiler, model_hash: String) -> VerifyReport {
    match compiler.link_and_verify() {
        Ok(()) => verify_report_ok(model_hash),
        Err(err) => {
            let diagnostic = map_graph_error(&err.to_string());
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            verify_report_with_failures(model_hash, "graph_integrity", &summary, vec![diagnostic])
        }
    }
}

fn verify_report_ok(model_hash: String) -> VerifyReport {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    VerifyReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash,
        status: OperationStatus::Ok,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        checks: vec![VerifyCheckResult {
            check_id: "graph_integrity".to_string(),
            status: VerifyCheckStatus::Pass,
            summary: "All references and dependency constraints verified".to_string(),
            diagnostic_codes: Vec::new(),
        }],
        diagnostics_ref: None,
        diagnostics,
    }
}

fn verify_report_with_failures(
    model_hash: String,
    check_id: &str,
    summary: &str,
    diagnostics: Vec<Diagnostic>,
) -> VerifyReport {
    let diagnostics = diagnostics_report(diagnostics);
    let mut diagnostic_codes: Vec<String> = diagnostics
        .diagnostics
        .iter()
        .map(|d| d.code.clone())
        .collect();
    diagnostic_codes.sort();
    diagnostic_codes.dedup();

    VerifyReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash,
        status: OperationStatus::Error,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        checks: vec![VerifyCheckResult {
            check_id: check_id.to_string(),
            status: VerifyCheckStatus::Fail,
            summary: summary.to_string(),
            diagnostic_codes,
        }],
        diagnostics_ref: None,
        diagnostics,
    }
}

fn diagnostics_report(diagnostics: Vec<Diagnostic>) -> DiagnosticsReport {
    let error_count = diagnostics
        .iter()
        .filter(|diag| diag.severity == DiagnosticSeverity::Error)
        .count() as u32;
    let warning_count = diagnostics
        .iter()
        .filter(|diag| diag.severity == DiagnosticSeverity::Warning)
        .count() as u32;

    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics,
        error_count,
        warning_count,
    }
}

fn summary_for_diagnostic_code(code: &str) -> &'static str {
    match code {
        E_UNKNOWN_COMPONENT_DEP => "Unknown dependency target",
        E_COMPONENT_DEP_CYCLE => "Component dependency cycle detected",
        E_INGEST_DUPLICATE_FACET => "Facet declared in more than one chunk",
        E_FACET_VALUE_UNDECLARED => "Condition value outside a closed facet domain",
        E_INSPECT_UNKNOWN_COMPONENT => "Unknown component in inspect query",
        E_INSPECT_UNKNOWN_DEFINITION => "Unknown definition in inspect query",
        E_INSPECT_UNKNOWN_ARTIFACT => "Unknown artifact in inspect query",
        E_INSPECT_UNKNOWN_PARAMETER => "Unknown parameter in inspect query",
        E_INSPECT_UNKNOWN_SCOPE => "Unknown scope in inspect query",
        E_INSPECT_QUERY_INVALID => "Invalid inspect query payload",
        E_UNSUPPORTED_SCHEMA_VERSION => "Unsupported schema version",
        E_COMPILE_INPUT_INVALID => "Model ingestion failed",
        E_COMPILE_EMIT_FAILED => "Failed to emit compiled model package",
        _ => "Model verification failed",
    }
}

/// Build an accurate, non-model-blaming hint for an emit (write) failure.
///
/// By the time the compiler reaches the emit step, `link_and_verify()` has
/// already passed, so the model is structurally valid — an emit failure is by
/// construction about writing the output package, which is almost always a
/// filesystem or permissions problem on the `--out` path (most commonly a
/// container bind mount that is not writable by the in-container UID). Walk the
/// `anyhow` error chain for the underlying `std::io::Error` and classify its
/// `ErrorKind` so operators are pointed at the real cause instead of being told
/// to "fix the model". `ErrorKind` is `#[non_exhaustive]`, so the wildcard arm
/// is both required and the correct home for the neutral fallback.
fn emit_failure_hint(err: &anyhow::Error) -> String {
    let io_kind = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .map(std::io::Error::kind);
    match io_kind {
        Some(std::io::ErrorKind::PermissionDenied) => {
            "Output directory is not writable by this process. Check the directory's \
             ownership and permissions. When running via the container toolchain, ensure the \
             bind-mounted output path is writable by the in-container user — let the cfx \
             wrapper select the user mode for your runtime (Docker Desktop / rootless / \
             native Linux), and relabel the mount with ':z' on SELinux hosts."
                .to_string()
        }
        Some(std::io::ErrorKind::ReadOnlyFilesystem) => {
            "Output path is on a read-only filesystem; choose a writable --out directory."
                .to_string()
        }
        Some(std::io::ErrorKind::NotFound) => {
            "A parent directory of the output path does not exist; create the --out parent \
             directory or point --out at an existing one."
                .to_string()
        }
        _ => {
            "Could not write the compiled model package to the output path; verify the \
             --out directory exists and is writable."
                .to_string()
        }
    }
}

fn map_graph_error(message: &str) -> Diagnostic {
    if message.contains("depends_on unknown component") {
        Diagnostic {
            code: E_UNKNOWN_COMPONENT_DEP.to_string(),
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
            source_id: None,
            entity_path: None,
            hint: Some("Ensure depends_on targets reference existing component IDs".to_string()),
        }
    } else if message
        .to_ascii_lowercase()
        .contains("dependency cycle detected")
    {
        Diagnostic {
            code: E_COMPONENT_DEP_CYCLE.to_string(),
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
            source_id: None,
            entity_path: None,
            hint: Some("Break the cycle so the dependency graph is acyclic".to_string()),
        }
    } else if message.contains("closed facet") {
        Diagnostic {
            code: E_FACET_VALUE_UNDECLARED.to_string(),
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
            source_id: None,
            entity_path: None,
            hint: Some(
                "Add the value to the facet's `values`, mark the facet `open: true`, or fix the \
                 condition to use a declared value"
                    .to_string(),
            ),
        }
    } else {
        Diagnostic {
            code: E_COMPILE_INPUT_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
            source_id: None,
            entity_path: None,
            hint: None,
        }
    }
}

fn map_compile_input_error(message: &str, source_id: Option<String>) -> Diagnostic {
    // A duplicate facet declaration is a specific ingest-merge violation
    // (ADR-0047 §2) that surfaces here because it is raised while merging
    // chunks in `add_chunk_auto`. Give it its dedicated code and hint rather
    // than the generic ingest bucket.
    if message.contains("declared in more than one chunk") {
        return Diagnostic {
            code: E_INGEST_DUPLICATE_FACET.to_string(),
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
            source_id,
            entity_path: None,
            hint: Some(
                "A facet is a pack-global domain; declare each facet in exactly one chunk"
                    .to_string(),
            ),
        };
    }
    Diagnostic {
        code: E_COMPILE_INPUT_INVALID.to_string(),
        severity: DiagnosticSeverity::Error,
        message: message.to_string(),
        source_id,
        entity_path: None,
        hint: Some("Fix TOML/schema issues before compile/verify".to_string()),
    }
}

fn hash_sources(sources: &[SourceManifestEntry]) -> String {
    let mut ordered: Vec<_> = sources.iter().collect();
    ordered.sort_by(|a, b| a.source_id.cmp(&b.source_id));

    let mut hasher = Sha256::new();
    for source in ordered {
        hasher.update(source.source_id.as_bytes());
        hasher.update([0]);
        hasher.update(source.inline_content.as_bytes());
        hasher.update([255]);
    }

    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push(hex_char(b >> 4));
        out.push(hex_char(b & 0x0f));
    }
    out
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader_api::{open_model, OpenModelRequest};
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn verify_with_chunk(content: &str) -> VerifyReport {
        verify_model(VerifyModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![SourceManifestEntry {
                source_id: "test.toml".to_string(),
                inline_content: content.to_string(),
            }],
        })
    }

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

    #[test]
    fn verify_unknown_dependency_emits_expected_code() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"
            depends_on = ["missing_component"]
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Error);
        assert_eq!(
            report.checks[0].diagnostic_codes,
            vec![E_UNKNOWN_COMPONENT_DEP.to_string()]
        );
    }

    #[test]
    fn product_schema_version_is_three() {
        // ADR-0047 §2: the facet namespace + model_hash rotation advance the
        // product-contract discriminator 2 -> 3.
        assert_eq!(PRODUCT_SCHEMA_VERSION, 3);
    }

    #[test]
    fn map_graph_error_tags_closed_facet_violation() {
        let diag = map_graph_error(
            "Condition value 'mars' is not in the closed facet 'region' domain [eu, us]",
        );
        assert_eq!(diag.code, E_FACET_VALUE_UNDECLARED);
    }

    #[test]
    fn map_compile_input_error_tags_duplicate_facet() {
        let diag =
            map_compile_input_error("Facet 'region' is declared in more than one chunk", None);
        assert_eq!(diag.code, E_INGEST_DUPLICATE_FACET);
    }

    #[test]
    fn verify_closed_facet_undeclared_value_emits_expected_code() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "us"]
            default = "eu"

            [components.motor]
            type = "actuator"
            condition = "region == 'mars'"
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Error);
        assert_eq!(
            report.checks[0].diagnostic_codes,
            vec![E_FACET_VALUE_UNDECLARED.to_string()]
        );
    }

    #[test]
    fn verify_closed_facet_declared_value_passes() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "us"]
            default = "eu"

            [components.motor]
            type = "actuator"
            condition = "region == 'us'"
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Ok);
    }

    #[test]
    fn verify_cycle_emits_expected_code() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.alpha]
            type = "actuator"
            depends_on = ["beta"]

            [components.beta]
            type = "actuator"
            depends_on = ["alpha"]
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Error);
        assert_eq!(
            report.checks[0].diagnostic_codes,
            vec![E_COMPONENT_DEP_CYCLE.to_string()]
        );
    }

    #[test]
    fn verify_diamond_is_accepted() {
        // ADR-0048: a diamond (shared `shared` reached from `root` via both
        // `left` and `right`) is a permitted DAG, not an error.
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.root]
            type = "actuator"
            depends_on = ["left", "right"]

            [components.left]
            type = "actuator"
            depends_on = ["shared"]

            [components.right]
            type = "actuator"
            depends_on = ["shared"]

            [components.shared]
            type = "actuator"
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Ok);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn inspect_summary_includes_component_definition_artifact_ids() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Summary,
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        assert_eq!(result.summary.source_count, 2);
        assert!(result
            .summary
            .component_ids
            .contains(&"thermal_control".to_string()));
        assert!(result
            .summary
            .definition_ids
            .contains(&"safe_flow".to_string()));
        assert!(result
            .summary
            .artifact_ids
            .contains(&"hydra_x200_single_driver".to_string()));
    }

    #[test]
    fn inspect_component_returns_component_item() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Component {
                component_id: "thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        match result.item {
            Some(InspectionItem::Component {
                component_id,
                param_count,
                ..
            }) => {
                assert_eq!(component_id, "thermal_control");
                assert!(param_count > 0);
            }
            other => panic!("expected component item, got {other:?}"),
        }
    }

    #[test]
    fn inspect_unknown_component_emits_expected_code() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Component {
                component_id: "missing_component".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Error);
        assert_eq!(result.error_count, 1);
        assert_eq!(result.diagnostics.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics.diagnostics[0].code,
            E_INSPECT_UNKNOWN_COMPONENT.to_string()
        );
    }

    #[test]
    fn inspect_parameter_returns_metadata_and_artifact_candidates() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Parameter {
                component_id: "thermal_control".to_string(),
                param_key: "control_driver".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        match result.item {
            Some(InspectionItem::Parameter {
                component_id,
                param_key,
                r#type,
                value,
                override_count,
                candidate_artifact_ids,
                ..
            }) => {
                assert_eq!(component_id, "thermal_control");
                assert_eq!(param_key, "control_driver");
                assert_eq!(r#type.as_deref(), Some("artifact"));
                assert_eq!(
                    value,
                    Some(crate::schema::Value::String(
                        "hydra_x200_single_driver".to_string()
                    ))
                );
                assert_eq!(override_count, 2);
                assert_eq!(
                    candidate_artifact_ids,
                    vec![
                        "aeroflux_a9_driver".to_string(),
                        "hydra_x200_dual_driver".to_string(),
                        "hydra_x200_single_driver".to_string()
                    ]
                );
            }
            other => panic!("expected parameter item, got {other:?}"),
        }
    }

    #[test]
    fn inspect_scoped_stats_counts_dependency_closure() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        match result.item {
            Some(InspectionItem::ScopedStats {
                component_count,
                parameter_count,
                artifact_count,
                component_ids,
                artifact_ids,
                ..
            }) => {
                assert_eq!(component_count, 2);
                assert_eq!(parameter_count, 3);
                assert_eq!(artifact_count, 3);
                assert_eq!(
                    component_ids,
                    vec!["power_bus".to_string(), "thermal_control".to_string()]
                );
                assert_eq!(
                    artifact_ids,
                    vec![
                        "aeroflux_a9_driver".to_string(),
                        "hydra_x200_dual_driver".to_string(),
                        "hydra_x200_single_driver".to_string()
                    ]
                );
            }
            other => panic!("expected scoped_stats item, got {other:?}"),
        }
    }

    #[test]
    fn inspect_unknown_parameter_emits_expected_code() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Parameter {
                component_id: "thermal_control".to_string(),
                param_key: "missing_param".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Error);
        assert_eq!(result.error_count, 1);
        assert_eq!(
            result.diagnostics.diagnostics[0].code,
            E_INSPECT_UNKNOWN_PARAMETER.to_string()
        );
    }

    #[test]
    fn inspect_unknown_scope_and_invalid_payload_emit_expected_codes() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let unknown_scope = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:missing_component".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
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
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );
        assert_eq!(invalid_payload.status, OperationStatus::Error);
        assert_eq!(
            invalid_payload.diagnostics.diagnostics[0].code,
            E_INSPECT_QUERY_INVALID.to_string()
        );
    }

    #[test]
    fn inspect_scoped_stats_payload_is_deterministic() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let first = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );
        let second = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(first.status, OperationStatus::Ok);
        assert_eq!(second.status, OperationStatus::Ok);
        let first_json = serde_json::to_string(&first).expect("serialize first");
        let second_json = serde_json::to_string(&second).expect("serialize second");
        assert_eq!(first_json, second_json);
    }

    #[test]
    fn compile_emits_manifest_ref_for_loader_handoff() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let output_dir = std::env::temp_dir().join(format!(
            "configflux-loop2-compile-handoff-{}-{}",
            std::process::id(),
            unique
        ));

        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "scenarios/s1/00_definitions.toml".to_string(),
                    inline_content: defs.to_string(),
                },
                SourceManifestEntry {
                    source_id: "scenarios/s1/10_components.toml".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(output_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });

        assert_eq!(result.status, OperationStatus::Ok);
        let cmp_manifest_ref = result.compiled_model_package_ref.expect("cmp manifest ref");
        assert!(
            cmp_manifest_ref.ends_with(ir::CMP_DEFAULT_MANIFEST_FILENAME),
            "cmp_manifest_ref={cmp_manifest_ref}"
        );
        assert!(Path::new(&cmp_manifest_ref).exists());

        let open_result = open_model(OpenModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            cmp_manifest_ref: cmp_manifest_ref.clone(),
        });
        assert_eq!(open_result.status, OperationStatus::Ok);
        assert_eq!(open_result.model_hash, Some(result.model_hash));

        std::fs::remove_dir_all(output_dir).ok();
    }

    // Regression for the dogfood-trial failure: a write/permission failure on
    // the output path (e.g. a container bind mount not writable by the
    // in-container UID) must be reported as a FILESYSTEM cause, not as a model
    // error. The companion `verify_*_emits_expected_code` tests above prove the
    // genuine-model-error path still produces its model-blaming diagnostics, so
    // the two together pin both halves of the classification.
    #[cfg(unix)]
    #[test]
    fn compile_emit_permission_denied_reports_filesystem_cause() {
        use std::os::unix::fs::PermissionsExt;

        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "configflux-emit-perm-{}-{}",
            std::process::id(),
            unique
        ));
        let ro_parent = base.join("ro");
        std::fs::create_dir_all(&ro_parent).expect("create ro parent");
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o555))
            .expect("chmod 0555");

        // Root bypasses mode bits — if a write into the read-only dir still
        // succeeds we cannot trigger PermissionDenied; skip rather than fail.
        if std::fs::File::create(ro_parent.join(".root_probe")).is_ok() {
            std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755)).ok();
            std::fs::remove_dir_all(&base).ok();
            eprintln!("skipping: running as root, mode bits do not deny writes");
            return;
        }

        // The output lands UNDER the read-only parent, so the emit write fails.
        let output_dir = ro_parent.join("cmp");
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "scenarios/s1/00_definitions.toml".to_string(),
                    inline_content: defs.to_string(),
                },
                SourceManifestEntry {
                    source_id: "scenarios/s1/10_components.toml".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(output_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });

        // Restore perms before asserting, so cleanup always succeeds.
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755)).ok();

        assert_eq!(result.status, OperationStatus::Error);
        let diags = &result.verify_report.diagnostics.diagnostics;
        assert_eq!(diags.len(), 1, "expected one emit diagnostic, got {diags:?}");
        let diag = &diags[0];
        // The frozen diagnostic code is preserved (contract stability).
        assert_eq!(diag.code, E_COMPILE_EMIT_FAILED);
        let hint = diag.hint.clone().expect("emit diagnostic carries a hint");
        assert!(
            hint.contains("writable"),
            "hint should name the filesystem/permission cause, got: {hint}"
        );
        assert!(
            !hint.contains("Fix model"),
            "hint must not blame the model, got: {hint}"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn budget_report_absent_for_unbudgeted_compile() {
        // configflux-9pjy.4 / ADR-0039 §5: the default (unbudgeted) compile
        // must carry NO budget_report — the field is absent from the wire
        // form, keeping the default result byte-identical to today.
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let output_dir = std::env::temp_dir().join(format!(
            "configflux-9pjy4-unbudgeted-{}-{}",
            std::process::id(),
            unique
        ));
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "scenarios/s1/00_definitions.toml".to_string(),
                    inline_content: defs.to_string(),
                },
                SourceManifestEntry {
                    source_id: "scenarios/s1/10_components.toml".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(output_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });
        assert_eq!(result.status, OperationStatus::Ok);
        assert!(
            result.budget_report.is_none(),
            "unbudgeted compile must not carry a budget_report"
        );
        // And it must not serialize a `budget_report` key at all.
        let json = serde_json::to_string(&result).expect("serialize result");
        assert!(
            !json.contains("budget_report"),
            "budget_report key must be absent from the unbudgeted wire form"
        );
        std::fs::remove_dir_all(&output_dir).ok();
    }

    #[test]
    fn budget_report_from_outcome_shapes_advisory_correctly() {
        // configflux-9pjy.4 / ADR-0039 §5 (AC2): the report exists only for
        // an RSS-bearing budget, and the cluster-size advisory appears ONLY
        // when the emitter flagged `cluster_size_too_large`.
        use crate::ccm_emitter::EmitBudgetOutcome;

        let rss_budget = ResourceBudget {
            max_rss_mb: Some(512),
            max_threads: None,
        };

        // No advisory flagged ⇒ report present, advisory absent.
        let calm = EmitBudgetOutcome {
            memo_shrink_count: 2,
            min_final_memo_cap: Some(8192),
            cluster_size_too_large: false,
            effective_cluster_size: Some(usize::MAX),
        };
        let report = budget_report_from_outcome(Some(&rss_budget), &calm)
            .expect("an RSS budget yields a report");
        assert_eq!(report.memo_shrink_count, 2);
        assert_eq!(report.final_memo_cap, Some(8192));
        assert!(
            report.cluster_size_advisory.is_none(),
            "no advisory when cluster_size_too_large is false"
        );

        // Advisory flagged ⇒ advisory present and names the effective size.
        let tight = EmitBudgetOutcome {
            memo_shrink_count: 5,
            min_final_memo_cap: Some(4096),
            cluster_size_too_large: true,
            effective_cluster_size: Some(2048),
        };
        let report = budget_report_from_outcome(Some(&rss_budget), &tight)
            .expect("an RSS budget yields a report");
        let advisory = report
            .cluster_size_advisory
            .expect("advisory present when flagged");
        assert_eq!(advisory.effective_cluster_size, 2048);
        assert!(advisory.message.contains("2048"));

        // A threads-only budget (no max_rss_mb) ⇒ no report at all.
        let threads_only = ResourceBudget {
            max_rss_mb: None,
            max_threads: Some(4),
        };
        assert!(
            budget_report_from_outcome(Some(&threads_only), &tight).is_none(),
            "a threads-only budget must not produce an RSS adaptation report"
        );
        // No budget ⇒ no report.
        assert!(budget_report_from_outcome(None, &tight).is_none());
    }

    #[test]
    fn budget_derived_cluster_size_matches_explicit_cluster_size_bytes() {
        // configflux-9pjy.4 / ADR-0039 §5/§8 (AC2): a budget that derives a
        // cluster_size must produce the SAME partition-layout bytes as the
        // operator passing that effective cluster_size explicitly with no
        // budget. The budget additionally sets a byte-neutral memo cap, so
        // the only thing that could differ is the cache size — which never
        // touches output bytes. This pins "the advisory/budget metadata
        // path does not perturb the byte-stable partition layout."
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
        let manifest = vec![
            SourceManifestEntry {
                source_id: "scenarios/s1/00_definitions.toml".to_string(),
                inline_content: defs.to_string(),
            },
            SourceManifestEntry {
                source_id: "scenarios/s1/10_components.toml".to_string(),
                inline_content: comps.to_string(),
            },
        ];
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "configflux-9pjy4-clustermatch-{}-{}",
            std::process::id(),
            unique
        ));

        // A tiny budget: below the 64 MiB overhead floor, so derive_knobs
        // partitions every nontrivial model (effective cluster_size = 1).
        let budget_dir = base.join("budgeted");
        let budgeted = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: manifest.clone(),
            output_dir: Some(budget_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: Some(ResourceBudget {
                max_rss_mb: Some(1),
                max_threads: None,
            }),
            stamp_time: false,
        });
        assert_eq!(budgeted.status, OperationStatus::Ok, "budgeted compile ok");
        let report = budgeted
            .budget_report
            .as_ref()
            .expect("an RSS budget yields a report");
        let effective = report
            .cluster_size_advisory
            .as_ref()
            .map(|a| a.effective_cluster_size)
            .unwrap_or(1);

        // The explicit-equivalent: same effective cluster_size, NO budget.
        let explicit_dir = base.join("explicit");
        let explicit = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: manifest,
            output_dir: Some(explicit_dir.to_string_lossy().into_owned()),
            cluster_size: Some(effective.max(1) as usize),
            budget: None,
            stamp_time: false,
        });
        assert_eq!(explicit.status, OperationStatus::Ok, "explicit compile ok");
        assert!(
            explicit.budget_report.is_none(),
            "explicit (unbudgeted) compile carries no report"
        );

        // The two .ccm trees must be byte-identical, file for file. Compare
        // the recursively-collected (relative path -> bytes) maps.
        let budget_ccm = budget_dir.join("ccm");
        let explicit_ccm = explicit_dir.join("ccm");
        let budget_files = collect_dir_bytes(&budget_ccm);
        let explicit_files = collect_dir_bytes(&explicit_ccm);
        assert_eq!(
            budget_files, explicit_files,
            "budget-derived cluster_size must yield byte-identical partition layout \
             to the same explicit cluster_size"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// Recursively collect a directory's files into a sorted
    /// `relative-path -> bytes` map for byte-for-byte tree comparison.
    fn collect_dir_bytes(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(
            dir: &Path,
            prefix: &Path,
            out: &mut std::collections::BTreeMap<String, Vec<u8>>,
        ) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let rel = prefix.join(entry.file_name());
                if path.is_dir() {
                    walk(&path, &rel, out);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    out.insert(rel.to_string_lossy().into_owned(), bytes);
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, Path::new(""), &mut out);
        out
    }
}
