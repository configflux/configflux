// SPDX-License-Identifier: BUSL-1.1

pub const E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION: &str = "E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION";
pub const E_RUNTIME_OPEN_INVALID: &str = "E_RUNTIME_OPEN_INVALID";
// ADR-0030 D2 frozen code: `runtime-open` fails closed when the snapshot's
// `ccm_ref` does not resolve to a usable solver model (empty reference,
// unloadable artifact, or symbol-less stub). The check is enforced by the
// runtime CLI wrapper (`runtime::cli_adapter`), which can import `solver`;
// the code is defined here so the whole `E_RUNTIME_OPEN_*` family stays in
// one place for the interface contract.
pub const E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE: &str =
    "E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE";
pub const E_RUNTIME_HASH_MISMATCH: &str = "E_RUNTIME_HASH_MISMATCH";
pub const E_RUNTIME_UNKNOWN_SCOPE: &str = "E_RUNTIME_UNKNOWN_SCOPE";
pub const E_RUNTIME_UNKNOWN_PATH: &str = "E_RUNTIME_UNKNOWN_PATH";
pub const E_RUNTIME_TYPE_MISMATCH: &str = "E_RUNTIME_TYPE_MISMATCH";
pub const E_RUNTIME_LIMIT_VIOLATION: &str = "E_RUNTIME_LIMIT_VIOLATION";
pub const E_RUNTIME_LIFECYCLE_IMMUTABLE: &str = "E_RUNTIME_LIFECYCLE_IMMUTABLE";
pub const E_RUNTIME_ARTIFACT_UNKNOWN: &str = "E_RUNTIME_ARTIFACT_UNKNOWN";
pub const E_RUNTIME_DIRTY_INVALID: &str = "E_RUNTIME_DIRTY_INVALID";
pub const E_RUNTIME_EVENT_INVALID: &str = "E_RUNTIME_EVENT_INVALID";
pub const E_RUNTIME_COMMIT_INVALID: &str = "E_RUNTIME_COMMIT_INVALID";
pub const E_RUNTIME_COMMIT_BASE_MISMATCH: &str = "E_RUNTIME_COMMIT_BASE_MISMATCH";
pub const E_RUNTIME_COMMIT_TARGET_HASH_MISMATCH: &str = "E_RUNTIME_COMMIT_TARGET_HASH_MISMATCH";
pub const E_RUNTIME_SYNC_INVALID: &str = "E_RUNTIME_SYNC_INVALID";
pub const E_RUNTIME_SYNC_BASE_MISMATCH: &str = "E_RUNTIME_SYNC_BASE_MISMATCH";
pub const E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH: &str = "E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH";
pub const E_RUNTIME_SYNC_TARGET_HASH_MISMATCH: &str = "E_RUNTIME_SYNC_TARGET_HASH_MISMATCH";
pub const E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED: &str = "E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED";
pub const E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN: &str = "E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN";
pub const E_RUNTIME_AUDIT_INVALID: &str = "E_RUNTIME_AUDIT_INVALID";
const DEFAULT_DIRTY_ACTOR: &str = "runtime_api.set_parameter";
const DEFAULT_SYSTEM_ACTOR: &str = "system/unknown";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyEntryMetadata {
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub dirty_since_unix_ms: u64,
    /// The **session-lease** deadline (ADR-0037 Decision 3): the short
    /// abandoned-experiment-cleanup bound. Historically the only deadline an
    /// override carried; it remains the lease bound. `None` when auto-reset is
    /// disabled for the path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_deadline_unix_ms: Option<u64>,
    /// The **hard-cap** deadline (ADR-0037 Decision 3): the long bound
    /// (`<= HARD_CAP_MAX_MS`, ≤ 1 year). Persisted alongside the session-lease
    /// deadline so the active driver (configflux-ccql.4) can classify which
    /// terminal event is due — a lease expiry vs. a hard-cap escalation — purely
    /// from state, without a caller-supplied selector (configflux-h6wc).
    ///
    /// `#[serde(default)]` so snapshots serialized before this field existed
    /// round-trip and load with `None` (no hard cap scheduled): pre-existing
    /// single-deadline overrides keep their exact prior meaning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_cap_deadline_unix_ms: Option<u64>,
    pub generation: u64,
    /// Governance intent of the override (ADR-0037). Defaults to
    /// `experimental` so snapshots serialized before this field existed
    /// deserialize sensibly (abandoned-experiment cleanup is the safe default).
    #[serde(default = "default_override_intent")]
    pub intent: OverrideIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoResetPathPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoResetPolicy {
    #[serde(default = "default_auto_reset_enabled")]
    pub enabled: bool,
    #[serde(default = "default_auto_reset_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default)]
    pub per_path_overrides: BTreeMap<String, AutoResetPathPolicy>,
    #[serde(default)]
    pub policy_revision: u64,
}

impl Default for AutoResetPolicy {
    fn default() -> Self {
        Self {
            enabled: default_auto_reset_enabled(),
            default_timeout_ms: default_auto_reset_timeout_ms(),
            per_path_overrides: BTreeMap::new(),
            policy_revision: 0,
        }
    }
}

/// Which terminal-event bound a scheduled entry represents (ADR-0037 Decision 3,
/// configflux-h6wc). A `DirtyEntryMetadata` now carries two deadlines — a
/// session-lease and a hard-cap — and a schedule entry is built per bound, so
/// the active driver can self-classify the due terminal event from the entry's
/// own `kind` rather than from a caller-supplied selector.
///
/// `LeaseExpiry` is the [`Default`] so a schedule entry serialized before this
/// discriminator existed (the single-deadline era) deserializes as the
/// session-lease bound — exactly its prior meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoResetDeadlineKind {
    /// The short session-lease bound (abandoned-experiment cleanup). Maps to
    /// [`TerminalEvent::SessionLeaseExpiry`].
    LeaseExpiry,
    /// The long hard-cap bound (`<= HARD_CAP_MAX_MS`). Maps to
    /// [`TerminalEvent::HardCap`].
    HardCap,
}

