// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the provenance lineage data model (configflux-ccql.1).
//!
//! A unit's complete state is the versioned tuple
//! `(model_version, selection_version, override_layer)` (ADR-0036 Decision
//! point 4, design doc sec 5). Each lineage entry carries actor / reason /
//! timestamp / parent-pointer provenance, is **content-addressed** (a
//! deterministic hash over its canonical serialization), and is **parent-linked**
//! into a chain. These tests pin the three acceptance criteria: content
//! addressing, parent linking, and round-trip stability.
//!
//! Pure data-model + serialization — no agent, no network, no signing. The
//! override-intent lifecycle invariant (configflux-ccql.2) is tested alongside
//! its implementation in `override_intent.rs`; the active cap/lease driver is a
//! sibling task (ccql.4).

use super::*;

fn sample_state(suffix: &str) -> ProvenanceVersionTriple {
    ProvenanceVersionTriple {
        model_version: format!("model-{suffix}"),
        selection_version: format!("selection-{suffix}"),
        override_layer: format!("override-{suffix}"),
    }
}

#[test]
fn lineage_entry_is_content_addressed() {
    let entry = ProvenanceLineageEntry::new(
        sample_state("a"),
        "alice".to_string(),
        Some("commissioning".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("entry construction");

    // The entry id is a deterministic 64-char sha256 hex content address.
    assert!(
        is_sha256_hex(&entry.entry_id),
        "entry_id must be a 64-char sha256 hex content address, got '{}'",
        entry.entry_id
    );

    // Recomputing the address from the same contents yields the identical id:
    // the address is a pure function of the entry's contents.
    let recomputed = compute_lineage_entry_content_address(
        &entry.state,
        &entry.actor,
        entry.reason.as_deref(),
        entry.timestamp_unix_ms,
        entry.intent,
        entry.parent_entry_id.as_deref(),
    )
    .expect("recompute content address");
    assert_eq!(
        entry.entry_id, recomputed,
        "stored entry_id must equal the content address recomputed from contents"
    );
}

#[test]
fn lineage_entry_content_address_changes_with_contents() {
    let base = ProvenanceLineageEntry::new(
        sample_state("a"),
        "alice".to_string(),
        Some("reason-a".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("base entry");

    // Changing any contributing field must change the content address.
    let different_actor = ProvenanceLineageEntry::new(
        sample_state("a"),
        "bob".to_string(),
        Some("reason-a".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("actor entry");
    assert_ne!(base.entry_id, different_actor.entry_id, "actor must affect address");

    let different_reason = ProvenanceLineageEntry::new(
        sample_state("a"),
        "alice".to_string(),
        Some("reason-b".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("reason entry");
    assert_ne!(base.entry_id, different_reason.entry_id, "reason must affect address");

    let different_timestamp = ProvenanceLineageEntry::new(
        sample_state("a"),
        "alice".to_string(),
        Some("reason-a".to_string()),
        1_700_000_000_001,
        OverrideIntent::Experimental,
        None,
    )
    .expect("timestamp entry");
    assert_ne!(
        base.entry_id, different_timestamp.entry_id,
        "timestamp must affect address"
    );

    let different_state = ProvenanceLineageEntry::new(
        sample_state("b"),
        "alice".to_string(),
        Some("reason-a".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("state entry");
    assert_ne!(
        base.entry_id, different_state.entry_id,
        "version triple must affect address"
    );

    // The override intent is part of the content address (configflux-ts7z): an
    // otherwise-identical entry under a different intent is a different entry.
    let different_intent = ProvenanceLineageEntry::new(
        sample_state("a"),
        "alice".to_string(),
        Some("reason-a".to_string()),
        1_700_000_000_000,
        OverrideIntent::Compensating,
        None,
    )
    .expect("intent entry");
    assert_ne!(
        base.entry_id, different_intent.entry_id,
        "override intent must affect the content address"
    );

    // Presence vs absence of an optional field is unambiguous in the address.
    let no_reason = ProvenanceLineageEntry::new(
        sample_state("a"),
        "alice".to_string(),
        None,
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("no-reason entry");
    assert_ne!(
        base.entry_id, no_reason.entry_id,
        "absent reason must be distinguishable from a present reason in the address"
    );
}

#[test]
fn lineage_entries_are_parent_linked() {
    let root = ProvenanceLineageEntry::new(
        sample_state("root"),
        "alice".to_string(),
        Some("initial".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("root entry");
    assert!(
        root.parent_entry_id.is_none(),
        "chain root must have no parent pointer"
    );

    let child = ProvenanceLineageEntry::new(
        sample_state("child"),
        "bob".to_string(),
        Some("override".to_string()),
        1_700_000_001_000,
        OverrideIntent::Experimental,
        Some(root.entry_id.clone()),
    )
    .expect("child entry");
    assert_eq!(
        child.parent_entry_id.as_deref(),
        Some(root.entry_id.as_str()),
        "child parent pointer must reference the root entry's content address"
    );

    let grandchild = ProvenanceLineageEntry::new(
        sample_state("grandchild"),
        "carol".to_string(),
        None,
        1_700_000_002_000,
        OverrideIntent::Experimental,
        Some(child.entry_id.clone()),
    )
    .expect("grandchild entry");
    assert_eq!(
        grandchild.parent_entry_id.as_deref(),
        Some(child.entry_id.as_str()),
        "grandchild parent pointer must reference the child entry's content address"
    );

    // The parent pointer is part of the content address: a child with a
    // different parent is a different entry even with identical other contents.
    let reparented = ProvenanceLineageEntry::new(
        sample_state("child"),
        "bob".to_string(),
        Some("override".to_string()),
        1_700_000_001_000,
        OverrideIntent::Experimental,
        Some(grandchild.entry_id.clone()),
    )
    .expect("reparented entry");
    assert_ne!(
        child.entry_id, reparented.entry_id,
        "parent pointer must affect the content address"
    );
}

#[test]
fn lineage_entry_round_trip_is_byte_stable() {
    let entry = ProvenanceLineageEntry::new(
        sample_state("rt"),
        "alice".to_string(),
        Some("commissioning".to_string()),
        1_700_000_000_000,
        OverrideIntent::Compensating,
        Some("a".repeat(64)),
    )
    .expect("entry");

    let first = serde_json::to_vec(&entry).expect("serialize entry");
    let decoded: ProvenanceLineageEntry =
        serde_json::from_slice(&first).expect("deserialize entry");
    let second = serde_json::to_vec(&decoded).expect("re-serialize entry");

    assert_eq!(first, second, "serialize -> deserialize -> serialize must be byte-stable");
    assert_eq!(entry, decoded, "round-trip must preserve the entry value");
    assert_eq!(
        entry.entry_id, decoded.entry_id,
        "content address must survive the round-trip unchanged"
    );
    assert_eq!(
        decoded.intent,
        OverrideIntent::Compensating,
        "a compensating intent must survive the lineage-entry round-trip"
    );
}

#[test]
fn lineage_entry_missing_intent_defaults_to_experimental_at_rest() {
    // A lineage entry persisted BEFORE the `intent` field existed has no `intent`
    // key. Because the stored entry is not re-verified against a device HMAC
    // (ADR-0038 amendment Decision A.6 / B), it deserializes via the serde
    // default to `experimental` (abandoned-experiment cleanup is the safe
    // default) rather than failing. This is the at-rest back-compat the amendment
    // relies on for already-stored chains.
    let legacy = r#"{
        "entry_id": "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        "state": {
            "model_version": "model-a",
            "selection_version": "selection-a",
            "override_layer": "override-a"
        },
        "actor": "alice",
        "reason": "pre-intent entry",
        "timestamp_unix_ms": 1700000000000
    }"#;
    let decoded: ProvenanceLineageEntry =
        serde_json::from_str(legacy).expect("pre-intent lineage entry must still deserialize");
    assert_eq!(
        decoded.intent,
        OverrideIntent::Experimental,
        "a lineage entry without `intent` defaults to experimental at rest"
    );
}

#[test]
fn lineage_chain_round_trip_is_byte_stable() {
    let root = ProvenanceLineageEntry::new(
        sample_state("root"),
        "alice".to_string(),
        Some("initial".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("root");
    let child = ProvenanceLineageEntry::new(
        sample_state("child"),
        "bob".to_string(),
        None,
        1_700_000_001_000,
        OverrideIntent::Experimental,
        Some(root.entry_id.clone()),
    )
    .expect("child");

    let chain = ProvenanceLineage {
        entries: vec![root.clone(), child.clone()],
    };

    let first = serde_json::to_vec(&chain).expect("serialize chain");
    let decoded: ProvenanceLineage = serde_json::from_slice(&first).expect("deserialize chain");
    let second = serde_json::to_vec(&decoded).expect("re-serialize chain");

    assert_eq!(first, second, "chain round-trip must be byte-stable");
    assert_eq!(chain, decoded, "chain round-trip must preserve every entry");
    assert_eq!(
        decoded.entries[1].parent_entry_id.as_deref(),
        Some(root.entry_id.as_str()),
        "parent links must survive the chain round-trip"
    );
}

#[test]
fn lineage_entry_address_excludes_self_id() {
    // The content address must be reproducible from the entry's contents alone,
    // which requires excluding `entry_id` itself from the hashed payload. Proven
    // by recomputing the address purely from contents and matching the stored id.
    let entry = ProvenanceLineageEntry::new(
        sample_state("self"),
        "alice".to_string(),
        Some("reason".to_string()),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        Some("b".repeat(64)),
    )
    .expect("entry");

    let from_contents = compute_lineage_entry_content_address(
        &entry.state,
        &entry.actor,
        entry.reason.as_deref(),
        entry.timestamp_unix_ms,
        entry.intent,
        entry.parent_entry_id.as_deref(),
    )
    .expect("address from contents");

    assert_eq!(
        entry.entry_id, from_contents,
        "content address must be reproducible from contents (entry_id excluded from the hash)"
    );
}

// --- active cap/lease driver (configflux-ccql.4, ADR-0037, design doc sec 6) --
//
// `drive_override_lifecycle` is the intent-aware active analogue of the lazy
// `apply_due_auto_resets`: it drives the pure `override_terminal_outcome` table
// per due entry. A silent outcome (experimental) reverts; a non-silent outcome
// (compensating) is escalated as an observable event and NEVER reverted. These
// tests pin both branches plus the not-yet-due no-op.

fn driver_fixture_snapshot() -> RuntimeSnapshot {
    // A one-component resolved config with a single runtime-writable scalar,
    // built via the PUBLIC, solver-free `runtime_open` (no `.ccm`, no solver).
    let resolved_output = serde_json::json!({
        "ctl": {
            "package": "pump",
            "version": "1.0.0",
            "components": {
                "ctl": {
                    "type": "controller",
                    "params": {
                        "threshold": {
                            "value": 4.5,
                            "type": "float",
                            "unit": "bar",
                            "safety": "q_m",
                            "lifecycle": "runtime",
                            "access": "technician",
                            "req_id": null,
                            "doc": null,
                            "limits": { "min": 0.0, "max": 10.0, "min_len": null, "max_len": null }
                        }
                    }
                }
            }
        }
    });
    let request = RuntimeOpenRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash: "ab".repeat(32),
        ccm_ref: String::new(),
        resolve_hash: "cd".repeat(32),
        scope: "component:ctl".to_string(),
        resolved_output,
        resolved_component_dependencies: Default::default(),
        resolved_artifacts: Default::default(),
        context_tags: Default::default(),
        choices: Default::default(),
        defaulted_choices: Default::default(),
        committed_overlay: Default::default(),
        dirty_overlay: Default::default(),
        dirty_generations: Default::default(),
        dirty_metadata: Default::default(),
        auto_reset_policy: Default::default(),
        auto_reset_scheduler: Default::default(),
        event_bus: Default::default(),
        sync_status: Default::default(),
        audit_events: Default::default(),
        audit_next_sequence: 1,
        audit_uploaded_sequence: 0,
        persistence_format_version: 1,
        persistence_journal_sequence: 0,
    };
    let result = runtime_open(request);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver fixture runtime_open must succeed (solver-free): {:?}",
        result.diagnostics
    );
    result
        .runtime_snapshot
        .expect("runtime_open Ok must carry a snapshot")
}

const DRIVER_PATH: &str = "component.ctl.param.threshold";
const DRIVER_SCOPE_ROOT: &str = "ctl";
// A deterministic clock base for the driver tests. The active driver takes an
// injected `now_unix_ms`, so the tests pin both the entry's `dirty_since` and
// the fired clock to this base rather than the real wall clock.
const DRIVER_BASE_UNIX_MS: u64 = 1_700_000_000_000;

// Write an override into the snapshot and force its `intent` + deadline so the
// driver sees a single due entry of the given intent. The write path hardcodes
// `Experimental`, so the compensating case is constructed directly on the
// metadata (operator-supplied intent threading is a sibling follow-up).
fn seed_due_override(
    snapshot: &mut RuntimeSnapshot,
    intent: OverrideIntent,
    deadline_unix_ms: u64,
) {
    let result = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: DRIVER_PATH.to_string(),
        value: crate::schema::Value::Float(7.5),
        intent: OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "seed set_parameter must succeed: {:?}",
        result.diagnostics
    );
    let mut seeded = result
        .runtime_snapshot
        .expect("set_parameter Ok carries a snapshot");

    // Pin the intent + the schedule deadline so the driver classifies this as a
    // due terminal event with the intent under test. The write path hardcodes
    // actor/reason/intent, so set them here to model a technician compensation.
    let generation = seeded.dirty_generations[DRIVER_SCOPE_ROOT][DRIVER_PATH];
    let metadata = seeded
        .dirty_metadata
        .get_mut(DRIVER_SCOPE_ROOT)
        .and_then(|m| m.get_mut(DRIVER_PATH))
        .expect("dirty metadata present after write");
    metadata.intent = intent;
    metadata.actor = "tech-42".to_string();
    metadata.reason = Some("loosened until part swap".to_string());
    // Pin dirty_since under the deadline so the snapshot validator accepts the
    // entry (deadline must be >= dirty_since). The write stamped dirty_since with
    // the real wall clock; override it with our deterministic base.
    metadata.dirty_since_unix_ms = DRIVER_BASE_UNIX_MS;
    metadata.reset_deadline_unix_ms = Some(deadline_unix_ms);

    seeded.auto_reset_scheduler.pending = vec![AutoResetScheduleEntry {
        scope_root: DRIVER_SCOPE_ROOT.to_string(),
        path: DRIVER_PATH.to_string(),
        generation,
        deadline_unix_ms,
        kind: AutoResetDeadlineKind::LeaseExpiry,
    }];
    *snapshot = seeded;
}

fn overlay_present(snapshot: &RuntimeSnapshot) -> bool {
    snapshot
        .dirty_overlay
        .get(DRIVER_SCOPE_ROOT)
        .and_then(|m| m.get(DRIVER_PATH))
        .is_some()
}

#[test]
fn driver_reverts_due_experimental_on_schedule() {
    let mut snapshot = driver_fixture_snapshot();
    let deadline = 1_700_000_030_000u64;
    seed_due_override(&mut snapshot, OverrideIntent::Experimental, deadline);
    assert!(overlay_present(&snapshot), "override is live before the driver runs");

    // Fire the active driver AT the deadline (entry is due: deadline <= now).
    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        now_unix_ms: deadline,
        terminal_event: TerminalEvent::SessionLeaseExpiry,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver pass must succeed: {:?}",
        result.diagnostics
    );
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    // Experimental lease expiry is a silent revert: the overlay is cleared and a
    // durable Reset audit event is emitted (NOT an escalation).
    assert!(
        !overlay_present(&after),
        "an experimental override is reverted on lease expiry"
    );
    assert_eq!(result.reverted_paths.len(), 1, "exactly one revert");
    assert!(result.escalated_paths.is_empty(), "no escalation for experimental");
    assert!(
        after.audit_events.iter().any(|e| e.event_kind == RuntimeAuditEventKind::Reset),
        "a Reset audit event is emitted on silent revert"
    );
    assert!(
        !after.audit_events.iter().any(|e| e.event_kind == RuntimeAuditEventKind::Escalation),
        "no Escalation audit event for an experimental revert"
    );
}

#[test]
fn driver_escalates_compensating_hard_cap_never_silent() {
    let mut snapshot = driver_fixture_snapshot();
    let deadline = 1_700_000_030_000u64;
    seed_due_override(&mut snapshot, OverrideIntent::Compensating, deadline);
    assert!(overlay_present(&snapshot), "compensating override is live before the driver");

    // The cardinal invariant, straight from the pure table: a compensating hard
    // cap is NEVER silent.
    assert!(
        !override_terminal_outcome(TerminalEvent::HardCap, OverrideIntent::Compensating)
            .is_silent(),
        "ADR-0037: a compensating hard cap is never silent"
    );

    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        now_unix_ms: deadline,
        terminal_event: TerminalEvent::HardCap,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver pass must succeed: {:?}",
        result.diagnostics
    );
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    // The override is NEVER silently reverted: the overlay is still present...
    assert!(
        overlay_present(&after),
        "a compensating override is NEVER silently reverted by the active driver"
    );
    assert!(result.reverted_paths.is_empty(), "no silent revert for compensating");
    assert_eq!(result.escalated_paths.len(), 1, "the compensating override is escalated");

    // ...and the termination is brought home as an observable event: a durable
    // Escalation audit event (exported by the reporting path) + an
    // OverrideEscalated runtime event. NEVER a Reset.
    assert_eq!(
        result.escalation_audit_event_ids.len(),
        1,
        "the escalation reports its audit event id"
    );
    assert!(
        after.audit_events.iter().any(|e| e.event_kind == RuntimeAuditEventKind::Escalation),
        "an Escalation audit event is emitted (brought home, observable)"
    );
    assert!(
        !after.audit_events.iter().any(|e| e.event_kind == RuntimeAuditEventKind::Reset),
        "a compensating override is never reverted, so no Reset audit event"
    );
    assert!(
        after
            .event_bus
            .events
            .iter()
            .any(|e| e.event_kind == RuntimeEventKind::OverrideEscalated),
        "an OverrideEscalated runtime event surfaces the decision"
    );

    // The schedule entry is dropped so the escalation does not re-fire every
    // tick (the override stays live, but it is not re-escalated on the next
    // pass with no new event).
    let audit_count_after_first = after
        .audit_events
        .iter()
        .filter(|e| e.event_kind == RuntimeAuditEventKind::Escalation)
        .count();
    let second = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: after,
        now_unix_ms: deadline + 60_000,
        terminal_event: TerminalEvent::HardCap,
    });
    let after_second = second.runtime_snapshot.expect("second pass Ok");
    assert!(
        second.escalated_paths.is_empty(),
        "a second tick does not re-escalate the same surfaced override"
    );
    assert_eq!(
        after_second
            .audit_events
            .iter()
            .filter(|e| e.event_kind == RuntimeAuditEventKind::Escalation)
            .count(),
        audit_count_after_first,
        "no duplicate escalation audit event on a subsequent tick"
    );
    assert!(
        overlay_present(&after_second),
        "the compensating override remains live across ticks"
    );
}

#[test]
fn driver_ignores_not_yet_due_entries() {
    let mut snapshot = driver_fixture_snapshot();
    let deadline = 1_700_000_030_000u64;
    seed_due_override(&mut snapshot, OverrideIntent::Experimental, deadline);

    // Fire BEFORE the deadline: nothing is due.
    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        now_unix_ms: deadline - 1,
        terminal_event: TerminalEvent::SessionLeaseExpiry,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver pass must succeed: {:?}",
        result.diagnostics
    );
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    assert!(result.reverted_paths.is_empty(), "nothing reverts before the deadline");
    assert!(result.escalated_paths.is_empty(), "nothing escalates before the deadline");
    assert!(overlay_present(&after), "the override stays live before its deadline");
    assert_eq!(
        after.auto_reset_scheduler.pending.len(),
        1,
        "the not-yet-due schedule entry is left pending"
    );
}

// --- dual-bound self-classification (configflux-h6wc, ADR-0037 Decision 3) ----
//
// `DirtyEntryMetadata` now persists BOTH a session-lease deadline
// (`reset_deadline_unix_ms`) and a hard-cap deadline
// (`hard_cap_deadline_unix_ms`), and each scheduled `AutoResetScheduleEntry`
// carries an `AutoResetDeadlineKind`. The active driver SELF-CLASSIFIES the due
// terminal event from the entry's own `kind` — it does NOT consult the
// request's `terminal_event` selector. These tests pin: (1) lease-kind classify,
// (2) hard-cap-kind classify, (3) both fire on their own per-entry schedules,
// (4) serde defaults for old snapshots, (5) the compensating invariant holds via
// the dual-bound path.

// Seed an override at `DRIVER_PATH` with an explicit intent and an explicit pair
// of (lease, hard_cap) deadlines, and stage one schedule entry of `kind` at
// `entry_deadline`. Lets a test pin exactly which bound is due and which kind the
// driver sees, independent of any caller-supplied selector.
fn seed_dual_bound_override(
    snapshot: &mut RuntimeSnapshot,
    intent: OverrideIntent,
    lease_deadline_unix_ms: Option<u64>,
    hard_cap_deadline_unix_ms: Option<u64>,
    entry_kind: AutoResetDeadlineKind,
    entry_deadline_unix_ms: u64,
) {
    let result = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: DRIVER_PATH.to_string(),
        value: crate::schema::Value::Float(7.5),
        intent: OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "seed set_parameter must succeed: {:?}",
        result.diagnostics
    );
    let mut seeded = result
        .runtime_snapshot
        .expect("set_parameter Ok carries a snapshot");

    let generation = seeded.dirty_generations[DRIVER_SCOPE_ROOT][DRIVER_PATH];
    let metadata = seeded
        .dirty_metadata
        .get_mut(DRIVER_SCOPE_ROOT)
        .and_then(|m| m.get_mut(DRIVER_PATH))
        .expect("dirty metadata present after write");
    metadata.intent = intent;
    metadata.actor = "tech-42".to_string();
    metadata.reason = Some("loosened until part swap".to_string());
    metadata.dirty_since_unix_ms = DRIVER_BASE_UNIX_MS;
    metadata.reset_deadline_unix_ms = lease_deadline_unix_ms;
    metadata.hard_cap_deadline_unix_ms = hard_cap_deadline_unix_ms;

    seeded.auto_reset_scheduler.pending = vec![AutoResetScheduleEntry {
        scope_root: DRIVER_SCOPE_ROOT.to_string(),
        path: DRIVER_PATH.to_string(),
        generation,
        deadline_unix_ms: entry_deadline_unix_ms,
        kind: entry_kind,
    }];
    *snapshot = seeded;
}

// Extract the `terminal_event` recorded on the most recent OverrideEscalated
// runtime event, if any. This is the observable proof of which terminal event
// the driver classified — it is written into the event payload by the driver.
fn last_escalated_terminal_event(snapshot: &RuntimeSnapshot) -> Option<TerminalEvent> {
    snapshot.event_bus.events.iter().rev().find_map(|event| {
        if let RuntimeEventPayload::OverrideEscalated { terminal_event, .. } = &event.payload {
            Some(*terminal_event)
        } else {
            None
        }
    })
}

#[test]
fn driver_self_classifies_lease_expiry_from_entry_kind() {
    // A LEASE-kind entry on a compensating override is due. Pass a deliberately
    // WRONG selector (HardCap) on the request: the driver must IGNORE it and
    // classify SessionLeaseExpiry from the entry's own kind. The compensating
    // lease-expiry outcome (Survive) is non-silent, so it escalates and records
    // the classified terminal event in the surfaced event.
    let mut snapshot = driver_fixture_snapshot();
    let lease_deadline = 1_700_000_030_000u64;
    seed_dual_bound_override(
        &mut snapshot,
        OverrideIntent::Compensating,
        Some(lease_deadline),
        None,
        AutoResetDeadlineKind::LeaseExpiry,
        lease_deadline,
    );

    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        now_unix_ms: lease_deadline,
        // Deliberately the WRONG selector — must NOT be consulted.
        terminal_event: TerminalEvent::HardCap,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver pass must succeed: {:?}",
        result.diagnostics
    );
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    assert_eq!(result.escalated_paths.len(), 1, "compensating override is escalated");
    assert_eq!(
        last_escalated_terminal_event(&after),
        Some(TerminalEvent::SessionLeaseExpiry),
        "the driver classifies the LEASE-kind entry as SessionLeaseExpiry from persisted state, \
         NOT from the request's (wrong) HardCap selector"
    );
}

#[test]
fn driver_self_classifies_hard_cap_from_entry_kind() {
    // A HARD-CAP-kind entry on a compensating override is due (the lease is not).
    // Pass a deliberately WRONG selector (SessionLeaseExpiry): the driver must
    // classify HardCap from the entry's own kind and record it in the surfaced
    // event. Both compensating outcomes are non-silent (never reverted), so the
    // ONLY observable difference is the classified terminal event.
    let mut snapshot = driver_fixture_snapshot();
    let hard_cap_deadline = 1_700_000_900_000u64;
    // Only the hard-cap bound exists (lease is None) so exactly one entry is due
    // and the observable is purely the classified terminal event.
    seed_dual_bound_override(
        &mut snapshot,
        OverrideIntent::Compensating,
        None,
        Some(hard_cap_deadline),
        AutoResetDeadlineKind::HardCap,
        hard_cap_deadline,
    );

    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        now_unix_ms: hard_cap_deadline,
        // Deliberately the WRONG selector — must NOT be consulted.
        terminal_event: TerminalEvent::SessionLeaseExpiry,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver pass must succeed: {:?}",
        result.diagnostics
    );
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    assert_eq!(result.escalated_paths.len(), 1, "compensating override is escalated");
    assert!(
        overlay_present(&after),
        "a compensating override is NEVER silently reverted at the hard cap"
    );
    assert_eq!(
        last_escalated_terminal_event(&after),
        Some(TerminalEvent::HardCap),
        "the driver classifies the HARD-CAP-kind entry as HardCap from persisted state, \
         NOT from the request's (wrong) SessionLeaseExpiry selector"
    );
}

#[test]
fn driver_fires_both_deadlines_on_their_own_schedules() {
    // Seed BOTH a lease entry and a hard-cap entry for the same override, both
    // due in a single pass (built directly so each kind is staged). An
    // experimental lease expiry is a silent revert; a hard-cap entry on the same
    // (now-reverted) override is superseded after the revert — so the observable
    // is that the driver processed each entry by its own kind without a selector.
    let mut snapshot = driver_fixture_snapshot();
    let lease_deadline = 1_700_000_030_000u64;
    let hard_cap_deadline = 1_700_000_030_000u64; // same instant: both due at once
    let now = 1_700_000_060_000u64;

    // Seed a compensating override with both bounds; stage BOTH entries.
    let result = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: DRIVER_PATH.to_string(),
        value: crate::schema::Value::Float(7.5),
        intent: OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(result.status, OperationStatus::Ok, "{:?}", result.diagnostics);
    let mut seeded = result.runtime_snapshot.expect("snapshot");
    let generation = seeded.dirty_generations[DRIVER_SCOPE_ROOT][DRIVER_PATH];
    let metadata = seeded
        .dirty_metadata
        .get_mut(DRIVER_SCOPE_ROOT)
        .and_then(|m| m.get_mut(DRIVER_PATH))
        .expect("dirty metadata present");
    metadata.intent = OverrideIntent::Compensating;
    metadata.actor = "tech-42".to_string();
    metadata.reason = Some("dual bound".to_string());
    metadata.dirty_since_unix_ms = DRIVER_BASE_UNIX_MS;
    metadata.reset_deadline_unix_ms = Some(lease_deadline);
    metadata.hard_cap_deadline_unix_ms = Some(hard_cap_deadline);
    seeded.auto_reset_scheduler.pending = vec![
        AutoResetScheduleEntry {
            scope_root: DRIVER_SCOPE_ROOT.to_string(),
            path: DRIVER_PATH.to_string(),
            generation,
            deadline_unix_ms: lease_deadline,
            kind: AutoResetDeadlineKind::LeaseExpiry,
        },
        AutoResetScheduleEntry {
            scope_root: DRIVER_SCOPE_ROOT.to_string(),
            path: DRIVER_PATH.to_string(),
            generation,
            deadline_unix_ms: hard_cap_deadline,
            kind: AutoResetDeadlineKind::HardCap,
        },
    ];
    snapshot = seeded;

    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        now_unix_ms: now,
        terminal_event: TerminalEvent::SessionLeaseExpiry,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver pass must succeed: {:?}",
        result.diagnostics
    );
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    // A compensating override is never silently reverted by EITHER bound; the
    // override stays live and is surfaced. Both bound entries were resolved from
    // their own kind (no selector), and the schedule no longer re-fires them.
    assert!(
        overlay_present(&after),
        "a compensating override stays live across both bounds"
    );
    assert!(
        result.reverted_paths.is_empty(),
        "a compensating override is never silently reverted by lease OR hard cap"
    );
    assert!(
        !result.escalated_paths.is_empty(),
        "at least one bound surfaced the compensating override as an observable escalation"
    );
    assert!(
        after
            .auto_reset_scheduler
            .pending
            .iter()
            .all(|e| e.deadline_unix_ms > now),
        "no due dual-bound entry is left pending to re-fire on the next tick"
    );
}

#[test]
fn dirty_metadata_defaults_hard_cap_deadline_on_old_snapshot() {
    // A snapshot serialized before `hard_cap_deadline_unix_ms` existed has no
    // such key. It must deserialize with the default (None — no hard cap), not
    // fail, so pre-existing single-deadline overrides round-trip unchanged.
    let legacy = r#"{
        "actor": "runtime_api.set_parameter",
        "dirty_since_unix_ms": 1700000000000,
        "reset_deadline_unix_ms": 1700000060000,
        "generation": 1,
        "intent": "compensating"
    }"#;
    let decoded: DirtyEntryMetadata =
        serde_json::from_str(legacy).expect("pre-dual-bound snapshot must still deserialize");
    assert_eq!(
        decoded.hard_cap_deadline_unix_ms, None,
        "missing hard_cap_deadline_unix_ms defaults to None (no hard cap scheduled)"
    );
    assert_eq!(
        decoded.reset_deadline_unix_ms,
        Some(1_700_000_060_000),
        "the existing session-lease deadline is preserved"
    );

    // Round-trip with the field present is byte-stable.
    let dual = DirtyEntryMetadata {
        actor: "tech-9".to_string(),
        reason: None,
        dirty_since_unix_ms: 1_700_000_000_000,
        reset_deadline_unix_ms: Some(1_700_000_060_000),
        hard_cap_deadline_unix_ms: Some(1_700_000_900_000),
        generation: 2,
        intent: OverrideIntent::Compensating,
    };
    let first = serde_json::to_vec(&dual).expect("serialize");
    let round: DirtyEntryMetadata = serde_json::from_slice(&first).expect("deserialize");
    let second = serde_json::to_vec(&round).expect("re-serialize");
    assert_eq!(first, second, "dual-bound metadata round-trip is byte-stable");
    assert_eq!(round, dual, "dual-bound metadata survives the round-trip");
}

