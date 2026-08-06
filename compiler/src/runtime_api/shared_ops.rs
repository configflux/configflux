// SPDX-License-Identifier: BUSL-1.1

#[derive(Debug, Clone)]
struct DirtyPathSelection {
    scope_root: String,
    path: String,
    canonical_path: String,
    generation: u64,
}

struct ResolvedRuntimePath {
    scope_root: String,
    path: String,
    canonical_path: String,
}

fn canonical_runtime_path(scope_root: &str, path: &str) -> String {
    format!("{scope_root}/{path}")
}

fn collect_dirty_path_selections(snapshot: &RuntimeSnapshot) -> Vec<DirtyPathSelection> {
    let mut entries = Vec::new();
    for (scope_root, by_path) in &snapshot.dirty_overlay {
        for path in by_path.keys() {
            let generation = snapshot
                .dirty_generations
                .get(scope_root)
                .and_then(|values| values.get(path))
                .copied()
                .or_else(|| {
                    snapshot
                        .dirty_metadata
                        .get(scope_root)
                        .and_then(|values| values.get(path))
                        .map(|metadata| metadata.generation)
                })
                .unwrap_or(1)
                .max(1);
            entries.push(DirtyPathSelection {
                scope_root: scope_root.clone(),
                path: path.clone(),
                canonical_path: canonical_runtime_path(scope_root, path),
                generation,
            });
        }
    }
    entries.sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
    entries
}

fn resolve_runtime_path(
    snapshot: &RuntimeSnapshot,
    raw_path: &str,
    entity_path: &str,
) -> std::result::Result<ResolvedRuntimePath, Diagnostic> {
    if let Some((scope_root, path)) = raw_path.split_once('/') {
        if scope_root.trim().is_empty() || path.trim().is_empty() {
            return Err(Diagnostic {
                code: E_RUNTIME_UNKNOWN_PATH.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!("Unknown parameter path '{}'", raw_path),
                source_id: None,
                entity_path: Some(entity_path.to_string()),
                hint: Some("Use '<scope_root>/component.<component_id>.param.<param_key>'".to_string()),
            });
        }
        find_parameter_in_scope(snapshot, scope_root, path)?;
        return Ok(ResolvedRuntimePath {
            scope_root: scope_root.to_string(),
            path: path.to_string(),
            canonical_path: canonical_runtime_path(scope_root, path),
        });
    }

    let (component_id, param_key) = parse_parameter_path(raw_path).ok_or_else(|| Diagnostic {
        code: E_RUNTIME_UNKNOWN_PATH.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Unknown parameter path '{}'", raw_path),
        source_id: None,
        entity_path: Some(entity_path.to_string()),
        hint: Some("Use 'component.<component_id>.param.<param_key>'".to_string()),
    })?;
    let scope_root = find_parameter_scope_root(snapshot, component_id, param_key)?;
    Ok(ResolvedRuntimePath {
        canonical_path: canonical_runtime_path(&scope_root, raw_path),
        scope_root,
        path: raw_path.to_string(),
    })
}

fn normalize_scope_root(scope: &str) -> Option<String> {
    if let Some(scope_root) = scope.strip_prefix("component:") {
        let trimmed = scope_root.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        let trimmed = scope.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

fn parse_parameter_path(path: &str) -> Option<(&str, &str)> {
    let mut parts = path.split('.');
    let Some(prefix) = parts.next() else {
        return None;
    };
    let Some(component_id) = parts.next() else {
        return None;
    };
    let Some(param_token) = parts.next() else {
        return None;
    };
    let Some(param_key) = parts.next() else {
        return None;
    };
    if parts.next().is_some()
        || prefix != "component"
        || param_token != "param"
        || component_id.trim().is_empty()
        || param_key.trim().is_empty()
    {
        return None;
    }
    Some((component_id, param_key))
}

fn sorted_parameter_paths(resolved_scope: &crate::resolved_models::ResolvedConfig) -> Vec<String> {
    let mut component_ids: Vec<String> = resolved_scope.components.keys().cloned().collect();
    component_ids.sort();

    let mut paths = Vec::new();
    for component_id in component_ids {
        let Some(component) = resolved_scope.components.get(&component_id) else {
            continue;
        };
        let mut param_keys: Vec<String> = component.params.keys().cloned().collect();
        param_keys.sort();
        for param_key in param_keys {
            paths.push(format!("component.{component_id}.param.{param_key}"));
        }
    }
    paths
}

fn find_parameter_scope_root(
    snapshot: &RuntimeSnapshot,
    component_id: &str,
    param_key: &str,
) -> std::result::Result<String, Diagnostic> {
    let mut matches = Vec::new();
    for (scope_root, resolved_scope) in &snapshot.resolved_output {
        let Some(component) = resolved_scope.components.get(component_id) else {
            continue;
        };
        if component.params.contains_key(param_key) {
            matches.push(scope_root.clone());
        }
    }

    match matches.len() {
        0 => Err(Diagnostic {
            code: E_RUNTIME_UNKNOWN_PATH.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unknown parameter path 'component.{component_id}.param.{param_key}'"
            ),
            source_id: None,
            entity_path: Some("request.path".to_string()),
            hint: Some("Use path format component.<component_id>.param.<param_key>".to_string()),
        }),
        1 => Ok(matches[0].clone()),
        _ => Err(Diagnostic {
            code: E_RUNTIME_OPEN_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Parameter path 'component.{component_id}.param.{param_key}' is ambiguous across multiple scope roots"
            ),
            source_id: None,
            entity_path: Some("request.path".to_string()),
            hint: Some("Use runtime snapshots with unique component IDs per scope root".to_string()),
        }),
    }
}

fn find_parameter_payload(
    snapshot: &RuntimeSnapshot,
    path: &str,
) -> std::result::Result<RuntimeParameterPayload, Diagnostic> {
    let (component_id, param_key) = parse_parameter_path(path).ok_or_else(|| Diagnostic {
        code: E_RUNTIME_UNKNOWN_PATH.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Unknown parameter path '{}'", path),
        source_id: None,
        entity_path: Some("request.path".to_string()),
        hint: Some("Use path format component.<component_id>.param.<param_key>".to_string()),
    })?;

    let scope_root = find_parameter_scope_root(snapshot, component_id, param_key)?;
    let parameter = find_parameter_in_scope(snapshot, &scope_root, path)?;
    parameter_payload(
        component_id,
        param_key,
        &effective_parameter(parameter, snapshot, &scope_root, path),
        &snapshot.resolved_artifacts,
    )
}

fn find_parameter_in_scope<'a>(
    snapshot: &'a RuntimeSnapshot,
    scope_root: &str,
    path: &str,
) -> std::result::Result<&'a crate::resolved_models::ResolvedParameter, Diagnostic> {
    let (component_id, param_key) = parse_parameter_path(path).ok_or_else(|| Diagnostic {
        code: E_RUNTIME_UNKNOWN_PATH.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Unknown parameter path '{}'", path),
        source_id: None,
        entity_path: Some("request.path".to_string()),
        hint: Some("Use path format component.<component_id>.param.<param_key>".to_string()),
    })?;

    let Some(resolved_scope) = snapshot.resolved_output.get(scope_root) else {
        return Err(Diagnostic {
            code: E_RUNTIME_UNKNOWN_SCOPE.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Scope root '{}' is not present in runtime snapshot",
                scope_root
            ),
            source_id: None,
            entity_path: Some("runtime_snapshot.resolved_output".to_string()),
            hint: Some("Open a valid runtime snapshot before reads".to_string()),
        });
    };
    let Some(component) = resolved_scope.components.get(component_id) else {
        return Err(Diagnostic {
            code: E_RUNTIME_UNKNOWN_PATH.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!("Unknown parameter path '{}'", path),
            source_id: None,
            entity_path: Some("request.path".to_string()),
            hint: Some("Use path format component.<component_id>.param.<param_key>".to_string()),
        });
    };
    let Some(parameter) = component.params.get(param_key) else {
        return Err(Diagnostic {
            code: E_RUNTIME_UNKNOWN_PATH.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!("Unknown parameter path '{}'", path),
            source_id: None,
            entity_path: Some("request.path".to_string()),
            hint: Some("Use path format component.<component_id>.param.<param_key>".to_string()),
        });
    };
    Ok(parameter)
}

fn effective_parameter(
    baseline_parameter: &crate::resolved_models::ResolvedParameter,
    snapshot: &RuntimeSnapshot,
    scope_root: &str,
    path: &str,
) -> crate::resolved_models::ResolvedParameter {
    let mut effective = baseline_parameter.clone();
    effective.value = effective_parameter_value(snapshot, scope_root, path, &baseline_parameter.value);
    effective
}

fn effective_parameter_value(
    snapshot: &RuntimeSnapshot,
    scope_root: &str,
    path: &str,
    baseline_value: &crate::schema::Value,
) -> crate::schema::Value {
    if let Some(value) = overlay_value(&snapshot.dirty_overlay, scope_root, path) {
        return value.clone();
    }
    if let Some(value) = overlay_value(&snapshot.committed_overlay, scope_root, path) {
        return value.clone();
    }
    baseline_value.clone()
}