impl Default for AutoResetDeadlineKind {
    fn default() -> Self {
        AutoResetDeadlineKind::LeaseExpiry
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoResetScheduleEntry {
    pub scope_root: String,
    pub path: String,
    pub generation: u64,
    pub deadline_unix_ms: u64,
    /// Which bound this entry represents (session-lease vs. hard-cap), so the
    /// active driver classifies the due terminal event from persisted state
    /// (configflux-h6wc). `#[serde(default)]` keeps pre-discriminator schedule
    /// entries loading as [`AutoResetDeadlineKind::LeaseExpiry`].
    #[serde(default)]
    pub kind: AutoResetDeadlineKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoResetSchedulerState {
    #[serde(default)]
    pub pending: Vec<AutoResetScheduleEntry>,
}

impl Default for AutoResetSchedulerState {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventKind {
    RuntimeOpened,
    ParameterChanged,
    DirtyStateChanged,
    ResetApplied,
    RollbackApplied,
    CommitApplied,
    SyncStateChanged,
    SyncConflictDetected,
    SyncConflictResolved,
    SyncApplyCompleted,
    /// The active cap/lease driver reached a terminal event for a compensating
    /// override and escalated it instead of silently reverting (ADR-0037
    /// Decision 2, configflux-ccql.4). Pairs with the durable
    /// `RuntimeAuditEventKind::Escalation`. Additive variant.
    OverrideEscalated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "payload_kind", rename_all = "snake_case")]
pub enum RuntimeEventPayload {
    RuntimeOpened {
        scope_root: String,
    },
    ParameterChanged {
        scope_root: String,
        path: String,
        generation: u64,
    },
    DirtyStateChanged {
        scope_root: String,
        path: String,
        dirty: bool,
        generation: u64,
    },
    ResetApplied {
        scope_root: String,
        path: String,
        cause: String,
        generation: u64,
    },
    RollbackApplied {
        rolled_back_paths: Vec<String>,
    },
    CommitApplied {
        commit_id: String,
        changed_paths: Vec<String>,
    },
    SyncStateChanged {
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    SyncConflictDetected {
        conflict_paths: Vec<String>,
    },
    SyncConflictResolved {
        conflict_paths: Vec<String>,
        applied_paths: Vec<String>,
        source: SyncApplySource,
    },
    SyncApplyCompleted {
        source: SyncApplySource,
        base_configuration_id: String,
        target_configuration_id: String,
        applied_paths: Vec<String>,
        conflict_paths: Vec<String>,
    },
    /// A compensating override was escalated by the active cap/lease driver at a
    /// terminal event rather than silently reverted (ADR-0037 Decision 2,
    /// configflux-ccql.4). Carries which override and which terminal event
    /// triggered the escalation, plus the intent (always `compensating`) so the
    /// brought-home event is self-describing. Additive variant.
    OverrideEscalated {
        scope_root: String,
        path: String,
        intent: OverrideIntent,
        terminal_event: TerminalEvent,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub event_id: String,
    pub sequence: u64,
    pub event_kind: RuntimeEventKind,
    pub scope: String,
    pub timestamp_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_value_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_value_hash: Option<String>,
    pub payload: RuntimeEventPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEventBusState {
    #[serde(default = "default_event_next_sequence")]
    pub next_sequence: u64,
    #[serde(default = "default_event_buffer_capacity")]
    pub buffer_capacity: usize,
    #[serde(default)]
    pub events: VecDeque<RuntimeEvent>,
    #[serde(default)]
    pub dropped_events: u64,
}

impl Default for RuntimeEventBusState {
    fn default() -> Self {
        Self {
            next_sequence: default_event_next_sequence(),
            buffer_capacity: default_event_buffer_capacity(),
            events: VecDeque::new(),
            dropped_events: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAuditEventKind {
    Write,
    Reset,
    Commit,
    SyncApply,
    DirectPush,
    /// A compensating override reached a terminal event (hard cap or
    /// session-lease expiry) but was NOT silently reverted: the active
    /// cap/lease driver surfaced it as an observable, state-gated decision
    /// instead (ADR-0037 Decision 2, configflux-ccql.4). A compensating
    /// override is a proxy for an unresolved physical condition, so its
    /// termination is always brought home as an audit event rather than a
    /// silent reset. Additive variant: old snapshots never contain it.
    Escalation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAuditEvent {
    pub event_id: String,
    pub sequence: u64,
    pub event_kind: RuntimeAuditEventKind,
    pub scope: String,
    pub timestamp_unix_ms: u64,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub committed_configuration_id: String,
    pub working_configuration_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_configuration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_configuration_id: Option<String>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSyncState {
    Idle,
    Checking,
    Pulling,
    Applying,
    Error,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSyncStatus {
    #[serde(default = "default_runtime_sync_state")]
    pub sync_state: RuntimeSyncState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_successful_sync_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_update_summary: Option<String>,
    #[serde(default)]
    pub sync_diagnostics: Vec<Diagnostic>,
}

impl Default for RuntimeSyncStatus {
    fn default() -> Self {
        Self {
            sync_state: default_runtime_sync_state(),
            last_successful_sync_unix_ms: None,
            pending_update_summary: None,
            sync_diagnostics: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncApplySource {
    Backend,
    DirectPush,
}

impl Default for SyncApplySource {
    fn default() -> Self {
        Self::Backend
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeOpenRequest {
    pub schema_version: u32,
    pub model_hash: String,
    /// Optional path to the `.ccm` artifact directory the model was compiled
    /// to (configflux-9hi2). When supplied it is recorded on the resulting
    /// `RuntimeSnapshot` so a future solver-backed `set_parameter` (g3f.3) can
    /// load it. Optional/defaulted to keep every existing open payload valid.
    #[serde(default)]
    pub ccm_ref: String,
    pub resolve_hash: String,
    pub scope: String,
    pub resolved_output: serde_json::Value,
    #[serde(default)]
    pub resolved_component_dependencies: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    pub resolved_artifacts: BTreeMap<String, crate::schema::Artifact>,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    // ADR-0047 §5 lockstep: carry the resolve's auto-bound-default provenance so
    // `runtime_open` reproduces the SAME `resolve_hash` the loader emitted. A
    // caller bridging a `ResolveResult` into a runtime open MUST copy
    // `resolve_result.defaulted_choices` here. `#[serde(default)]` so every
    // pre-ADR-0047 open payload (empty map) stays valid and byte-identical.
    #[serde(default)]
    pub defaulted_choices: BTreeMap<String, String>,
    #[serde(default)]
    pub committed_overlay: BTreeMap<String, BTreeMap<String, crate::schema::Value>>,
    #[serde(default)]
    pub dirty_overlay: BTreeMap<String, BTreeMap<String, crate::schema::Value>>,
    #[serde(default)]
    pub dirty_generations: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    pub dirty_metadata: BTreeMap<String, BTreeMap<String, DirtyEntryMetadata>>,
    #[serde(default)]
    pub auto_reset_policy: AutoResetPolicy,
    #[serde(default)]
    pub auto_reset_scheduler: AutoResetSchedulerState,
    #[serde(default)]
    pub event_bus: RuntimeEventBusState,
    #[serde(default)]
    pub sync_status: RuntimeSyncStatus,
    #[serde(default)]
    pub audit_events: Vec<RuntimeAuditEvent>,
    #[serde(default = "default_audit_next_sequence")]
    pub audit_next_sequence: u64,
    #[serde(default)]
    pub audit_uploaded_sequence: u64,
    #[serde(default = "default_persistence_format_version")]
    pub persistence_format_version: u32,
    #[serde(default)]
    pub persistence_journal_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub schema_version: u32,
    pub model_hash: String,
    /// Path to the `.ccm` artifact directory this snapshot's model was
    /// compiled to (configflux-9hi2). Round-tripped through the runtime open
    /// envelope so a future solver-backed `set_parameter` (g3f.3) can
    /// `Session::<CuddBackend>::load_ccm(ccm_ref)` and re-derive the session
    /// per ADR-0017 §3. Empty when the opener did not supply one; this field
    /// only *surfaces* the path — no runtime constraint wiring consumes it yet
    /// (that is g3f.3's scope).
    #[serde(default)]
    pub ccm_ref: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    pub resolved_output: BTreeMap<String, crate::resolved_models::ResolvedConfig>,
    #[serde(default)]
    pub resolved_component_dependencies: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    pub resolved_artifacts: BTreeMap<String, crate::schema::Artifact>,
    #[serde(default)]
    pub committed_overlay: BTreeMap<String, BTreeMap<String, crate::schema::Value>>,
    #[serde(default)]
    pub dirty_overlay: BTreeMap<String, BTreeMap<String, crate::schema::Value>>,
    #[serde(default)]
    pub dirty_generations: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    pub dirty_metadata: BTreeMap<String, BTreeMap<String, DirtyEntryMetadata>>,
    #[serde(default)]
    pub auto_reset_policy: AutoResetPolicy,
    #[serde(default)]
    pub auto_reset_scheduler: AutoResetSchedulerState,
    #[serde(default)]
    pub event_bus: RuntimeEventBusState,
    #[serde(default)]
    pub sync_status: RuntimeSyncStatus,
    #[serde(default)]
    pub audit_events: Vec<RuntimeAuditEvent>,
    #[serde(default = "default_audit_next_sequence")]
    pub audit_next_sequence: u64,
    #[serde(default)]
    pub audit_uploaded_sequence: u64,
    #[serde(default = "default_persistence_format_version")]
    pub persistence_format_version: u32,
    #[serde(default)]
    pub persistence_journal_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeOpenResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeMetadata {
    pub component_count: u32,
    pub parameter_count: u32,
    pub artifact_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetScopeMetadataRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub scope_root: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetScopeMetadataResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    pub scope_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ScopeMetadata>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeArtifactBinding {
    pub artifact_id: String,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeParameterPayload {
    pub path: String,
    pub component_id: String,
    pub param_key: String,
    pub r#type: String,
    pub value: crate::schema::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub safety: crate::schema::SafetyLevel,
    pub lifecycle: crate::schema::Lifecycle,
    pub access: crate::schema::Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<crate::schema::Limits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<RuntimeArtifactBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetParameterRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetParameterResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<RuntimeParameterPayload>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListParametersRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub scope_root: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListParametersResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    pub scope_root: String,
    pub parameter_paths: Vec<String>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetParameterRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub path: String,
    pub value: crate::schema::Value,
    /// Governance intent the operator declares for this override (ADR-0037
    /// Decision 1, design doc sec 6/sec 7, configflux-irid). Threaded into the
    /// persisted `DirtyEntryMetadata.intent` so a technician can declare a
    /// `compensating` override at write time and have the never-silently-reverted
    /// invariant govern it end to end. Optional with a serde default so existing
    /// payloads (and the agent's older clients) round-trip backward-compatibly,
    /// defaulting to `experimental` — preserving today's exact behavior.
    #[serde(default = "default_override_intent")]
    pub intent: OverrideIntent,
    /// Operator identity for this write (configflux-irid). When omitted the write
    /// falls back to the default dirty actor, exactly as before. Optional with a
    /// serde default for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// Operator-supplied reason for this write (configflux-irid). Persisted onto
    /// the dirty metadata so a declared compensating override carries its
    /// justification. Optional with a serde default for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetParameterResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<RuntimeParameterPayload>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

/// Request envelope for the runtime `explain-rejection` command (ADR-0031 D1,
/// configflux-3b5y). Asks why setting parameter `path` to `value` would be
/// rejected against the current `runtime_snapshot`. The request vocabulary is
/// the runtime's native **{parameter, value}** (a `component.<id>.param.<key>`
/// write path and its candidate scalar), exactly the shape `SetParameterRequest`
/// uses; the runtime wrapper maps it to the solver's **{facet, option}** before
/// calling `solver::Session::explain_rejection` (see
/// `runtime::explain_rejection` for the documented mapping).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeExplainRejectionRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    /// The `component.<component_id>.param.<param_key>` write path. `param_key`
    /// is the candidate **facet**.
    pub path: String,
    /// The candidate parameter value. A `Value::String` is the candidate
    /// **option** the solver explains; a non-string value names no modeled
    /// option (a free-form scalar — a division-of-labor case, no core).
    pub value: crate::schema::Value,
}

/// Result envelope for the runtime `explain-rejection` command (ADR-0031 D2/D3,
/// configflux-3b5y). Mirrors `loader_api::ExplainRejectionResult` but carries
/// the runtime's **{parameter, value}** identity (`path`/`value`) rather than
/// `{facet, option}`, so the response is self-describing in the caller's own
/// vocabulary. The `rejection` payload — including the labeled `unsat_core` — is
/// the shared compiler-side `RejectionReason`/`UnsatCore` type (the single
/// ADR-0031 D3 schema), populated by the runtime wrapper from the solver-owned
/// `LabeledCore`.
///
/// `status` follows the ADR-0031 D2/D4 convention: a genuine rejection
/// explanation is a **success** (`Ok`, exit 0) carrying the core; a
/// division-of-labor unknown facet/option or a fail-closed solver fault is a
/// command **error** (`Error`, exit 2) with no core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeExplainRejectionResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    /// The `component.<id>.param.<key>` path that was explained (echoed for
    /// self-containment — the runtime {parameter} identity).
    pub path: String,
    /// The candidate value that was explained (the runtime {value} identity).
    pub value: crate::schema::Value,
    pub rejection: crate::loader_api::RejectionReason,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicParameterWrite {
    pub path: String,
    pub value: crate::schema::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetParametersAtomicallyRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    #[serde(default)]
    pub writes: Vec<AtomicParameterWrite>,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_working_configuration_id: Option<String>,
    /// Governance intent the operator declares for every write in this atomic
    /// batch (ADR-0037 Decision 1, configflux-irid). Threaded into each written
    /// path's persisted `DirtyEntryMetadata.intent`. `actor`/`reason` are already
    /// carried by this request; this adds the intent axis. Optional with a serde
    /// default so existing payloads round-trip backward-compatibly, defaulting to
    /// `experimental` — preserving today's exact behavior.
    #[serde(default = "default_override_intent")]
    pub intent: OverrideIntent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetParametersAtomicallyResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    pub applied_count: u32,
    #[serde(default)]
    pub rejected_paths: Vec<String>,
    pub dirty_generation_max: u64,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListDirtyParametersRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub scope_root: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListDirtyParametersResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    pub scope_root: String,
    #[serde(default)]
    pub dirty_paths: Vec<String>,
    pub dirty_count: u32,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetDirtyMetadataRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetDirtyMetadataResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    pub path: String,
    pub dirty: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<DirtyEntryMetadata>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetAutoResetPolicyRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub auto_reset_policy: AutoResetPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetAutoResetPolicyResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_reset_policy: Option<AutoResetPolicy>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetAutoResetPolicyRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetAutoResetPolicyResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_reset_policy: Option<AutoResetPolicy>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckForUpdatesRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    #[serde(default = "default_backend_connected")]
    pub backend_connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_update_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckForUpdatesResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_status: Option<RuntimeSyncStatus>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullUpdateWrite {
    pub path: String,
    pub value: crate::schema::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_leaf_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_leaf_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullUpdatesRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default = "default_backend_connected")]
    pub backend_connected: bool,
    #[serde(default)]
    pub source: SyncApplySource,
    #[serde(default)]
    pub writes: Vec<PullUpdateWrite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_configuration_id: Option<String>,
    #[serde(default)]
    pub full_snapshot: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_update_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_configuration_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullUpdatesResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    #[serde(default)]
    pub applied_paths: Vec<String>,
    #[serde(default)]
    pub conflict_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_configuration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_configuration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_status: Option<RuntimeSyncStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetSyncStatusRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetSyncStatusResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_status: Option<RuntimeSyncStatus>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushAuditEventsRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    #[serde(default = "default_backend_connected")]
    pub backend_connected: bool,
    #[serde(default = "default_push_audit_max_events")]
    pub max_events: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushAuditEventsResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    #[serde(default)]
    pub pushed_event_ids: Vec<String>,
    pub pushed_count: u32,
    pub pending_count: u32,
    pub last_uploaded_sequence: u64,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfflineReconciliationBundle {
    pub schema_version: u32,
    pub bundle_id: String,
    pub generated_at_unix_ms: u64,
    pub committed_configuration_id: String,
    pub working_configuration_id: String,
    pub dirty_paths: Vec<String>,
    pub audit_uploaded_sequence: u64,
    pub pending_audit_count: u32,
    pub pending_audit_events: Vec<RuntimeAuditEvent>,
    pub sync_status: RuntimeSyncStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPendingSyncBundleRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    #[serde(default = "default_push_audit_max_events")]
    pub max_audit_events: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPendingSyncBundleResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<OfflineReconciliationBundle>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConfigurationIdentity {
    pub committed_configuration_id: String,
    pub working_configuration_id: String,
    pub diff_hash: String,
    pub dirty_diff_hash: String,
}

/// A unit's complete state expressed as the versioned tuple
/// `(model_version, selection_version, override_layer)` (ADR-0036 Decision
/// point 4, design doc sec 5). Each axis is a version identity string
/// (typically a content hash), consistent with how `model_hash`,
/// `resolve_hash`, and the `RuntimeConfigurationIdentity` ids are represented
/// as sha256-hex / id strings elsewhere in this contract surface:
///
/// - `model_version`: the 150% model version the unit was resolved from.
/// - `selection_version`: the preselection (facet/option) version applied.
/// - `override_layer`: the working-overlay (dirty/override) identity layered on
///   top of the resolved output.
///
/// This is the *state* a provenance lineage entry pins; it carries no actor /
/// reason / timestamp itself (those live on `ProvenanceLineageEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceVersionTriple {
    pub model_version: String,
    pub selection_version: String,
    pub override_layer: String,
}

/// One entry in a unit's provenance lineage (ADR-0036 Decision point 4, design
/// doc sec 5). An entry pins a `(model_version, selection_version,
/// override_layer)` state together with who/why/when it came to be, and a
/// parent-pointer to the prior entry, forming a content-addressed,
/// parent-linked chain.
///
/// Invariants:
/// - `entry_id` is the entry's **content address**: a deterministic sha256-hex
///   over the entry's canonical serialization (everything *except* `entry_id`
///   itself), computed by [`compute_lineage_entry_content_address`]. It is
///   reproducible from the entry's contents alone.
/// - `parent_entry_id` is the **parent-pointer**: it references the prior
///   entry's `entry_id`. It is `None` for the root of a chain. Because it is
///   part of the hashed payload, re-parenting an otherwise-identical entry
///   yields a different content address.
///
/// This is pure data-model + serialization; it carries no lifecycle/intent
/// semantics (those are sibling tasks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceLineageEntry {
    /// Content address of this entry (sha256-hex over the canonical payload).
    pub entry_id: String,
    /// The versioned `(model_version, selection_version, override_layer)` state.
    pub state: ProvenanceVersionTriple,
    /// Who produced this state (operator / technician / CI identity).
    pub actor: String,
    /// Why this state came to be. Optional, matching the surrounding contract
    /// convention for provenance reasons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When this state was recorded, unix milliseconds.
    pub timestamp_unix_ms: u64,
    /// Governance intent of the unit's active override layer at the time this
    /// state was recorded (ADR-0037; threaded from the device report in
    /// configflux-ts7z). It is the report-level (sticky-Compensating-aggregated)
    /// classification — `compensating` if ANY active override path was
    /// compensating, else `experimental` — so the ledger can distinguish a
    /// compensating deviation from an experimental one.
    ///
    /// `#[serde(default = "default_override_intent")]` for at-rest back-compat:
    /// entries persisted before this field existed deserialize as `experimental`
    /// (mirroring the `DirtyEntryMetadata.intent` precedent). This is safe — and
    /// deliberately UNLIKE the signed report, where a serde default would be a
    /// tamper trap (ADR-0038 amendment Decision A.6) — because a stored lineage
    /// entry is not re-verified against a device-produced HMAC.
    #[serde(default = "default_override_intent")]
    pub intent: OverrideIntent,
    /// Parent-pointer: the `entry_id` of the prior lineage entry, or `None` at
    /// the chain root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_entry_id: Option<String>,
}

/// A unit's provenance lineage: an ordered, parent-linked chain of
/// [`ProvenanceLineageEntry`] values. Each entry (after the root) points at its
/// predecessor via `parent_entry_id`. This is the serializable carrier for the
/// lineage; it imposes no additional semantics beyond holding the entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProvenanceLineage {
    #[serde(default)]
    pub entries: Vec<ProvenanceLineageEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetConfigurationIdentityRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetConfigurationIdentityResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<RuntimeConfigurationIdentity>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeEventsRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    #[serde(default)]
    pub from_sequence: u64,
    #[serde(default = "default_subscribe_max_events")]
    pub max_events: u32,
    #[serde(default)]
    pub event_kinds: Vec<RuntimeEventKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeEventsResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    pub from_sequence: u64,
    pub next_sequence: u64,
    pub dropped_events: u64,
    pub events: Vec<RuntimeEvent>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDeltaChangeKind {
    Set,
    Delete,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDeltaPathChange {
    pub path: String,
    pub change_kind: RuntimeDeltaChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_leaf_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_leaf_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_value: Option<crate::schema::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_value: Option<crate::schema::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDeltaManifest {
    pub schema_version: u32,
    pub manifest_id: String,
    pub base_configuration_id: String,
    pub target_configuration_id: String,
    pub changed_paths: Vec<RuntimeDeltaPathChange>,
    pub created_at_unix_ms: u64,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitConfigurationRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_base_configuration_id: Option<String>,
    #[serde(default)]
    pub changed_paths_hint: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitConfigurationResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_configuration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_configuration_id: Option<String>,
    #[serde(default)]
    pub changed_paths: Vec<RuntimeDeltaPathChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_manifest: Option<RuntimeDeltaManifest>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackMode {
    All,
    Subset,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollbackDirtyRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    pub mode: RollbackMode,
    #[serde(default)]
    pub paths: Vec<String>,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollbackDirtyResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    pub rolled_back_paths: Vec<String>,
    pub remaining_dirty_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_event_id: Option<String>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
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
    // ADR-0047 §5 lockstep: this runtime-side recipe is a SEPARATE, independent
    // duplicate of the loader_api one, cross-validated at `runtime_open`. It
    // MUST fold `defaulted_choices` with byte-identical skip-if-empty
    // serialization, or a model with an auto-bound default AND a non-empty
    // context_tags/choices would produce a `resolve_hash` the runtime cannot
    // reproduce → spurious `E_RUNTIME_HASH_MISMATCH`. Appended LAST + skipped
    // when empty keeps facet-free models byte-identical across both recipes.
    #[serde(skip_serializing_if = "ref_btreemap_is_empty")]
    defaulted_choices: &'a BTreeMap<String, String>,
}

/// `skip_serializing_if` predicate for a borrowed `&BTreeMap` field (serde hands
/// the closure `&(&BTreeMap)`; the double reference auto-derefs to the map's own
/// `is_empty`). Keeps the runtime resolve-hash pre-image byte-identical to the
/// loader recipe for facet-free models (ADR-0047 §5 lockstep).
fn ref_btreemap_is_empty(map: &&BTreeMap<String, String>) -> bool {
    map.is_empty()
}

fn default_persistence_format_version() -> u32 {
    1
}

fn default_auto_reset_enabled() -> bool {
    true
}

fn default_auto_reset_timeout_ms() -> u64 {
    30_000
}

fn default_event_next_sequence() -> u64 {
    1
}

fn default_event_buffer_capacity() -> usize {
    512
}

fn default_subscribe_max_events() -> u32 {
    256
}

fn default_runtime_sync_state() -> RuntimeSyncState {
    RuntimeSyncState::Idle
}

fn default_audit_next_sequence() -> u64 {
    1
}

fn default_backend_connected() -> bool {
    true
}

fn default_push_audit_max_events() -> u32 {
    256
}