#[test]
fn schedule_entry_defaults_kind_to_lease_expiry_on_old_snapshot() {
    // A schedule entry serialized before the `kind` discriminator existed has no
    // `kind` key and must deserialize as LeaseExpiry — its prior single-deadline
    // meaning (the session-lease bound).
    let legacy = r#"{
        "scope_root": "ctl",
        "path": "component.ctl.param.threshold",
        "generation": 1,
        "deadline_unix_ms": 1700000060000
    }"#;
    let decoded: AutoResetScheduleEntry =
        serde_json::from_str(legacy).expect("pre-kind schedule entry must still deserialize");
    assert_eq!(
        decoded.kind,
        AutoResetDeadlineKind::LeaseExpiry,
        "a kind-less schedule entry defaults to the session-lease bound"
    );
    // And the kind mapping is the single source of classification truth.
    assert_eq!(
        terminal_event_for_deadline_kind(AutoResetDeadlineKind::LeaseExpiry),
        TerminalEvent::SessionLeaseExpiry
    );
    assert_eq!(
        terminal_event_for_deadline_kind(AutoResetDeadlineKind::HardCap),
        TerminalEvent::HardCap
    );
}

#[test]
fn driver_never_silently_reverts_compensating_via_dual_bound_path() {
    // The ccql.2 cardinal invariant must still hold when the hard cap is the due
    // bound and the driver classifies it from persisted state alone (no
    // selector). The override stays live; an Escalation audit event is brought
    // home; no Reset is ever emitted.
    let mut snapshot = driver_fixture_snapshot();
    let hard_cap_deadline = 1_700_000_500_000u64;
    seed_dual_bound_override(
        &mut snapshot,
        OverrideIntent::Compensating,
        None,
        Some(hard_cap_deadline),
        AutoResetDeadlineKind::HardCap,
        hard_cap_deadline,
    );

    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        now_unix_ms: hard_cap_deadline,
        terminal_event: TerminalEvent::SessionLeaseExpiry,
    });
    assert_eq!(result.status, OperationStatus::Ok, "{:?}", result.diagnostics);
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    assert!(
        overlay_present(&after),
        "a compensating override is NEVER silently reverted, even at the hard cap"
    );
    assert!(result.reverted_paths.is_empty(), "no silent revert for compensating");
    assert!(
        after
            .audit_events
            .iter()
            .any(|e| e.event_kind == RuntimeAuditEventKind::Escalation),
        "the hard-cap termination is brought home as an observable Escalation audit event"
    );
    assert!(
        !after
            .audit_events
            .iter()
            .any(|e| e.event_kind == RuntimeAuditEventKind::Reset),
        "a compensating override is never reverted, so no Reset audit event"
    );
}