fn overlay_value<'a>(
    overlay: &'a BTreeMap<String, BTreeMap<String, crate::schema::Value>>,
    scope_root: &str,
    path: &str,
) -> Option<&'a crate::schema::Value> {
    overlay.get(scope_root).and_then(|entries| entries.get(path))
}

fn current_time_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_runtime_snapshot_state(
    snapshot: &mut RuntimeSnapshot,
    now_unix_ms: u64,
) -> std::result::Result<(), Diagnostic> {
    normalize_event_bus_state(&mut snapshot.event_bus);
    normalize_sync_status(&mut snapshot.sync_status);
    normalize_audit_state(snapshot);

    let mut normalized_metadata: BTreeMap<String, BTreeMap<String, DirtyEntryMetadata>> =
        BTreeMap::new();
    for (scope_root, entries) in &snapshot.dirty_overlay {
        for path in entries.keys() {
            let generation = snapshot
                .dirty_generations
                .get(scope_root)
                .and_then(|values| values.get(path))
                .copied()
                .or_else(|| {
                    snapshot
                        .dirty_metadata
                        .get(scope_root)
                        .and_then(|values| values.get(path))
                        .map(|metadata| metadata.generation)
                })
                .unwrap_or(0)
                .max(1);

            snapshot
                .dirty_generations
                .entry(scope_root.clone())
                .or_default()
                .insert(path.clone(), generation);

            let mut metadata = snapshot
                .dirty_metadata
                .get(scope_root)
                .and_then(|values| values.get(path))
                .cloned()
                .unwrap_or_else(|| DirtyEntryMetadata {
                    actor: DEFAULT_DIRTY_ACTOR.to_string(),
                    reason: None,
                    dirty_since_unix_ms: now_unix_ms,
                    reset_deadline_unix_ms: None,
                    // A synthesized metadata entry carries no hard cap; the
                    // session-lease deadline is populated from policy below
                    // (configflux-h6wc).
                    hard_cap_deadline_unix_ms: None,
                    generation,
                    intent: OverrideIntent::default(),
                });

            if metadata.actor.trim().is_empty() {
                metadata.actor = DEFAULT_DIRTY_ACTOR.to_string();
            }
            metadata.reason = sanitize_optional_text(metadata.reason);
            metadata.generation = generation;
            if metadata.dirty_since_unix_ms == 0 {
                metadata.dirty_since_unix_ms = now_unix_ms;
            }
            if metadata.reset_deadline_unix_ms.is_none() {
                let policy = resolve_auto_reset_for_path(&snapshot.auto_reset_policy, scope_root, path);
                if policy.enabled {
                    metadata.reset_deadline_unix_ms =
                        Some(now_unix_ms.saturating_add(policy.timeout_ms));
                }
            }

            normalized_metadata
                .entry(scope_root.clone())
                .or_default()
                .insert(path.clone(), metadata);
        }
    }
    snapshot.dirty_metadata = normalized_metadata;

    snapshot.auto_reset_scheduler.pending.retain(|entry| {
        snapshot
            .dirty_overlay
            .get(&entry.scope_root)
            .and_then(|entries| entries.get(&entry.path))
            .is_some()
    });
    for (scope_root, entries) in &snapshot.dirty_metadata {
        for (path, metadata) in entries {
            // Rebuild a schedule entry PER persisted bound (ADR-0037 Decision 3,
            // configflux-h6wc): the session-lease deadline and the hard-cap
            // deadline each get their own kind-tagged entry, so the active driver
            // fires both terminal events on their own schedules and classifies
            // each from the entry's `kind` alone. A pre-dual-bound snapshot has
            // `hard_cap_deadline_unix_ms == None`, so only the lease entry is
            // pushed — exactly the prior single-deadline behavior.
            if let Some(deadline_unix_ms) = metadata.reset_deadline_unix_ms {
                snapshot.auto_reset_scheduler.pending.push(AutoResetScheduleEntry {
                    scope_root: scope_root.clone(),
                    path: path.clone(),
                    generation: metadata.generation,
                    deadline_unix_ms,
                    kind: AutoResetDeadlineKind::LeaseExpiry,
                });
            }
            if let Some(deadline_unix_ms) = metadata.hard_cap_deadline_unix_ms {
                snapshot.auto_reset_scheduler.pending.push(AutoResetScheduleEntry {
                    scope_root: scope_root.clone(),
                    path: path.clone(),
                    generation: metadata.generation,
                    deadline_unix_ms,
                    kind: AutoResetDeadlineKind::HardCap,
                });
            }
        }
    }
    sort_and_dedup_schedule_entries(&mut snapshot.auto_reset_scheduler.pending);
    Ok(())
}

fn normalize_event_bus_state(event_bus: &mut RuntimeEventBusState) {
    if event_bus.buffer_capacity == 0 {
        event_bus.buffer_capacity = default_event_buffer_capacity();
    }
    if event_bus.next_sequence == 0 {
        event_bus.next_sequence = default_event_next_sequence();
    }

    let mut events: Vec<RuntimeEvent> = event_bus.events.drain(..).collect();
    events.sort_by_key(|event| event.sequence);
    events.dedup_by(|left, right| left.sequence == right.sequence);

    let capacity = event_bus.buffer_capacity.max(1);
    if events.len() > capacity {
        let overflow = events.len() - capacity;
        events.drain(0..overflow);
        event_bus.dropped_events = event_bus.dropped_events.saturating_add(overflow as u64);
    }

    if let Some(max_sequence) = events.last().map(|event| event.sequence) {
        if event_bus.next_sequence <= max_sequence {
            event_bus.next_sequence = max_sequence.saturating_add(1);
        }
    }
    event_bus.events = VecDeque::from(events);
}

fn normalize_sync_status(sync_status: &mut RuntimeSyncStatus) {
    sync_status.pending_update_summary =
        sanitize_optional_text(sync_status.pending_update_summary.clone());
}

fn normalize_audit_state(snapshot: &mut RuntimeSnapshot) {
    if snapshot.audit_next_sequence == 0 {
        snapshot.audit_next_sequence = default_audit_next_sequence();
    }

    snapshot.audit_events.sort_by_key(|event| event.sequence);
    snapshot
        .audit_events
        .dedup_by(|left, right| left.sequence == right.sequence);

    for event in &mut snapshot.audit_events {
        if event.sequence == 0 {
            event.sequence = snapshot.audit_next_sequence;
            snapshot.audit_next_sequence = snapshot.audit_next_sequence.saturating_add(1);
        }
        if event.event_id.trim().is_empty() {
            event.event_id = format!("audit-{:016x}", event.sequence);
        }
        if event.actor.trim().is_empty() {
            event.actor = DEFAULT_SYSTEM_ACTOR.to_string();
        }
        event.reason = sanitize_optional_text(event.reason.clone());
        event.base_configuration_id = sanitize_optional_text(event.base_configuration_id.clone());
        event.target_configuration_id = sanitize_optional_text(event.target_configuration_id.clone());
        event.changed_paths.sort();
        event.changed_paths.dedup();
    }

    if let Some(max_sequence) = snapshot.audit_events.last().map(|event| event.sequence) {
        if snapshot.audit_next_sequence <= max_sequence {
            snapshot.audit_next_sequence = max_sequence.saturating_add(1);
        }
        if snapshot.audit_uploaded_sequence > max_sequence {
            snapshot.audit_uploaded_sequence = max_sequence;
        }
    } else {
        snapshot.audit_uploaded_sequence = 0;
    }
}

fn sort_and_dedup_schedule_entries(entries: &mut Vec<AutoResetScheduleEntry>) {
    // `kind` participates in the ordering and the dedup key (configflux-h6wc):
    // a session-lease and a hard-cap entry for the same path/generation that
    // happen to share a deadline are DISTINCT terminal events and must both
    // survive. Ordering by deadline first preserves the due-prefix scan the
    // driver and the lazy path rely on.
    entries.sort_by(|left, right| {
        (
            left.deadline_unix_ms,
            &left.scope_root,
            &left.path,
            left.generation,
            left.kind,
        )
            .cmp(&(
                right.deadline_unix_ms,
                &right.scope_root,
                &right.path,
                right.generation,
                right.kind,
            ))
    });
    entries.dedup_by(|left, right| {
        left.deadline_unix_ms == right.deadline_unix_ms
            && left.scope_root == right.scope_root
            && left.path == right.path
            && left.generation == right.generation
            && left.kind == right.kind
    });
}

