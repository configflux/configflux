// SPDX-License-Identifier: BUSL-1.1

pub const E_LOADER_UNSUPPORTED_SCHEMA_VERSION: &str = "E_LOADER_UNSUPPORTED_SCHEMA_VERSION";
pub const E_LOADER_MANIFEST_INVALID: &str = "E_LOADER_MANIFEST_INVALID";
pub const E_LOADER_MANIFEST_INCONSISTENT: &str = "E_LOADER_MANIFEST_INCONSISTENT";
pub const E_LOADER_INDEX_INVALID: &str = "E_LOADER_INDEX_INVALID";
pub const E_SELECTION_STATE_INVALID: &str = "E_SELECTION_STATE_INVALID";
pub const E_SELECTION_UNKNOWN_FACET: &str = "E_SELECTION_UNKNOWN_FACET";
pub const E_SELECTION_INVALID_OPTION: &str = "E_SELECTION_INVALID_OPTION";
pub const E_SELECTION_CONFLICT: &str = "E_SELECTION_CONFLICT";
pub const E_SELECTION_UNSATISFIABLE: &str = "E_SELECTION_UNSATISFIABLE";
// ADR-0030 frozen codes (CCM hard precondition + fallback retirement). A usable
// `.ccm` solver model is required for the selection path; absence and internal
// faults fail closed instead of degrading to the legacy compiler path. Defined
// here, alongside the other selection/resolve codes, so both the interpreter
// (`interpreter::solver_session`) and the runtime (`runtime::solver_validation`)
// reference one canonical definition.
//
// Emitted when `options`/`select` cannot reach a usable solver model (empty
// reference, unloadable artifact, or symbol-less stub). ADR-0030 D1.
pub const E_SELECTION_SOLVER_MODEL_UNAVAILABLE: &str = "E_SELECTION_SOLVER_MODEL_UNAVAILABLE";
// The selection surface's internal-fault family (ADR-0030). Emitted when the
// solver rejects a `select` the legacy engine accepts (engine divergence, D3),
// or when a solver-owned `options`/`select`/`set-parameter` query faults
// internally (D4). Both fail closed rather than degrade to legacy.
pub const E_SELECTION_ENGINE_DIVERGENCE: &str = "E_SELECTION_ENGINE_DIVERGENCE";
pub const E_RESOLVE_SCOPE_INVALID: &str = "E_RESOLVE_SCOPE_INVALID";
pub const E_RESOLVE_MODEL_INVALID: &str = "E_RESOLVE_MODEL_INVALID";
pub const E_RESOLVE_CONTEXT_UNSATISFIED: &str = "E_RESOLVE_CONTEXT_UNSATISFIED";
pub const E_RESOLVE_FAILED: &str = "E_RESOLVE_FAILED";
// Emitted when `resolve` cannot reach a usable solver model to gate
// satisfiability (absence, or a sat-gate fault). ADR-0030 D1/D4.
pub const E_RESOLVE_SOLVER_MODEL_UNAVAILABLE: &str = "E_RESOLVE_SOLVER_MODEL_UNAVAILABLE";
pub const E_EXPORT_PROFILE_INVALID: &str = "E_EXPORT_PROFILE_INVALID";
pub const E_EXPORT_RESOLVE_INVALID: &str = "E_EXPORT_RESOLVE_INVALID";
pub const E_EXPORT_ARTIFACT_INVALID: &str = "E_EXPORT_ARTIFACT_INVALID";
pub const E_EXPORT_SYMBOL_INVALID: &str = "E_EXPORT_SYMBOL_INVALID";
pub const E_EXPORT_FAILED: &str = "E_EXPORT_FAILED";
pub const E_SBOM_PROFILE_INVALID: &str = "E_SBOM_PROFILE_INVALID";
pub const E_SBOM_RESOLVE_INVALID: &str = "E_SBOM_RESOLVE_INVALID";
pub const E_SBOM_PATH_INVALID: &str = "E_SBOM_PATH_INVALID";
pub const E_SBOM_ARTIFACT_INVALID: &str = "E_SBOM_ARTIFACT_INVALID";
pub const E_SBOM_BINDING_INVALID: &str = "E_SBOM_BINDING_INVALID";
pub const E_SBOM_STATS_INVALID: &str = "E_SBOM_STATS_INVALID";
pub const E_SBOM_HASH_INVALID: &str = "E_SBOM_HASH_INVALID";
pub const E_SBOM_FAILED: &str = "E_SBOM_FAILED";