// --- operator-declared override intent at write time (configflux-irid) --------
//
// `apply_dirty_write` historically hardcoded `OverrideIntent::Experimental`, so
// the ONLY way a compensating override reached the active driver was a
// pre-seeded snapshot (the driver tests above mutate `metadata.intent`
// directly). These tests pin the closed loop: a technician declares
// `intent=Compensating` (+actor/reason) at `SetParameter[Atomically]` time, it
// is persisted into `DirtyEntryMetadata.intent`, and the active driver then sees
// it WITHOUT any pre-seeding. The omitted-intent path still defaults to
// Experimental (today's behavior unchanged). ADR-0037 Decision 1, design doc
// sec 6/sec 7.

// Re-pin only the deadline/schedule of an already-written override so the active
// driver fires it as a due terminal event of the given `kind`, WITHOUT touching
// `intent` — the whole point is that `intent` arrived through the write path.
fn force_due_preserving_intent(
    snapshot: &mut RuntimeSnapshot,
    deadline_unix_ms: u64,
    kind: AutoResetDeadlineKind,
) {
    let generation = snapshot.dirty_generations[DRIVER_SCOPE_ROOT][DRIVER_PATH];
    let metadata = snapshot
        .dirty_metadata
        .get_mut(DRIVER_SCOPE_ROOT)
        .and_then(|m| m.get_mut(DRIVER_PATH))
        .expect("dirty metadata present after write");
    // Pin dirty_since under the deadline so the snapshot validator accepts the
    // entry (deadline must be >= dirty_since). `intent` is intentionally left as
    // the write path persisted it.
    metadata.dirty_since_unix_ms = DRIVER_BASE_UNIX_MS;
    match kind {
        AutoResetDeadlineKind::LeaseExpiry => {
            metadata.reset_deadline_unix_ms = Some(deadline_unix_ms);
            metadata.hard_cap_deadline_unix_ms = None;
        }
        AutoResetDeadlineKind::HardCap => {
            metadata.reset_deadline_unix_ms = None;
            metadata.hard_cap_deadline_unix_ms = Some(deadline_unix_ms);
        }
    }

    snapshot.auto_reset_scheduler.pending = vec![AutoResetScheduleEntry {
        scope_root: DRIVER_SCOPE_ROOT.to_string(),
        path: DRIVER_PATH.to_string(),
        generation,
        deadline_unix_ms,
        kind,
    }];
}