fn apply_due_auto_resets(
    snapshot: &mut RuntimeSnapshot,
    now_unix_ms: u64,
) -> std::result::Result<(), Diagnostic> {
    if snapshot.auto_reset_scheduler.pending.is_empty() {
        return Ok(());
    }

    sort_and_dedup_schedule_entries(&mut snapshot.auto_reset_scheduler.pending);
    let due_count = snapshot
        .auto_reset_scheduler
        .pending
        .iter()
        .take_while(|entry| entry.deadline_unix_ms <= now_unix_ms)
        .count();
    if due_count == 0 {
        return Ok(());
    }

    let due_entries: Vec<AutoResetScheduleEntry> = snapshot
        .auto_reset_scheduler
        .pending
        .drain(0..due_count)
        .collect();

    for entry in due_entries {
        revert_due_entry(snapshot, &entry, now_unix_ms, "timeout")?;
    }

    Ok(())
}

/// Validate a due schedule entry against the current dirty state, returning the
/// entry's metadata + the overlay value being reverted if (and only if) the
/// entry is still live and current. Returns `Ok(None)` for a stale/superseded
/// entry (a newer write replaced it, or the deadline no longer matches), in
/// which case the caller skips it. Shared by the lazy auto-reset path
/// (`apply_due_auto_resets`) and the active cap/lease driver
/// (`drive_override_lifecycle`, configflux-ccql.4) so neither reimplements the
/// generation/deadline staleness guards.
fn validate_due_schedule_entry(
    snapshot: &RuntimeSnapshot,
    entry: &AutoResetScheduleEntry,
) -> Option<(DirtyEntryMetadata, crate::schema::Value)> {
    let current_generation = snapshot
        .dirty_generations
        .get(&entry.scope_root)
        .and_then(|values| values.get(&entry.path))
        .copied()?;
    if current_generation != entry.generation {
        return None;
    }

    let metadata = snapshot
        .dirty_metadata
        .get(&entry.scope_root)
        .and_then(|values| values.get(&entry.path))
        .cloned()?;
    // Match the entry's deadline against the persisted bound for ITS kind
    // (configflux-h6wc): a lease entry must still match the session-lease
    // deadline, a hard-cap entry the hard-cap deadline. A bound that was cleared
    // or moved (e.g. an escalation disarmed the lease) makes the entry stale.
    let persisted_deadline = match entry.kind {
        AutoResetDeadlineKind::LeaseExpiry => metadata.reset_deadline_unix_ms,
        AutoResetDeadlineKind::HardCap => metadata.hard_cap_deadline_unix_ms,
    };
    if metadata.generation != entry.generation || persisted_deadline != Some(entry.deadline_unix_ms)
    {
        return None;
    }

    let old_value =
        overlay_value(&snapshot.dirty_overlay, &entry.scope_root, &entry.path).cloned()?;
    Some((metadata, old_value))
}

/// Silently revert a single due override entry: clear the dirty overlay back to
/// the committed-or-baseline value and emit the `ResetApplied` /
/// `DirtyStateChanged` runtime events plus the durable `Reset` audit event. This
/// is the per-entry body that was previously inlined in `apply_due_auto_resets`,
/// extracted verbatim so the active driver (configflux-ccql.4) reuses the exact
/// same reset mechanics rather than reimplementing them. A stale/superseded
/// entry is a no-op (`Ok(())`). `cause` labels the `ResetApplied` event payload
/// ("timeout" for the lazy path; the active driver passes its own cause).
fn revert_due_entry(
    snapshot: &mut RuntimeSnapshot,
    entry: &AutoResetScheduleEntry,
    now_unix_ms: u64,
    cause: &str,
) -> std::result::Result<(), Diagnostic> {
    let Some((metadata, old_value)) = validate_due_schedule_entry(snapshot, entry) else {
        return Ok(());
    };

    let baseline_parameter =
        find_parameter_in_scope(snapshot, &entry.scope_root, &entry.path)?.clone();

    clear_dirty_entry(snapshot, &entry.scope_root, &entry.path, entry.generation);
    let committed_or_baseline = effective_parameter_value(
        snapshot,
        &entry.scope_root,
        &entry.path,
        &baseline_parameter.value,
    );

    snapshot.persistence_journal_sequence = snapshot.persistence_journal_sequence.saturating_add(1);
    emit_runtime_event(
        snapshot,
        RuntimeEventKind::ResetApplied,
        now_unix_ms,
        Some(&metadata.actor),
        metadata.reason.as_deref(),
        Some(&old_value),
        Some(&committed_or_baseline),
        RuntimeEventPayload::ResetApplied {
            scope_root: entry.scope_root.clone(),
            path: entry.path.clone(),
            cause: cause.to_string(),
            generation: entry.generation,
        },
    );
    emit_runtime_event(
        snapshot,
        RuntimeEventKind::DirtyStateChanged,
        now_unix_ms,
        Some(&metadata.actor),
        metadata.reason.as_deref(),
        None,
        None,
        RuntimeEventPayload::DirtyStateChanged {
            scope_root: entry.scope_root.clone(),
            path: entry.path.clone(),
            dirty: false,
            generation: entry.generation,
        },
    );
    let identity = compute_configuration_identity(snapshot)?;
    append_audit_event(
        snapshot,
        RuntimeAuditEventKind::Reset,
        now_unix_ms,
        &metadata.actor,
        metadata.reason.as_deref(),
        vec![canonical_runtime_path(&entry.scope_root, &entry.path)],
        &identity,
        None,
        None,
    );
    Ok(())
}

fn clear_dirty_entry(snapshot: &mut RuntimeSnapshot, scope_root: &str, path: &str, generation: u64) {
    let remove_dirty_overlay_scope = if let Some(entries) = snapshot.dirty_overlay.get_mut(scope_root) {
        entries.remove(path);
        entries.is_empty()
    } else {
        false
    };
    if remove_dirty_overlay_scope {
        snapshot.dirty_overlay.remove(scope_root);
    }

    let remove_dirty_generation_scope =
        if let Some(entries) = snapshot.dirty_generations.get_mut(scope_root) {
            entries.remove(path);
            entries.is_empty()
        } else {
            false
        };
    if remove_dirty_generation_scope {
        snapshot.dirty_generations.remove(scope_root);
    }

    let remove_dirty_metadata_scope = if let Some(entries) = snapshot.dirty_metadata.get_mut(scope_root) {
        entries.remove(path);
        entries.is_empty()
    } else {
        false
    };
    if remove_dirty_metadata_scope {
        snapshot.dirty_metadata.remove(scope_root);
    }

    snapshot.auto_reset_scheduler.pending.retain(|entry| {
        !(entry.scope_root == scope_root && entry.path == path && entry.generation <= generation)
    });
}

#[derive(Debug, Clone, Copy)]
struct ResolvedAutoResetPolicy {
    enabled: bool,
    timeout_ms: u64,
}

fn resolve_auto_reset_for_path(
    policy: &AutoResetPolicy,
    scope_root: &str,
    path: &str,
) -> ResolvedAutoResetPolicy {
    let scoped_key = format!("{scope_root}/{path}");
    let override_policy = policy
        .per_path_overrides
        .get(&scoped_key)
        .or_else(|| policy.per_path_overrides.get(path));

    let enabled = override_policy
        .and_then(|entry| entry.enabled)
        .unwrap_or(policy.enabled);
    let timeout_ms = override_policy
        .and_then(|entry| entry.timeout_ms)
        .unwrap_or(policy.default_timeout_ms);

    ResolvedAutoResetPolicy {
        enabled,
        timeout_ms,
    }
}

fn apply_dirty_write(
    snapshot: &mut RuntimeSnapshot,
    scope_root: &str,
    path: &str,
    value: crate::schema::Value,
    now_unix_ms: u64,
    actor: &str,
    reason: Option<&str>,
    intent: OverrideIntent,
) -> DirtyEntryMetadata {
    snapshot
        .dirty_overlay
        .entry(scope_root.to_string())
        .or_default()
        .insert(path.to_string(), value);

    let next_generation = snapshot
        .dirty_generations
        .get(scope_root)
        .and_then(|generations| generations.get(path))
        .copied()
        .unwrap_or(0)
        .saturating_add(1);
    snapshot
        .dirty_generations
        .entry(scope_root.to_string())
        .or_default()
        .insert(path.to_string(), next_generation);

    let policy = resolve_auto_reset_for_path(&snapshot.auto_reset_policy, scope_root, path);
    let reset_deadline_unix_ms = if policy.enabled {
        Some(now_unix_ms.saturating_add(policy.timeout_ms))
    } else {
        None
    };
    let metadata = DirtyEntryMetadata {
        actor: actor.to_string(),
        reason: sanitize_optional_text(reason.map(ToString::to_string)),
        dirty_since_unix_ms: now_unix_ms,
        reset_deadline_unix_ms,
        // The write path schedules only the session-lease bound; a hard cap is
        // set by operator-supplied `OverrideTimeBounds` threading (a sibling
        // task, configflux-h6wc follow-up). `None` here means no hard-cap entry
        // is scheduled for a plain write.
        hard_cap_deadline_unix_ms: None,
        generation: next_generation,
        // Governance intent supplied by the caller (configflux-irid). The
        // operator declares it at `SetParameter[Atomically]` time and it is
        // persisted here so the active cap/lease driver (configflux-ccql.4) sees
        // a real compensating override WITHOUT a pre-seeded snapshot. Callers that
        // do not declare an intent pass `OverrideIntent::default()` (Experimental)
        // — abandoned-experiment cleanup remains the safe default.
        intent,
    };
    snapshot
        .dirty_metadata
        .entry(scope_root.to_string())
        .or_default()
        .insert(path.to_string(), metadata.clone());

    if let Some(deadline_unix_ms) = metadata.reset_deadline_unix_ms {
        snapshot
            .auto_reset_scheduler
            .pending
            .push(AutoResetScheduleEntry {
                scope_root: scope_root.to_string(),
                path: path.to_string(),
                generation: next_generation,
                deadline_unix_ms,
                kind: AutoResetDeadlineKind::LeaseExpiry,
            });
        sort_and_dedup_schedule_entries(&mut snapshot.auto_reset_scheduler.pending);
    }

    snapshot.persistence_journal_sequence = snapshot.persistence_journal_sequence.saturating_add(1);
    metadata
}

