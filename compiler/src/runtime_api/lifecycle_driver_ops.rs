// SPDX-License-Identifier: BUSL-1.1

// Active override-lifecycle driver (configflux-ccql.4, ADR-0037, design doc
// sec 6 + sec 8).
//
// Today `apply_due_auto_resets` (shared_ops.rs) is LAZY (evaluated only inside
// other ops) and INTENT-BLIND (it unconditionally reverts every due entry). The
// per-host agent (ccql.3) supplies the ACTIVE driver: a real timer fires this
// op on schedule. This op is the intent-aware analogue of the lazy reset — it
// drives the EXISTING pure decision table `override_terminal_outcome`
// (override_intent.rs) per due entry and branches on the governing invariant:
//
//   - an outcome that `is_silent()` (experimental SilentRevert / Wipe) reuses
//     the EXACT existing revert mechanics (`revert_due_entry`, shared_ops.rs) —
//     no reset logic is reimplemented here;
//   - an outcome that is NOT silent (a compensating Survive / Escalate /
//     Surface / Carry / AbsorbAndLog) is NEVER reverted. The override stays
//     live and is brought home as an OBSERVABLE, state-gated event: a durable
//     `RuntimeAuditEventKind::Escalation` audit event (which the ccql.5
//     reporting path already exports) plus an `OverrideEscalated` runtime
//     event. The due schedule entry is then dropped so it does not re-fire on
//     every tick; the dirty overlay is untouched.
//
// This is the structural enforcement of ADR-0037 Decision 2: a compensating
// override may never end silently, because it is a proxy for an unresolved
// physical condition the system must not assume resolved itself.
//
// `include!`d into the flat `runtime_api` namespace via `operations.rs` (so no
// `//!` module docs, no per-file `use`; sibling types/helpers/constants are
// referenced directly).

/// Legacy default for the now-vestigial `terminal_event` request field
/// (configflux-h6wc). The driver SELF-CLASSIFIES the terminal event per due
/// entry from the entry's persisted [`AutoResetDeadlineKind`], so this field is
/// no longer consulted; it is retained only so requests serialized before
/// self-classification (and existing callers) still deserialize/compile. The
/// default is [`TerminalEvent::SessionLeaseExpiry`] for backward compatibility.
fn default_driver_terminal_event() -> TerminalEvent {
    TerminalEvent::SessionLeaseExpiry
}

/// Request to actively drive the override lifecycle for all due entries
/// (configflux-ccql.4). The caller (the agent's timer) supplies `now_unix_ms`
/// so active firing is deterministic and testable — this op does NOT read the
/// wall clock for the due decision, unlike the lazy ops which inject
/// `current_time_unix_ms()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveOverrideLifecycleRequest {
    pub schema_version: u32,
    pub runtime_snapshot: RuntimeSnapshot,
    /// Device-stamped current time. An entry is due iff its scheduled deadline
    /// is `<= now_unix_ms`.
    pub now_unix_ms: u64,
    /// **Deprecated / vestigial (configflux-h6wc):** the driver now derives each
    /// due entry's terminal event from its persisted [`AutoResetDeadlineKind`]
    /// (`reset_deadline_unix_ms` ⇒ lease, `hard_cap_deadline_unix_ms` ⇒ hard
    /// cap), so this caller-supplied selector is **ignored** for classification.
    /// Kept (defaulted) only for request/serde backward compatibility — both
    /// lease-expiry and hard-cap now fire on their own per-entry schedules
    /// regardless of this value.
    #[serde(default = "default_driver_terminal_event")]
    pub terminal_event: TerminalEvent,
}

/// Result of one active driver pass (configflux-ccql.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveOverrideLifecycleResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub resolve_hash: String,
    pub scope: String,
    /// The mutated snapshot on success (`None` on error, mirroring every other
    /// write op so a failed pass never silently swaps in partial state).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<RuntimeSnapshot>,
    /// Canonical paths of experimental overrides that were silently reverted.
    pub reverted_paths: Vec<String>,
    /// Canonical paths of compensating overrides that were escalated (surfaced
    /// as an observable event) instead of being silently reverted.
    pub escalated_paths: Vec<String>,
    /// The `event_id`s of the durable `Escalation` audit events emitted this
    /// pass, in order — so the caller can correlate the brought-home events.
    pub escalation_audit_event_ids: Vec<String>,
    pub error_count: u32,
    pub warning_count: u32,
    pub diagnostics: DiagnosticsReport,
}