#[test]
fn set_parameter_declared_compensating_reaches_driver_without_preseed() {
    let snapshot = driver_fixture_snapshot();

    // A technician declares a COMPENSATING override at write time (the issue's
    // core scenario: "loosened until the part is swapped"). No snapshot
    // pre-seeding — the intent travels through the request.
    let write = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: DRIVER_PATH.to_string(),
        value: crate::schema::Value::Float(7.5),
        intent: OverrideIntent::Compensating,
        actor: Some("tech-42".to_string()),
        reason: Some("loosened until part swap".to_string()),
    });
    assert_eq!(
        write.status,
        OperationStatus::Ok,
        "declared-compensating set_parameter must succeed: {:?}",
        write.diagnostics
    );
    let mut seeded = write
        .runtime_snapshot
        .expect("set_parameter Ok carries a snapshot");

    // The persisted metadata carries the declared intent/actor/reason — proving
    // the write path threaded it (NOT a pre-seeded snapshot).
    let metadata = seeded.dirty_metadata[DRIVER_SCOPE_ROOT][DRIVER_PATH].clone();
    assert_eq!(
        metadata.intent,
        OverrideIntent::Compensating,
        "the operator-declared compensating intent must be persisted by the write path"
    );
    assert_eq!(metadata.actor, "tech-42", "declared actor is persisted");
    assert_eq!(
        metadata.reason.as_deref(),
        Some("loosened until part swap"),
        "declared reason is persisted"
    );

    // Fire the active driver at the hard cap. Because the persisted intent is
    // Compensating, the cardinal invariant (ccql.2) governs: it is escalated,
    // never silently reverted — reached WITHOUT pre-seeding metadata.
    let hard_cap_deadline = DRIVER_BASE_UNIX_MS + 500_000;
    force_due_preserving_intent(
        &mut seeded,
        hard_cap_deadline,
        AutoResetDeadlineKind::HardCap,
    );
    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: seeded,
        now_unix_ms: hard_cap_deadline,
        terminal_event: TerminalEvent::SessionLeaseExpiry,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "driver pass must succeed: {:?}",
        result.diagnostics
    );
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    assert!(
        overlay_present(&after),
        "a write-path-declared compensating override is NEVER silently reverted"
    );
    assert!(
        result.reverted_paths.is_empty(),
        "no silent revert for a declared compensating override"
    );
    assert_eq!(
        result.escalated_paths.len(),
        1,
        "the compensating override is escalated as an observable event"
    );
    assert!(
        after
            .audit_events
            .iter()
            .any(|e| e.event_kind == RuntimeAuditEventKind::Escalation),
        "the terminal event is brought home as an Escalation audit event"
    );
    assert!(
        !after
            .audit_events
            .iter()
            .any(|e| e.event_kind == RuntimeAuditEventKind::Reset),
        "a declared compensating override is never reverted, so no Reset audit event"
    );
}