fn sanitize_optional_text(input: Option<String>) -> Option<String> {
    input.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn emit_runtime_event(
    snapshot: &mut RuntimeSnapshot,
    event_kind: RuntimeEventKind,
    timestamp_unix_ms: u64,
    actor: Option<&str>,
    reason: Option<&str>,
    old_value: Option<&crate::schema::Value>,
    new_value: Option<&crate::schema::Value>,
    payload: RuntimeEventPayload,
) -> String {
    if snapshot.event_bus.buffer_capacity == 0 {
        snapshot.event_bus.buffer_capacity = default_event_buffer_capacity();
    }
    if snapshot.event_bus.next_sequence == 0 {
        snapshot.event_bus.next_sequence = default_event_next_sequence();
    }

    let sequence = snapshot.event_bus.next_sequence;
    snapshot.event_bus.next_sequence = snapshot.event_bus.next_sequence.saturating_add(1);
    let event = RuntimeEvent {
        event_id: format!("evt-{sequence:016x}"),
        sequence,
        event_kind,
        scope: snapshot.scope.clone(),
        timestamp_unix_ms,
        actor: actor
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        reason: reason
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        old_value_hash: old_value.and_then(value_hash),
        new_value_hash: new_value.and_then(value_hash),
        payload,
    };

    let capacity = snapshot.event_bus.buffer_capacity.max(1);
    while snapshot.event_bus.events.len() >= capacity {
        snapshot.event_bus.events.pop_front();
        snapshot.event_bus.dropped_events = snapshot.event_bus.dropped_events.saturating_add(1);
    }
    let event_id = event.event_id.clone();
    snapshot.event_bus.events.push_back(event);
    event_id
}

fn value_hash(value: &crate::schema::Value) -> Option<String> {
    serde_json::to_vec(value).ok().map(|bytes| sha256_hex(&bytes))
}

fn append_audit_event(
    snapshot: &mut RuntimeSnapshot,
    event_kind: RuntimeAuditEventKind,
    timestamp_unix_ms: u64,
    actor: &str,
    reason: Option<&str>,
    mut changed_paths: Vec<String>,
    identity: &RuntimeConfigurationIdentity,
    base_configuration_id: Option<String>,
    target_configuration_id: Option<String>,
) -> String {
    let sequence = snapshot.audit_next_sequence.max(1);
    snapshot.audit_next_sequence = sequence.saturating_add(1);

    changed_paths.sort();
    changed_paths.dedup();
    let event_id = format!("audit-{sequence:016x}");
    let event = RuntimeAuditEvent {
        event_id: event_id.clone(),
        sequence,
        event_kind,
        scope: snapshot.scope.clone(),
        timestamp_unix_ms,
        actor: {
            let trimmed = actor.trim();
            if trimmed.is_empty() {
                DEFAULT_SYSTEM_ACTOR.to_string()
            } else {
                trimmed.to_string()
            }
        },
        reason: sanitize_optional_text(reason.map(ToString::to_string)),
        committed_configuration_id: identity.committed_configuration_id.clone(),
        working_configuration_id: identity.working_configuration_id.clone(),
        base_configuration_id: sanitize_optional_text(base_configuration_id),
        target_configuration_id: sanitize_optional_text(target_configuration_id),
        changed_paths,
    };
    snapshot.audit_events.push(event);
    event_id
}

/// Canonical, hashable view of a [`ProvenanceLineageEntry`]'s contents.
///
/// Deliberately excludes `entry_id` (the content address can't depend on
/// itself) and pins a fixed field order plus `schema_version`, mirroring the
/// `IdentityLeafCanonical` / `SelectionStateCanonical` convention so the
/// resulting sha256-hex is deterministic and byte-stable. The optionals are
/// carried through unchanged so absence (`None`) and presence are unambiguous
/// in the hashed payload.
#[derive(Debug, Clone, Serialize)]
struct ProvenanceLineageEntryCanonical<'a> {
    schema_version: u32,
    state: &'a ProvenanceVersionTriple,
    actor: &'a str,
    reason: Option<&'a str>,
    timestamp_unix_ms: u64,
    // The override-layer governance intent (configflux-ts7z). Included in the
    // content address so it is part of the entry's identity, not cosmetic
    // metadata bolted on afterwards — flipping experimental<->compensating yields
    // a different `entry_id`, the same way re-parenting does. Adding it changes
    // the content address of NEW entries (accepted per the ADR-0038 amendment:
    // the canonical form is versioned by `schema_version`, not frozen; already-
    // stored entries keep their existing addresses).
    intent: OverrideIntent,
    parent_entry_id: Option<&'a str>,
}

impl ProvenanceLineageEntry {
    /// Construct a lineage entry, computing its content address (`entry_id`)
    /// from the supplied contents. The parent-pointer (`parent_entry_id`)
    /// references the prior entry's content address, or `None` at the chain
    /// root.
    ///
    /// Part of the public lineage contract surface: report producers and
    /// verifiers (configflux-ccql.5 / configflux-ccql.6) build entries through
    /// this constructor so the content address is computed one way.
    pub fn new(
        state: ProvenanceVersionTriple,
        actor: String,
        reason: Option<String>,
        timestamp_unix_ms: u64,
        intent: OverrideIntent,
        parent_entry_id: Option<String>,
    ) -> std::result::Result<Self, Diagnostic> {
        let entry_id = compute_lineage_entry_content_address(
            &state,
            &actor,
            reason.as_deref(),
            timestamp_unix_ms,
            intent,
            parent_entry_id.as_deref(),
        )?;
        Ok(Self {
            entry_id,
            state,
            actor,
            reason,
            timestamp_unix_ms,
            intent,
            parent_entry_id,
        })
    }
}

/// Compute the content address of a lineage entry: a deterministic sha256-hex
/// over the entry's canonical serialization (excluding `entry_id`). Reuses the
/// `stable_hash` idiom (`serde_json::to_vec` -> `sha256_hex`) so the result is
/// a 64-char hex string consistent with `model_hash` / `resolve_hash` /
/// `committed_configuration_id`.
///
/// Public so downstream verifiers (configflux-ccql.6) can verify a reported
/// entry's content address without reconstructing the entry.
pub fn compute_lineage_entry_content_address(
    state: &ProvenanceVersionTriple,
    actor: &str,
    reason: Option<&str>,
    timestamp_unix_ms: u64,
    intent: OverrideIntent,
    parent_entry_id: Option<&str>,
) -> std::result::Result<String, Diagnostic> {
    let canonical = ProvenanceLineageEntryCanonical {
        schema_version: PRODUCT_SCHEMA_VERSION,
        state,
        actor,
        reason,
        timestamp_unix_ms,
        intent,
        parent_entry_id,
    };
    stable_hash(&canonical).map_err(|error| Diagnostic {
        code: E_RUNTIME_OPEN_INVALID.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Failed to canonicalize provenance lineage entry payload: {error}"),
        source_id: None,
        entity_path: Some("provenance_lineage_entry".to_string()),
        hint: Some("Use canonical provenance lineage entry contents".to_string()),
    })
}

#[derive(Debug, Clone, Serialize)]
struct IdentityLeafCanonical {
    scope_root: String,
    path: String,
    r#type: String,
    unit: Option<String>,
    lifecycle: crate::schema::Lifecycle,
    value: crate::schema::Value,
}