pub const EXPORT_PROFILE_CPP_EARLY_BINDING_V1: &str = "cpp_early_binding_v1";
pub const EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT: &str = "full_audit";
pub const EXPORT_SOFTWARE_BOM_PROFILE_VALUE_REDACTED: &str = "value_redacted";
pub const GENERATED_CONFIG_HPP_PATH: &str = "generated/config.hpp";
pub const GENERATED_CONFIG_BUILD_FLAGS_PATH: &str = "generated/config_build_flags.cmake";
pub const GENERATED_CONFIG_ARTIFACT_MANIFEST_PATH: &str = "generated/config_artifact_manifest.json";
pub const SOFTWARE_BOM_HASH_ALGO: &str = "sha256";
pub const SOFTWARE_BOM_CANONICALIZATION_VERSION: u32 = 1;
pub const SOFTWARE_BOM_VERSION: u32 = 1;
pub const SOFTWARE_BOM_GENERATOR_NAME: &str = "configflux-sbom";
pub const SOFTWARE_BOM_GENERATOR_VERSION: &str = "0.1.0";
pub const SOFTWARE_BOM_GENERATED_AT_RFC3339: &str = "1970-01-01T00:00:00Z";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionState {
    pub schema_version: u32,
    pub model_hash: String,
    pub scope: String,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    pub selection_state_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionDelta {
    pub facet: String,
    pub option: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunedOptionReason {
    pub option: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSelectionOptionsRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
    pub facet: String,
    #[serde(default)]
    pub include_pruned_reasons: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSelectionOptionsResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    pub facet: String,
    pub valid_options: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pruned_options: Option<Vec<PrunedOptionReason>>,
    pub selection_state_hash: String,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplySelectionRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
    pub selection_delta: SelectionDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplySelectionResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_state: Option<SelectionState>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainRejectionRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
    pub rejected_option: SelectionDelta,
}

/// One labeled `{facet}.{option}` pair inside an unsat core. Used both for the
/// `rejected` selection being explained and for each facet/option named by a
/// conflicting constraint. Mirrors the ADR-0031 D3 `{ facet, option }` shape.
/// Labeled names only — never raw BDD variable indices (ADR-0031 D3 invariant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConstraintFacet {
    pub facet: String,
    pub option: String,
}

/// Whether a conflicting constraint is a prior *selection* the caller already
/// made (a choice in `selection_state`) or a *model rule* baked into the `.ccm`
/// (a `requires`/`excludes`-style constraint). ADR-0031 D3 `kind` field.
///
/// Serializes snake_case (`"selection"` / `"model_rule"`) to match the stable
/// JSON schema. The compiler only serializes this enum; the solver decides
/// which variant each MUS clause is (ADR-0031 D3, solver-decided content).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    Selection,
    ModelRule,
}

/// One entry in the labeled minimal unsatisfiable subset (MUS): a single
/// constraint that, together with the `rejected` selection, contributes to the
/// unsatisfiability. ADR-0031 D3 `conflicting_constraints[]` element.
///
/// `summary` is advisory human-gloss text, not a parsed field (ADR-0031 D3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictingConstraint {
    pub kind: ConstraintKind,
    pub facets: Vec<ConstraintFacet>,
    pub summary: String,
}