fn drive_override_lifecycle_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> DriveOverrideLifecycleResult {
    let diagnostics = diagnostics_report(diagnostics);
    DriveOverrideLifecycleResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        reverted_paths: Vec::new(),
        escalated_paths: Vec::new(),
        escalation_audit_event_ids: Vec::new(),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics,
    }
}

/// Actively drive the override lifecycle: revert due experimental overrides
/// silently (reusing the lazy path's mechanics) and ESCALATE due compensating
/// overrides as observable events instead of reverting them (ADR-0037 Decision
/// 2). Returns the mutated snapshot plus the per-entry outcome summary.
///
/// The branch is driven entirely by the pure decision table: for each due entry
/// the terminal event is SELF-CLASSIFIED from the entry's persisted
/// `AutoResetDeadlineKind` (configflux-h6wc) and
/// `override_terminal_outcome(terminal_event, metadata.intent)` decides the
/// outcome, with `TerminalOutcome::is_silent()` as the single switch — a silent
/// outcome reverts, a non-silent outcome escalates. The request's
/// `terminal_event` selector is no longer consulted. No compensating terminal
/// path is ever silent (asserted in `override_intent.rs`), so a compensating
/// override is never reverted here.
pub fn drive_override_lifecycle(
    request: DriveOverrideLifecycleRequest,
) -> DriveOverrideLifecycleResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = request.now_unix_ms;
    // `request.terminal_event` is intentionally NOT read: the terminal event is
    // self-classified per due entry from its persisted `AutoResetDeadlineKind`
    // (configflux-h6wc).

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return drive_override_lifecycle_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set drive_override_lifecycle.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    // Normalize + validate exactly like every other op so the driver operates on
    // a well-formed snapshot (rebuilds the schedule from metadata, etc.). The
    // driver does NOT call the lazy `apply_due_auto_resets` — it IS the active
    // replacement, and routing every due entry through the intent-aware branch
    // is what makes it safe for compensating overrides.
    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return drive_override_lifecycle_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return drive_override_lifecycle_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    if snapshot.auto_reset_scheduler.pending.is_empty() {
        return drive_override_lifecycle_ok(
            snapshot,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
    }

    sort_and_dedup_schedule_entries(&mut snapshot.auto_reset_scheduler.pending);
    let due_count = snapshot
        .auto_reset_scheduler
        .pending
        .iter()
        .take_while(|entry| entry.deadline_unix_ms <= now_unix_ms)
        .count();
    if due_count == 0 {
        return drive_override_lifecycle_ok(snapshot, Vec::new(), Vec::new(), Vec::new());
    }

    let due_entries: Vec<AutoResetScheduleEntry> = snapshot
        .auto_reset_scheduler
        .pending
        .drain(0..due_count)
        .collect();

    let mut reverted_paths = Vec::new();
    let mut escalated_paths = Vec::new();
    let mut escalation_audit_event_ids = Vec::new();

    for entry in due_entries {
        // Re-validate the entry against current state (a prior iteration may have
        // changed the snapshot). A stale/superseded entry yields `None`.
        let Some((metadata, _old_value)) = validate_due_schedule_entry(&snapshot, &entry) else {
            continue;
        };

        // SELF-CLASSIFY the terminal event from the entry's persisted bound kind
        // (configflux-h6wc): a lease-expiry entry fires SessionLeaseExpiry, a
        // hard-cap entry fires HardCap — no caller-supplied selector. The pure
        // decision table then maps (terminal_event, intent) to the outcome.
        let terminal_event = terminal_event_for_deadline_kind(entry.kind);
        let outcome = override_terminal_outcome(terminal_event, metadata.intent);
        let canonical_path = canonical_runtime_path(&entry.scope_root, &entry.path);

        if outcome.is_silent() {
            // Experimental abandoned-experiment cleanup: reuse the EXACT lazy
            // revert mechanics — no reset logic reimplemented.
            if let Err(diagnostic) =
                revert_due_entry(&mut snapshot, &entry, now_unix_ms, "lease_expiry")
            {
                return drive_override_lifecycle_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![diagnostic],
                );
            }
            reverted_paths.push(canonical_path);
        } else {
            // Compensating: NEVER silently reverted. Surface it as an observable,
            // state-gated event and keep the override live. Drop the schedule
            // entry so it does not re-fire each tick; the dirty overlay is left
            // untouched.
            match escalate_compensating_entry(
                &mut snapshot,
                &entry,
                &metadata,
                terminal_event,
                now_unix_ms,
            ) {
                Ok(audit_event_id) => {
                    drop_schedule_entry(&mut snapshot, &entry);
                    escalated_paths.push(canonical_path);
                    escalation_audit_event_ids.push(audit_event_id);
                }
                Err(diagnostic) => {
                    return drive_override_lifecycle_failed(
                        model_hash,
                        resolve_hash,
                        scope,
                        vec![diagnostic],
                    );
                }
            }
        }
    }

    drive_override_lifecycle_ok(
        snapshot,
        reverted_paths,
        escalated_paths,
        escalation_audit_event_ids,
    )
}