fn compute_configuration_identity(
    snapshot: &RuntimeSnapshot,
) -> std::result::Result<RuntimeConfigurationIdentity, Diagnostic> {
    let mut scope_roots: Vec<&String> = snapshot.resolved_output.keys().collect();
    scope_roots.sort();

    let mut committed_leaves = Vec::new();
    let mut working_leaves = Vec::new();
    let mut committed_diff: BTreeMap<String, crate::schema::Value> = BTreeMap::new();
    let mut dirty_diff: BTreeMap<String, crate::schema::Value> = BTreeMap::new();

    for scope_root in scope_roots {
        let Some(resolved_scope) = snapshot.resolved_output.get(scope_root) else {
            continue;
        };
        for path in sorted_parameter_paths(resolved_scope) {
            let parameter = find_parameter_in_scope(snapshot, scope_root, &path)?;
            let committed_value =
                committed_parameter_value(snapshot, scope_root, &path, &parameter.value);
            let working_value = effective_parameter_value(snapshot, scope_root, &path, &parameter.value);
            let canonical_path = format!("{scope_root}/{path}");

            let committed_leaf_hash =
                identity_leaf_hash(scope_root, &path, parameter, committed_value.clone())?;
            let working_leaf_hash =
                identity_leaf_hash(scope_root, &path, parameter, working_value.clone())?;

            committed_leaves.push((canonical_path.clone(), committed_leaf_hash));
            working_leaves.push((canonical_path.clone(), working_leaf_hash));

            if committed_value != parameter.value {
                committed_diff.insert(canonical_path.clone(), committed_value);
            }
            if working_value != committed_parameter_value(snapshot, scope_root, &path, &parameter.value) {
                dirty_diff.insert(canonical_path, working_value);
            }
        }
    }

    let committed_configuration_id = root_hash_from_leaves(committed_leaves);
    let working_configuration_id = root_hash_from_leaves(working_leaves);
    let diff_hash = stable_hash(&committed_diff).map_err(|error| Diagnostic {
        code: E_RUNTIME_OPEN_INVALID.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Failed to canonicalize committed diff hash payload: {error}"),
        source_id: None,
        entity_path: Some("runtime_snapshot.committed_overlay".to_string()),
        hint: Some("Use canonical runtime overlay payloads".to_string()),
    })?;
    let dirty_diff_hash = stable_hash(&dirty_diff).map_err(|error| Diagnostic {
        code: E_RUNTIME_OPEN_INVALID.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Failed to canonicalize dirty diff hash payload: {error}"),
        source_id: None,
        entity_path: Some("runtime_snapshot.dirty_overlay".to_string()),
        hint: Some("Use canonical runtime overlay payloads".to_string()),
    })?;

    Ok(RuntimeConfigurationIdentity {
        committed_configuration_id,
        working_configuration_id,
        diff_hash,
        dirty_diff_hash,
    })
}

fn committed_parameter_value(
    snapshot: &RuntimeSnapshot,
    scope_root: &str,
    path: &str,
    baseline_value: &crate::schema::Value,
) -> crate::schema::Value {
    if let Some(value) = overlay_value(&snapshot.committed_overlay, scope_root, path) {
        return value.clone();
    }
    baseline_value.clone()
}

fn identity_leaf_hash(
    scope_root: &str,
    path: &str,
    parameter: &crate::resolved_models::ResolvedParameter,
    value: crate::schema::Value,
) -> std::result::Result<String, Diagnostic> {
    let canonical = IdentityLeafCanonical {
        scope_root: scope_root.to_string(),
        path: path.to_string(),
        r#type: parameter.r#type.clone(),
        unit: parameter.unit.clone(),
        lifecycle: parameter.lifecycle.clone(),
        value,
    };
    stable_hash(&canonical).map_err(|error| Diagnostic {
        code: E_RUNTIME_OPEN_INVALID.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Failed to canonicalize identity leaf hash payload: {error}"),
        source_id: None,
        entity_path: Some(path.to_string()),
        hint: Some("Use canonical runtime parameter payloads".to_string()),
    })
}

fn root_hash_from_leaves(mut leaves: Vec<(String, String)>) -> String {
    leaves.sort_by(|left, right| left.0.cmp(&right.0));
    let canonical = leaves
        .into_iter()
        .map(|(path, hash)| format!("{path}:{hash}"))
        .collect::<Vec<_>>()
        .join("\n");
    sha256_hex(canonical.as_bytes())
}

fn stable_hash<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).context("Failed to serialize canonical hash payload")?;
    Ok(sha256_hex(&bytes))
}

fn parameter_payload(
    component_id: &str,
    param_key: &str,
    parameter: &crate::resolved_models::ResolvedParameter,
    artifacts: &BTreeMap<String, crate::schema::Artifact>,
) -> std::result::Result<RuntimeParameterPayload, Diagnostic> {
    let path = format!("component.{component_id}.param.{param_key}");
    let artifact = if parameter.r#type == "artifact" {
        let artifact_id = match &parameter.value {
            crate::schema::Value::String(value) if !value.trim().is_empty() => value.trim(),
            _ => {
                return Err(Diagnostic {
                    code: E_RUNTIME_ARTIFACT_UNKNOWN.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Artifact parameter '{}' does not contain a non-empty artifact ID string",
                        path
                    ),
                    source_id: None,
                    entity_path: Some(path.clone()),
                    hint: Some(
                        "Use artifact parameter values as valid string artifact IDs".to_string(),
                    ),
                });
            }
        };
        let Some(metadata) = artifacts.get(artifact_id) else {
            return Err(Diagnostic {
                code: E_RUNTIME_ARTIFACT_UNKNOWN.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Artifact '{}' referenced by '{}' is not present in runtime snapshot",
                    artifact_id, path
                ),
                source_id: None,
                entity_path: Some(path.clone()),
                hint: Some(
                    "Use artifact IDs available in runtime_snapshot.resolved_artifacts".to_string(),
                ),
            });
        };
        Some(RuntimeArtifactBinding {
            artifact_id: artifact_id.to_string(),
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            hash: metadata.hash.clone(),
            source: metadata.source.clone(),
            target: metadata.target.clone(),
            doc: metadata.doc.clone(),
        })
    } else {
        None
    };

    Ok(RuntimeParameterPayload {
        path,
        component_id: component_id.to_string(),
        param_key: param_key.to_string(),
        r#type: parameter.r#type.clone(),
        value: parameter.value.clone(),
        unit: parameter.unit.clone(),
        safety: parameter.safety.clone(),
        lifecycle: parameter.lifecycle.clone(),
        access: parameter.access.clone(),
        req_id: parameter.req_id.clone(),
        doc: parameter.doc.clone(),
        limits: parameter.limits.clone(),
        artifact,
    })
}

fn is_value_compatible_with_type(param_type: &str, value: &crate::schema::Value) -> bool {
    match param_type {
        "int" | "integer" => matches!(value, crate::schema::Value::Integer(_)),
        "float" => matches!(
            value,
            crate::schema::Value::Integer(_) | crate::schema::Value::Float(_)
        ),
        "bool" | "boolean" => matches!(value, crate::schema::Value::Boolean(_)),
        "string" => matches!(value, crate::schema::Value::String(_)),
        "artifact" => {
            matches!(value, crate::schema::Value::String(text) if !text.trim().is_empty())
        }
        _ => true,
    }
}

fn value_kind(value: &crate::schema::Value) -> &'static str {
    match value {
        crate::schema::Value::Integer(_) => "integer",
        crate::schema::Value::Float(_) => "float",
        crate::schema::Value::Boolean(_) => "boolean",
        crate::schema::Value::String(_) => "string",
    }
}

fn validate_limits(
    path: &str,
    value: &crate::schema::Value,
    limits: Option<&crate::schema::Limits>,
) -> std::result::Result<(), Diagnostic> {
    let Some(limits) = limits else {
        return Ok(());
    };

    if let Some(min_len) = limits.min_len {
        if let crate::schema::Value::String(text) = value {
            if text.len() < min_len {
                return Err(Diagnostic {
                    code: E_RUNTIME_LIMIT_VIOLATION.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!("Value for '{}' violates min_len {}", path, min_len),
                    source_id: None,
                    entity_path: Some(path.to_string()),
                    hint: Some(
                        "Write a string value that satisfies min_len/max_len limits".to_string(),
                    ),
                });
            }
        }
    }

    if let Some(max_len) = limits.max_len {
        if let crate::schema::Value::String(text) = value {
            if text.len() > max_len {
                return Err(Diagnostic {
                    code: E_RUNTIME_LIMIT_VIOLATION.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!("Value for '{}' violates max_len {}", path, max_len),
                    source_id: None,
                    entity_path: Some(path.to_string()),
                    hint: Some(
                        "Write a string value that satisfies min_len/max_len limits".to_string(),
                    ),
                });
            }
        }
    }

    let Some(value_num) = numeric_value(value) else {
        return Ok(());
    };

    if let Some(min) = limits.min.as_ref().and_then(numeric_value) {
        if value_num < min {
            return Err(Diagnostic {
                code: E_RUNTIME_LIMIT_VIOLATION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!("Value for '{}' is below min {}", path, min),
                source_id: None,
                entity_path: Some(path.to_string()),
                hint: Some("Write a value within the configured numeric limits".to_string()),
            });
        }
    }
    if let Some(max) = limits.max.as_ref().and_then(numeric_value) {
        if value_num > max {
            return Err(Diagnostic {
                code: E_RUNTIME_LIMIT_VIOLATION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!("Value for '{}' is above max {}", path, max),
                source_id: None,
                entity_path: Some(path.to_string()),
                hint: Some("Write a value within the configured numeric limits".to_string()),
            });
        }
    }

    Ok(())
}