#[test]
fn set_parameter_omitted_intent_defaults_experimental() {
    let snapshot = driver_fixture_snapshot();

    // Omitting the intent must preserve today's behavior exactly: the persisted
    // intent defaults to Experimental, so a lease expiry silently reverts.
    let write = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: DRIVER_PATH.to_string(),
        value: crate::schema::Value::Float(7.5),
        intent: OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(
        write.status,
        OperationStatus::Ok,
        "set_parameter must succeed: {:?}",
        write.diagnostics
    );
    let mut seeded = write
        .runtime_snapshot
        .expect("set_parameter Ok carries a snapshot");

    let metadata = seeded.dirty_metadata[DRIVER_SCOPE_ROOT][DRIVER_PATH].clone();
    assert_eq!(
        metadata.intent,
        OverrideIntent::Experimental,
        "omitting intent defaults to Experimental (today's behavior unchanged)"
    );
    // The omitted actor falls back to the default dirty actor, exactly as before.
    assert_eq!(
        metadata.actor, DEFAULT_DIRTY_ACTOR,
        "omitted actor falls back to the default dirty actor"
    );

    let lease_deadline = DRIVER_BASE_UNIX_MS + 30_000;
    force_due_preserving_intent(
        &mut seeded,
        lease_deadline,
        AutoResetDeadlineKind::LeaseExpiry,
    );
    let result = drive_override_lifecycle(DriveOverrideLifecycleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: seeded,
        now_unix_ms: lease_deadline,
        terminal_event: TerminalEvent::SessionLeaseExpiry,
    });
    assert_eq!(result.status, OperationStatus::Ok, "{:?}", result.diagnostics);
    let after = result.runtime_snapshot.expect("driver Ok carries a snapshot");

    assert!(
        !overlay_present(&after),
        "an experimental (default) override is silently reverted on lease expiry"
    );
    assert_eq!(
        result.reverted_paths.len(),
        1,
        "exactly one silent revert on the default experimental path"
    );
    assert!(
        result.escalated_paths.is_empty(),
        "no escalation for a default experimental override"
    );
}