fn drive_override_lifecycle_ok(
    snapshot: RuntimeSnapshot,
    reverted_paths: Vec<String>,
    escalated_paths: Vec<String>,
    escalation_audit_event_ids: Vec<String>,
) -> DriveOverrideLifecycleResult {
    let diagnostics = diagnostics_report(Vec::new());
    DriveOverrideLifecycleResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash: snapshot.model_hash.clone(),
        resolve_hash: snapshot.resolve_hash.clone(),
        scope: snapshot.scope.clone(),
        runtime_snapshot: Some(snapshot),
        reverted_paths,
        escalated_paths,
        escalation_audit_event_ids,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics,
    }
}

/// Emit the observable escalation for a compensating override that reached a
/// terminal event: a durable `Escalation` audit event (brought home by the
/// reporting path) plus an `OverrideEscalated` runtime event. The dirty overlay
/// is NOT cleared — the override survives, only its terminal decision is
/// surfaced. Returns the audit event's `event_id`.
fn escalate_compensating_entry(
    snapshot: &mut RuntimeSnapshot,
    entry: &AutoResetScheduleEntry,
    metadata: &DirtyEntryMetadata,
    terminal_event: TerminalEvent,
    now_unix_ms: u64,
) -> std::result::Result<String, Diagnostic> {
    let canonical_path = canonical_runtime_path(&entry.scope_root, &entry.path);

    emit_runtime_event(
        snapshot,
        RuntimeEventKind::OverrideEscalated,
        now_unix_ms,
        Some(&metadata.actor),
        metadata.reason.as_deref(),
        None,
        None,
        RuntimeEventPayload::OverrideEscalated {
            scope_root: entry.scope_root.clone(),
            path: entry.path.clone(),
            intent: metadata.intent,
            terminal_event,
            generation: entry.generation,
        },
    );

    let identity = compute_configuration_identity(snapshot)?;
    let audit_event_id = append_audit_event(
        snapshot,
        RuntimeAuditEventKind::Escalation,
        now_unix_ms,
        &metadata.actor,
        metadata.reason.as_deref(),
        vec![canonical_path],
        &identity,
        None,
        None,
    );

    // Disarm the auto-reset clock for this override WITHOUT touching its value:
    // an escalated compensating override is now a pending human decision, so the
    // bound that just fired is consumed and must not re-fire. The overlay value,
    // generation, actor, reason, and intent are all preserved — only the
    // deadline matching THIS entry's kind is cleared, so
    // `normalize_runtime_snapshot_state` does not re-push the same schedule entry
    // and re-escalate on the next tick (configflux-h6wc). A still-armed sibling
    // bound (e.g. the lease was surfaced; the hard cap remains as a later
    // backstop) is intentionally left intact. The override stays live until a
    // human (extend / promote / revert) acts on it.
    if let Some(entry_metadata) = snapshot
        .dirty_metadata
        .get_mut(&entry.scope_root)
        .and_then(|paths| paths.get_mut(&entry.path))
    {
        match entry.kind {
            AutoResetDeadlineKind::LeaseExpiry => {
                entry_metadata.reset_deadline_unix_ms = None;
            }
            AutoResetDeadlineKind::HardCap => {
                entry_metadata.hard_cap_deadline_unix_ms = None;
            }
        }
    }

    Ok(audit_event_id)
}

/// Drop every pending schedule entry for `(scope_root, path)` at or below the
/// fired generation, WITHOUT touching the dirty overlay/metadata. Used after a
/// compensating escalation so the surfaced entry does not re-fire on each tick
/// while the override itself remains live (contrast `clear_dirty_entry`, which
/// also removes the overlay).
fn drop_schedule_entry(snapshot: &mut RuntimeSnapshot, entry: &AutoResetScheduleEntry) {
    snapshot.auto_reset_scheduler.pending.retain(|pending| {
        !(pending.scope_root == entry.scope_root
            && pending.path == entry.path
            && pending.generation <= entry.generation)
    });
}