fn numeric_value(value: &crate::schema::Value) -> Option<f64> {
    match value {
        crate::schema::Value::Integer(value) => Some(*value as f64),
        crate::schema::Value::Float(value) => Some(*value),
        _ => None,
    }
}

fn validate_runtime_snapshot(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    if snapshot.schema_version != PRODUCT_SCHEMA_VERSION {
        return Err(schema_version_diagnostic(
            E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
            snapshot.schema_version,
            "runtime_snapshot.schema_version",
            "Use runtime_snapshot emitted by runtime_open",
        ));
    }
    if !is_sha256_hex(&snapshot.model_hash) {
        return Err(Diagnostic {
            code: E_RUNTIME_OPEN_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.model_hash must be a 64-char sha256 hex string".to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.model_hash".to_string()),
            hint: Some("Use runtime_snapshot emitted by runtime_open".to_string()),
        });
    }
    if !is_sha256_hex(&snapshot.resolve_hash) {
        return Err(Diagnostic {
            code: E_RUNTIME_OPEN_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.resolve_hash must be a 64-char sha256 hex string"
                .to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.resolve_hash".to_string()),
            hint: Some("Use runtime_snapshot emitted by runtime_open".to_string()),
        });
    }
    if snapshot.persistence_format_version == 0 {
        return Err(Diagnostic {
            code: E_RUNTIME_OPEN_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.persistence_format_version must be >= 1".to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.persistence_format_version".to_string()),
            hint: Some("Use runtime_snapshot emitted by runtime_open".to_string()),
        });
    }
    validate_overlay_values(snapshot, &snapshot.committed_overlay, "runtime_snapshot.committed_overlay")?;
    validate_overlay_values(snapshot, &snapshot.dirty_overlay, "runtime_snapshot.dirty_overlay")?;
    validate_generation_overlay(snapshot)?;
    validate_dirty_metadata_overlay(snapshot)?;
    validate_auto_reset_policy(snapshot)?;
    validate_auto_reset_scheduler(snapshot)?;
    validate_event_bus(snapshot)?;
    validate_sync_status(snapshot)?;
    validate_audit_log(snapshot)?;
    Ok(())
}

fn validate_overlay_values(
    snapshot: &RuntimeSnapshot,
    overlay: &BTreeMap<String, BTreeMap<String, crate::schema::Value>>,
    entity_root: &str,
) -> std::result::Result<(), Diagnostic> {
    for (scope_root, path_values) in overlay {
        if !snapshot.resolved_output.contains_key(scope_root) {
            return Err(Diagnostic {
                code: E_RUNTIME_UNKNOWN_SCOPE.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Overlay references unknown scope root '{}'",
                    scope_root
                ),
                source_id: None,
                entity_path: Some(format!("{entity_root}.{scope_root}")),
                hint: Some("Use scope roots available in runtime_snapshot.resolved_output".to_string()),
            });
        }

        for (path, value) in path_values {
            let parameter = find_parameter_in_scope(snapshot, scope_root, path)?;
            if !is_value_compatible_with_type(&parameter.r#type, value) {
                return Err(Diagnostic {
                    code: E_RUNTIME_TYPE_MISMATCH.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Overlay value for '{}' is not compatible with type '{}'",
                        path, parameter.r#type
                    ),
                    source_id: None,
                    entity_path: Some(format!("{entity_root}.{scope_root}.{path}")),
                    hint: Some("Use overlay values that match resolved parameter types".to_string()),
                });
            }
            validate_limits(path, value, parameter.limits.as_ref())?;

            if parameter.r#type == "artifact" {
                let artifact_id = match value {
                    crate::schema::Value::String(id) if !id.trim().is_empty() => id.trim(),
                    _ => {
                        return Err(Diagnostic {
                            code: E_RUNTIME_TYPE_MISMATCH.to_string(),
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "Artifact overlay for '{}' must be a non-empty string artifact ID",
                                path
                            ),
                            source_id: None,
                            entity_path: Some(format!("{entity_root}.{scope_root}.{path}")),
                            hint: Some("Use artifact IDs available in resolved_artifacts".to_string()),
                        });
                    }
                };
                if !snapshot.resolved_artifacts.contains_key(artifact_id) {
                    return Err(Diagnostic {
                        code: E_RUNTIME_ARTIFACT_UNKNOWN.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Overlay artifact '{}' for '{}' is not present in runtime_snapshot.resolved_artifacts",
                            artifact_id, path
                        ),
                        source_id: None,
                        entity_path: Some(format!("{entity_root}.{scope_root}.{path}")),
                        hint: Some("Use artifact IDs available in runtime_snapshot.resolved_artifacts".to_string()),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_generation_overlay(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    for (scope_root, generation_map) in &snapshot.dirty_generations {
        if !snapshot.resolved_output.contains_key(scope_root) {
            return Err(Diagnostic {
                code: E_RUNTIME_UNKNOWN_SCOPE.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "dirty_generations references unknown scope root '{}'",
                    scope_root
                ),
                source_id: None,
                entity_path: Some(format!("runtime_snapshot.dirty_generations.{scope_root}")),
                hint: Some("Use scope roots available in runtime_snapshot.resolved_output".to_string()),
            });
        }

        for path in generation_map.keys() {
            find_parameter_in_scope(snapshot, scope_root, path)?;
        }
    }
    Ok(())
}

fn validate_dirty_metadata_overlay(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    for (scope_root, metadata_map) in &snapshot.dirty_metadata {
        if !snapshot.resolved_output.contains_key(scope_root) {
            return Err(Diagnostic {
                code: E_RUNTIME_DIRTY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "dirty_metadata references unknown scope root '{}'",
                    scope_root
                ),
                source_id: None,
                entity_path: Some(format!("runtime_snapshot.dirty_metadata.{scope_root}")),
                hint: Some("Use scope roots available in runtime_snapshot.resolved_output".to_string()),
            });
        }

        for (path, metadata) in metadata_map {
            find_parameter_in_scope(snapshot, scope_root, path)?;
            if metadata.actor.trim().is_empty() {
                return Err(Diagnostic {
                    code: E_RUNTIME_DIRTY_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "dirty_metadata actor for '{}' in scope '{}' must be non-empty",
                        path, scope_root
                    ),
                    source_id: None,
                    entity_path: Some(format!("runtime_snapshot.dirty_metadata.{scope_root}.{path}")),
                    hint: Some("Provide actor identity for dirty metadata entries".to_string()),
                });
            }
            if metadata.generation == 0 {
                return Err(Diagnostic {
                    code: E_RUNTIME_DIRTY_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "dirty_metadata generation for '{}' in scope '{}' must be >= 1",
                        path, scope_root
                    ),
                    source_id: None,
                    entity_path: Some(format!("runtime_snapshot.dirty_metadata.{scope_root}.{path}")),
                    hint: Some("Use monotonic per-path generation counters".to_string()),
                });
            }

            if let Some(generation) = snapshot
                .dirty_generations
                .get(scope_root)
                .and_then(|map| map.get(path))
            {
                if *generation != metadata.generation {
                    return Err(Diagnostic {
                        code: E_RUNTIME_DIRTY_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "dirty_metadata generation mismatch for '{}' in scope '{}' (metadata={}, dirty_generations={})",
                            path, scope_root, metadata.generation, generation
                        ),
                        source_id: None,
                        entity_path: Some(format!("runtime_snapshot.dirty_metadata.{scope_root}.{path}")),
                        hint: Some("Keep dirty_metadata.generation aligned with dirty_generations".to_string()),
                    });
                }
            }

            if let Some(deadline_unix_ms) = metadata.reset_deadline_unix_ms {
                if deadline_unix_ms < metadata.dirty_since_unix_ms {
                    return Err(Diagnostic {
                        code: E_RUNTIME_DIRTY_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "dirty_metadata reset deadline for '{}' in scope '{}' is earlier than dirty_since timestamp",
                            path, scope_root
                        ),
                        source_id: None,
                        entity_path: Some(format!("runtime_snapshot.dirty_metadata.{scope_root}.{path}")),
                        hint: Some("Use reset deadlines greater than or equal to dirty_since timestamp".to_string()),
                    });
                }
            }
            // The hard-cap deadline is validated symmetrically with the
            // session-lease deadline (configflux-h6wc): both are absolute
            // instants at or after `dirty_since`. A bound earlier than
            // `dirty_since` is malformed.
            if let Some(deadline_unix_ms) = metadata.hard_cap_deadline_unix_ms {
                if deadline_unix_ms < metadata.dirty_since_unix_ms {
                    return Err(Diagnostic {
                        code: E_RUNTIME_DIRTY_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "dirty_metadata hard-cap deadline for '{}' in scope '{}' is earlier than dirty_since timestamp",
                            path, scope_root
                        ),
                        source_id: None,
                        entity_path: Some(format!("runtime_snapshot.dirty_metadata.{scope_root}.{path}")),
                        hint: Some("Use hard-cap deadlines greater than or equal to dirty_since timestamp".to_string()),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_auto_reset_policy(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    for override_key in snapshot.auto_reset_policy.per_path_overrides.keys() {
        if override_key.trim().is_empty() {
            return Err(Diagnostic {
                code: E_RUNTIME_DIRTY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "auto_reset_policy.per_path_overrides contains an empty key".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.auto_reset_policy.per_path_overrides".to_string()),
                hint: Some("Use non-empty parameter path override keys".to_string()),
            });
        }
    }
    Ok(())
}

fn validate_auto_reset_scheduler(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    let mut previous: Option<(u64, String, String, u64)> = None;
    for entry in &snapshot.auto_reset_scheduler.pending {
        if entry.generation == 0 {
            return Err(Diagnostic {
                code: E_RUNTIME_DIRTY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "auto_reset_scheduler entry for '{}' in scope '{}' has generation 0",
                    entry.path, entry.scope_root
                ),
                source_id: None,
                entity_path: Some("runtime_snapshot.auto_reset_scheduler.pending".to_string()),
                hint: Some("Use monotonic generation values >= 1 for scheduler entries".to_string()),
            });
        }
        find_parameter_in_scope(snapshot, &entry.scope_root, &entry.path)?;

        let current = (
            entry.deadline_unix_ms,
            entry.scope_root.clone(),
            entry.path.clone(),
            entry.generation,
        );
        if let Some(previous_entry) = &previous {
            if current < *previous_entry {
                return Err(Diagnostic {
                    code: E_RUNTIME_DIRTY_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: "auto_reset_scheduler.pending must be sorted by deadline, scope, path, and generation".to_string(),
                    source_id: None,
                    entity_path: Some("runtime_snapshot.auto_reset_scheduler.pending".to_string()),
                    hint: Some("Sort scheduler entries deterministically".to_string()),
                });
            }
        }
        previous = Some(current);
    }
    Ok(())
}

fn validate_event_bus(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    if snapshot.event_bus.buffer_capacity == 0 {
        return Err(Diagnostic {
            code: E_RUNTIME_EVENT_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.event_bus.buffer_capacity must be >= 1".to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.event_bus.buffer_capacity".to_string()),
            hint: Some("Set event buffer capacity to a positive integer".to_string()),
        });
    }
    if snapshot.event_bus.next_sequence == 0 {
        return Err(Diagnostic {
            code: E_RUNTIME_EVENT_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.event_bus.next_sequence must be >= 1".to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.event_bus.next_sequence".to_string()),
            hint: Some("Use monotonic event sequence values >= 1".to_string()),
        });
    }

    let mut last_sequence = 0_u64;
    for event in &snapshot.event_bus.events {
        if event.sequence <= last_sequence {
            return Err(Diagnostic {
                code: E_RUNTIME_EVENT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.event_bus.events must be strictly sequence-ordered".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.event_bus.events".to_string()),
                hint: Some("Store event bus records in ascending sequence order".to_string()),
            });
        }
        last_sequence = event.sequence;
    }

    if let Some(last_sequence) = snapshot.event_bus.events.back().map(|event| event.sequence) {
        if snapshot.event_bus.next_sequence <= last_sequence {
            return Err(Diagnostic {
                code: E_RUNTIME_EVENT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.event_bus.next_sequence must be greater than buffered event sequences".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.event_bus.next_sequence".to_string()),
                hint: Some("Advance next_sequence after each emitted event".to_string()),
            });
        }
    }

    Ok(())
}