#[test]
fn set_parameters_atomically_declared_compensating_persists() {
    let snapshot = driver_fixture_snapshot();

    let write = set_parameters_atomically(SetParametersAtomicallyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        writes: vec![AtomicParameterWrite {
            path: DRIVER_PATH.to_string(),
            value: crate::schema::Value::Float(8.0),
        }],
        actor: "tech-77".to_string(),
        reason: Some("atomic compensating tweak".to_string()),
        expected_working_configuration_id: None,
        intent: OverrideIntent::Compensating,
    });
    assert_eq!(
        write.status,
        OperationStatus::Ok,
        "declared-compensating atomic write must succeed: {:?}",
        write.diagnostics
    );
    let seeded = write
        .runtime_snapshot
        .expect("set_parameters_atomically Ok carries a snapshot");

    let metadata = seeded.dirty_metadata[DRIVER_SCOPE_ROOT][DRIVER_PATH].clone();
    assert_eq!(
        metadata.intent,
        OverrideIntent::Compensating,
        "the declared compensating intent is persisted for an atomic write"
    );
    assert_eq!(metadata.actor, "tech-77", "atomic write actor is persisted");
}

#[test]
fn set_parameters_atomically_omitted_intent_defaults_experimental() {
    let snapshot = driver_fixture_snapshot();

    // An atomic request whose `intent` is omitted on the wire deserializes to the
    // Experimental default and persists it — today's behavior unchanged.
    let payload = serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": snapshot,
        "writes": [ { "path": DRIVER_PATH, "value": 8.0 } ],
        "actor": "tech-77"
    });
    let request: SetParametersAtomicallyRequest =
        serde_json::from_value(payload).expect("atomic request without intent must deserialize");
    assert_eq!(
        request.intent,
        OverrideIntent::Experimental,
        "an omitted atomic-request intent defaults to Experimental"
    );

    let write = set_parameters_atomically(request);
    assert_eq!(write.status, OperationStatus::Ok, "{:?}", write.diagnostics);
    let seeded = write.runtime_snapshot.expect("snapshot");
    assert_eq!(
        seeded.dirty_metadata[DRIVER_SCOPE_ROOT][DRIVER_PATH].intent,
        OverrideIntent::Experimental,
        "the persisted intent is Experimental when omitted (unchanged behavior)"
    );
}