/// The labeled unsat core attached to a genuine constraint-conflict rejection
/// (`E_SELECTION_CONFLICT` / `E_SELECTION_UNSATISFIABLE`). ADR-0031 D3.
///
/// The solver produces the decision content (the labeled MUS); the compiler
/// serializes these envelope bytes. The interpreter/runtime wrappers convert
/// the solver-owned core type into this compiler-side type — the compiler never
/// imports the solver (ADR-0003 §2). `conflicting_constraints` MUST contain
/// only labeled `{facet}.{option}` names, never raw BDD variable indices
/// (ADR-0031 D3 schema invariant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsatCore {
    pub rejected: ConstraintFacet,
    pub conflicting_constraints: Vec<ConflictingConstraint>,
    pub minimal: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RejectionReason {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub blocking_choices: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// The labeled unsat core (ADR-0031 D3). Present only on solver-decided
    /// constraint-conflict rejections (`E_SELECTION_CONFLICT` /
    /// `E_SELECTION_UNSATISFIABLE`); `None`/omitted for the division-of-labor
    /// rejections the compiler owns without a solver core (ADR-0030 D5) and for
    /// fail-closed command errors (ADR-0031 D4). Additive `Option` field so the
    /// existing constructors and all prior callers degrade cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsat_core: Option<UnsatCore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainRejectionResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    pub facet: String,
    pub option: String,
    pub rejection: RejectionReason,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveFromSelectionRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    pub selection_state_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub choices: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_component_dependencies: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_artifacts: BTreeMap<String, crate::schema::Artifact>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportResolvedRequest {
    pub schema_version: u32,
    pub resolve_result: ResolveResult,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedArtifact {
    pub path: String,
    pub contents: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedArtifacts {
    pub profile: String,
    pub generator_hash: String,
    pub files: Vec<GeneratedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportResolvedResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_artifacts: Option<GeneratedArtifacts>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
    /// Tool-identity stamp (ADR-0044 D1 / `configflux-pq2w.1`). This is an
    /// ENVELOPE-ONLY builder (no output dir → no sidecar file), so identity
    /// rides this additive optional field instead. Never enters `resolve_hash`
    /// (computed over `SelectionState`/resolved output, not this envelope), so
    /// adding it changes no hash and implies no `schema_version` bump.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_version: Option<String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoftwareBomBindingPhase {
    Early,
    Late,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomGeneratorMetadata {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomComponentEntry {
    pub component_id: String,
    pub r#type: String,
    pub dependency_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoftwareBomParameterEntry {
    pub path: String,
    pub component_id: String,
    pub param_key: String,
    pub r#type: String,
    pub value: crate::schema::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub safety: crate::schema::SafetyLevel,
    pub lifecycle: crate::schema::Lifecycle,
    pub binding_phase: SoftwareBomBindingPhase,
    pub access: crate::schema::Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<crate::schema::Limits>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomArtifactEntry {
    pub artifact_id: String,
    pub bound_paths: Vec<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomStats {
    pub component_count: u32,
    pub parameter_count: u32,
    pub artifact_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoftwareBomV1 {
    pub schema_version: u32,
    pub bom_version: u32,
    pub bom_hash: String,
    pub hash_algo: String,
    pub canonicalization_version: u32,
    pub model_hash: String,
    pub resolve_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_state_hash: Option<String>,
    pub scope_root: String,
    pub generated_at: String,
    pub generator: SoftwareBomGeneratorMetadata,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    pub components: Vec<SoftwareBomComponentEntry>,
    pub parameters: Vec<SoftwareBomParameterEntry>,
    pub artifacts: Vec<SoftwareBomArtifactEntry>,
    pub stats: SoftwareBomStats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSoftwareBomRequest {
    pub schema_version: u32,
    pub resolve_result: ResolveResult,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSoftwareBomResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bom_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software_bom: Option<SoftwareBomV1>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
    /// Tool-identity stamp (ADR-0044 D1 / `configflux-pq2w.1`). Envelope-only
    /// builder (no output dir → no sidecar file); identity rides this additive
    /// optional field. The hashed SBOM `generator`/`generated_at` stay frozen
    /// (D1.5) — this field lives on the RESULT envelope, NOT inside
    /// `SoftwareBomV1`, so `bom_hash` is unchanged and no `schema_version`
    /// bump is implied.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SelectionStateCanonical<'a> {
    schema_version: u32,
    model_hash: &'a str,
    scope: &'a str,
    context_tags: &'a BTreeMap<String, String>,
    choices: &'a BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
struct ResolveHashCanonical<'a> {
    schema_version: u32,
    model_hash: &'a str,
    scope: &'a str,
    selection_state: SelectionStateCanonical<'a>,
    resolved_output: &'a serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct GeneratorHashCanonical<'a> {
    schema_version: u32,
    profile: &'a str,
    model_hash: &'a str,
    scope: &'a str,
    resolve_hash: &'a str,
    files: &'a [GeneratedArtifactHashCanonical<'a>],
}

#[derive(Debug, Clone, Serialize)]
struct GeneratedArtifactHashCanonical<'a> {
    path: &'a str,
    content_hash: &'a str,
}

#[derive(Debug, Clone)]
struct ConstructionParamBinding {
    entity_path: String,
    value: crate::schema::Value,
    header_symbol: String,
    cmake_var: String,
    compile_definition: String,
}

#[derive(Debug, Clone, Serialize)]
struct ArtifactManifest {
    schema_version: u32,
    profile: String,
    model_hash: String,
    resolve_hash: String,
    artifacts: Vec<ArtifactManifestEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct ArtifactManifestEntry {
    artifact_id: String,
    bound_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct EarlyBindingExtraction {
    construction_bindings: Vec<ConstructionParamBinding>,
    artifact_manifest_entries: Vec<ArtifactManifestEntry>,
}

#[derive(Debug, Clone)]
struct ExportGenerationError {
    code: &'static str,
    message: String,
    entity_path: Option<String>,
    hint: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SoftwareBomGenerationError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) entity_path: Option<String>,
    pub(crate) hint: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct SelectionConstraintModel {
    facet_domains: BTreeMap<String, BTreeSet<String>>,
    conditions: Vec<ConditionExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenModelRequest {
    pub schema_version: u32,
    pub cmp_manifest_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelHandle {
    pub model_hash: String,
    pub cmp_manifest_ref: String,
    pub index_ref: String,
    pub chunk_set_ref: String,
    /// Path to the sibling `.ccm` artifact directory emitted alongside the
    /// CMP package (configflux-9hi2). Downstream callers (`solver::Session::
    /// load_ccm`, ADR-0017 §2) locate and load the v2 multi-part `.ccm` here.
    /// Resolved as `<cmp_manifest_dir>/ccm`. Defaults to empty for handles
    /// produced before this field existed; consumers treat empty as
    /// "no sibling `.ccm` advertised".
    #[serde(default)]
    pub ccm_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenModelResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_handle: Option<ModelHandle>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeSelectionStateRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeSelectionStateResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_state: Option<SelectionState>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}