fn validate_sync_status(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    if let Some(last_successful_sync_unix_ms) = snapshot.sync_status.last_successful_sync_unix_ms {
        if last_successful_sync_unix_ms == 0 {
            return Err(Diagnostic {
                code: E_RUNTIME_SYNC_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.sync_status.last_successful_sync_unix_ms must be >= 1 when present".to_string(),
                source_id: None,
                entity_path: Some(
                    "runtime_snapshot.sync_status.last_successful_sync_unix_ms".to_string(),
                ),
                hint: Some("Use unix millisecond timestamps >= 1".to_string()),
            });
        }
    }
    if snapshot
        .sync_status
        .pending_update_summary
        .as_ref()
        .map(|value| value.trim().is_empty())
        .unwrap_or(false)
    {
        return Err(Diagnostic {
            code: E_RUNTIME_SYNC_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.sync_status.pending_update_summary must be non-empty when present".to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.sync_status.pending_update_summary".to_string()),
            hint: Some("Use a non-empty summary string or omit the field".to_string()),
        });
    }
    Ok(())
}

fn validate_audit_log(snapshot: &RuntimeSnapshot) -> std::result::Result<(), Diagnostic> {
    let mut last_sequence = 0_u64;
    for event in &snapshot.audit_events {
        if event.sequence == 0 {
            return Err(Diagnostic {
                code: E_RUNTIME_AUDIT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.audit_events entries must have sequence >= 1".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.audit_events".to_string()),
                hint: Some("Use monotonic audit sequence values".to_string()),
            });
        }
        if event.sequence <= last_sequence {
            return Err(Diagnostic {
                code: E_RUNTIME_AUDIT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.audit_events must be strictly sequence-ordered".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.audit_events".to_string()),
                hint: Some("Store audit events in ascending sequence order".to_string()),
            });
        }
        if event.actor.trim().is_empty() {
            return Err(Diagnostic {
                code: E_RUNTIME_AUDIT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.audit_events actor must be non-empty".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.audit_events.actor".to_string()),
                hint: Some("Provide actor identity for each audit event".to_string()),
            });
        }
        if event
            .changed_paths
            .windows(2)
            .any(|window| window[0] >= window[1])
        {
            return Err(Diagnostic {
                code: E_RUNTIME_AUDIT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.audit_events.changed_paths must be sorted and unique".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.audit_events.changed_paths".to_string()),
                hint: Some("Sort and deduplicate changed paths in audit records".to_string()),
            });
        }
        last_sequence = event.sequence;
    }
    if let Some(max_sequence) = snapshot.audit_events.last().map(|event| event.sequence) {
        if snapshot.audit_uploaded_sequence > max_sequence {
            return Err(Diagnostic {
                code: E_RUNTIME_AUDIT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_snapshot.audit_uploaded_sequence cannot exceed max audit sequence".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot.audit_uploaded_sequence".to_string()),
                hint: Some("Use uploaded sequence <= latest local audit sequence".to_string()),
            });
        }
    } else if snapshot.audit_uploaded_sequence != 0 {
        return Err(Diagnostic {
            code: E_RUNTIME_AUDIT_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.audit_uploaded_sequence must be 0 when no audit events exist".to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.audit_uploaded_sequence".to_string()),
            hint: Some("Reset uploaded sequence to 0 for empty audit logs".to_string()),
        });
    }
    if snapshot.audit_next_sequence == 0 {
        return Err(Diagnostic {
            code: E_RUNTIME_AUDIT_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "runtime_snapshot.audit_next_sequence must be >= 1".to_string(),
            source_id: None,
            entity_path: Some("runtime_snapshot.audit_next_sequence".to_string()),
            hint: Some("Initialize audit_next_sequence to 1".to_string()),
        });
    }
    Ok(())
}