#[test]
fn set_parameter_request_intent_serde_is_optional_back_compat() {
    // The agent (configflux-ccql.3) deserializes a client JSON payload directly
    // into `SetParameterRequest` (it injects only the snapshot). An older client
    // payload carries NO `intent`/`actor`/`reason`; it must round-trip
    // backward-compatibly, defaulting intent to Experimental and actor/reason to
    // None — so existing callers keep working.
    let snapshot = driver_fixture_snapshot();
    let legacy_payload = serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": snapshot,
        "path": DRIVER_PATH,
        "value": 7.5
    });
    let request: SetParameterRequest = serde_json::from_value(legacy_payload)
        .expect("a SetParameter payload without intent must still deserialize");
    assert_eq!(
        request.intent,
        OverrideIntent::Experimental,
        "an omitted intent defaults to Experimental"
    );
    assert_eq!(request.actor, None, "an omitted actor defaults to None");
    assert_eq!(request.reason, None, "an omitted reason defaults to None");

    // And a payload that DOES declare a compensating intent round-trips it.
    let declared_payload = serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": driver_fixture_snapshot(),
        "path": DRIVER_PATH,
        "value": 7.5,
        "intent": "compensating",
        "actor": "tech-42",
        "reason": "loosened until part swap"
    });
    let declared: SetParameterRequest =
        serde_json::from_value(declared_payload).expect("declared-intent payload must deserialize");
    assert_eq!(declared.intent, OverrideIntent::Compensating);
    assert_eq!(declared.actor.as_deref(), Some("tech-42"));
    assert_eq!(declared.reason.as_deref(), Some("loosened until part swap"));
}