fn validate_resolved_component_dependencies(
    resolved_output: &BTreeMap<String, crate::resolved_models::ResolvedConfig>,
    dependencies: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
) -> std::result::Result<(), Diagnostic> {
    for (scope_root, by_component) in dependencies {
        let Some(scope_config) = resolved_output.get(scope_root) else {
            return Err(Diagnostic {
                code: E_RUNTIME_OPEN_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "resolved_component_dependencies references unknown scope root '{}'",
                    scope_root
                ),
                source_id: None,
                entity_path: Some("request.resolved_component_dependencies".to_string()),
                hint: Some("Use scope roots that exist in request.resolved_output".to_string()),
            });
        };

        for (component_id, dependency_ids) in by_component {
            if !scope_config.components.contains_key(component_id) {
                return Err(Diagnostic {
                    code: E_RUNTIME_OPEN_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "resolved_component_dependencies references unknown component '{}' in scope '{}'",
                        component_id, scope_root
                    ),
                    source_id: None,
                    entity_path: Some(format!(
                        "request.resolved_component_dependencies.{}.{}",
                        scope_root, component_id
                    )),
                    hint: Some("Use component IDs present in the resolved scope output".to_string()),
                });
            }

            if dependency_ids
                .windows(2)
                .any(|window| window[0] >= window[1])
            {
                return Err(Diagnostic {
                    code: E_RUNTIME_OPEN_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Dependency list for '{}' in scope '{}' must be sorted and unique",
                        component_id, scope_root
                    ),
                    source_id: None,
                    entity_path: Some(format!(
                        "request.resolved_component_dependencies.{}.{}",
                        scope_root, component_id
                    )),
                    hint: Some("Sort dependency IDs lexicographically and deduplicate".to_string()),
                });
            }

            for dependency_id in dependency_ids {
                if !scope_config.components.contains_key(dependency_id) {
                    return Err(Diagnostic {
                        code: E_RUNTIME_OPEN_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Dependency '{}' for component '{}' in scope '{}' is not present in resolved output",
                            dependency_id, component_id, scope_root
                        ),
                        source_id: None,
                        entity_path: Some(format!(
                            "request.resolved_component_dependencies.{}.{}",
                            scope_root, component_id
                        )),
                        hint: Some("Use dependencies from resolve_result.resolved_component_dependencies".to_string()),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_artifact_references(
    resolved_output: &BTreeMap<String, crate::resolved_models::ResolvedConfig>,
    artifacts: &BTreeMap<String, crate::schema::Artifact>,
) -> std::result::Result<(), Diagnostic> {
    for (scope_root, resolved_scope) in resolved_output {
        for (component_id, component) in &resolved_scope.components {
            for (param_key, parameter) in &component.params {
                if parameter.r#type != "artifact" {
                    continue;
                }
                let path = format!("component.{component_id}.param.{param_key}");
                let artifact_id = match &parameter.value {
                    crate::schema::Value::String(value) if !value.trim().is_empty() => value.trim(),
                    _ => {
                        return Err(Diagnostic {
                            code: E_RUNTIME_ARTIFACT_UNKNOWN.to_string(),
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "Artifact parameter '{}' in scope '{}' must contain a non-empty string artifact ID",
                                path, scope_root
                            ),
                            source_id: None,
                            entity_path: Some(path),
                            hint: Some("Use artifact parameter values as valid string artifact IDs".to_string()),
                        });
                    }
                };
                if !artifacts.contains_key(artifact_id) {
                    return Err(Diagnostic {
                        code: E_RUNTIME_ARTIFACT_UNKNOWN.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Artifact '{}' referenced by '{}' in scope '{}' is not present in request.resolved_artifacts",
                            artifact_id, path, scope_root
                        ),
                        source_id: None,
                        entity_path: Some(path),
                        hint: Some("Include all referenced artifacts in runtime_open_request.resolved_artifacts".to_string()),
                    });
                }
            }
        }
    }
    Ok(())
}

fn compute_resolve_hash(
    model_hash: &str,
    scope: &str,
    context_tags: &BTreeMap<String, String>,
    choices: &BTreeMap<String, String>,
    resolved_output: &serde_json::Value,
    defaulted_choices: &BTreeMap<String, String>,
) -> Result<String> {
    let canonical_output = canonicalize_json_value(resolved_output.clone());
    let canonical = ResolveHashCanonical {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash,
        scope,
        selection_state: SelectionStateCanonical {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_hash,
            scope,
            context_tags,
            choices,
        },
        resolved_output: &canonical_output,
        defaulted_choices,
    };
    let bytes = serde_json::to_vec(&canonical)
        .context("Failed to canonicalize runtime resolve hash payload")?;
    Ok(sha256_hex(&bytes))
}

fn canonicalize_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut entries: Vec<(String, serde_json::Value)> = object.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));

            let mut ordered = serde_json::Map::new();
            for (key, value) in entries {
                ordered.insert(key, canonicalize_json_value(value));
            }
            serde_json::Value::Object(ordered)
        }
        serde_json::Value::Array(array) => {
            serde_json::Value::Array(array.into_iter().map(canonicalize_json_value).collect())
        }
        other => other,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn schema_version_diagnostic(
    code: &str,
    schema_version: u32,
    entity_path: &str,
    hint: &str,
) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!(
            "Unsupported schema_version {} (expected {})",
            schema_version, PRODUCT_SCHEMA_VERSION
        ),
        source_id: None,
        entity_path: Some(entity_path.to_string()),
        hint: Some(hint.to_string()),
    }
}

fn runtime_open_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> RuntimeOpenResult {
    let diagnostics = diagnostics_report(diagnostics);
    RuntimeOpenResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn get_scope_metadata_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    scope_root: String,
    diagnostics: Vec<Diagnostic>,
) -> GetScopeMetadataResult {
    let diagnostics = diagnostics_report(diagnostics);
    GetScopeMetadataResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        scope_root,
        metadata: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn list_parameters_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    scope_root: String,
    diagnostics: Vec<Diagnostic>,
) -> ListParametersResult {
    let diagnostics = diagnostics_report(diagnostics);
    ListParametersResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        scope_root,
        parameter_paths: Vec::new(),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn get_parameter_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    path: String,
    diagnostics: Vec<Diagnostic>,
) -> GetParameterResult {
    let diagnostics = diagnostics_report(diagnostics);
    GetParameterResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        path,
        parameter: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn get_configuration_identity_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> GetConfigurationIdentityResult {
    let diagnostics = diagnostics_report(diagnostics);
    GetConfigurationIdentityResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        identity: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn subscribe_events_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    from_sequence: u64,
    diagnostics: Vec<Diagnostic>,
) -> SubscribeEventsResult {
    let diagnostics = diagnostics_report(diagnostics);
    SubscribeEventsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        from_sequence,
        next_sequence: 0,
        dropped_events: 0,
        events: Vec::new(),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn commit_configuration_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> CommitConfigurationResult {
    let diagnostics = diagnostics_report(diagnostics);
    CommitConfigurationResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        commit_id: None,
        base_configuration_id: None,
        target_configuration_id: None,
        changed_paths: Vec::new(),
        delta_manifest: None,
        unsat_core: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn rollback_dirty_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> RollbackDirtyResult {
    let diagnostics = diagnostics_report(diagnostics);
    RollbackDirtyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        rolled_back_paths: Vec::new(),
        remaining_dirty_paths: Vec::new(),
        rollback_event_id: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn set_parameter_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    path: String,
    diagnostics: Vec<Diagnostic>,
) -> SetParameterResult {
    let diagnostics = diagnostics_report(diagnostics);
    SetParameterResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        path,
        runtime_snapshot: None,
        parameter: None,
        unsat_core: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn set_parameters_atomically_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    rejected_paths: Vec<String>,
    diagnostics: Vec<Diagnostic>,
) -> SetParametersAtomicallyResult {
    let diagnostics = diagnostics_report(diagnostics);
    SetParametersAtomicallyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        applied_count: 0,
        rejected_paths,
        dirty_generation_max: 0,
        unsat_core: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn list_dirty_parameters_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    scope_root: String,
    diagnostics: Vec<Diagnostic>,
) -> ListDirtyParametersResult {
    let diagnostics = diagnostics_report(diagnostics);
    ListDirtyParametersResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        scope_root,
        dirty_paths: Vec::new(),
        dirty_count: 0,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn get_dirty_metadata_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    path: String,
    diagnostics: Vec<Diagnostic>,
) -> GetDirtyMetadataResult {
    let diagnostics = diagnostics_report(diagnostics);
    GetDirtyMetadataResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        path,
        dirty: false,
        scope_root: None,
        metadata: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn set_auto_reset_policy_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> SetAutoResetPolicyResult {
    let diagnostics = diagnostics_report(diagnostics);
    SetAutoResetPolicyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        auto_reset_policy: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn get_auto_reset_policy_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> GetAutoResetPolicyResult {
    let diagnostics = diagnostics_report(diagnostics);
    GetAutoResetPolicyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        auto_reset_policy: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn check_for_updates_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> CheckForUpdatesResult {
    let diagnostics = diagnostics_report(diagnostics);
    CheckForUpdatesResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        sync_status: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn pull_updates_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> PullUpdatesResult {
    let diagnostics = diagnostics_report(diagnostics);
    PullUpdatesResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        applied_paths: Vec::new(),
        conflict_paths: Vec::new(),
        base_configuration_id: None,
        target_configuration_id: None,
        sync_status: None,
        audit_event_id: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn get_sync_status_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> GetSyncStatusResult {
    let diagnostics = diagnostics_report(diagnostics);
    GetSyncStatusResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        sync_status: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn export_pending_sync_bundle_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> ExportPendingSyncBundleResult {
    let diagnostics = diagnostics_report(diagnostics);
    ExportPendingSyncBundleResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        bundle: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn push_audit_events_failed(
    model_hash: String,
    resolve_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> PushAuditEventsResult {
    let diagnostics = diagnostics_report(diagnostics);
    PushAuditEventsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: None,
        pushed_event_ids: Vec::new(),
        pushed_count: 0,
        pending_count: 0,
        last_uploaded_sequence: 0,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn diagnostics_report(diagnostics: Vec<Diagnostic>) -> DiagnosticsReport {
    let error_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .count() as u32;
    let warning_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
        .count() as u32;

    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics,
        error_count,
        warning_count,
    }
}
