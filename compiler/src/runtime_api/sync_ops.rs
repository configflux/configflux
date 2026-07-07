// SPDX-License-Identifier: BUSL-1.1

pub fn check_for_updates(request: CheckForUpdatesRequest) -> CheckForUpdatesResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return check_for_updates_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                "Set check_for_updates.schema_version to 1",
            )],
        );
    }
    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return check_for_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return check_for_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return check_for_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    if request.backend_connected {
        snapshot.sync_status.sync_state = RuntimeSyncState::Checking;
        emit_sync_state_event(&mut snapshot, "checking", None);
        snapshot.sync_status.sync_state = RuntimeSyncState::Idle;
        snapshot.sync_status.pending_update_summary =
            sanitize_optional_text(request.pending_update_summary);
        snapshot.sync_status.sync_diagnostics.clear();
        let pending_summary = snapshot.sync_status.pending_update_summary.clone();
        emit_sync_state_event(
            &mut snapshot,
            "idle",
            pending_summary,
        );
    } else {
        snapshot.sync_status.sync_state = RuntimeSyncState::Offline;
        snapshot.sync_status.sync_diagnostics = vec![Diagnostic {
            code: crate::sync_transport::E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED.to_string(),
            severity: DiagnosticSeverity::Warning,
            message: "Backend sync transport is offline".to_string(),
            source_id: None,
            entity_path: Some("request.backend_connected".to_string()),
            hint: Some("Reconnect backend transport and retry update check".to_string()),
        }];
        emit_sync_state_event(
            &mut snapshot,
            "offline",
            Some("backend_disconnected".to_string()),
        );
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return check_for_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    CheckForUpdatesResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        sync_status: Some(snapshot.sync_status.clone()),
        runtime_snapshot: Some(snapshot),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn pull_updates(request: PullUpdatesRequest) -> PullUpdatesResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return pull_updates_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                "Set pull_updates.schema_version to 1",
            )],
        );
    }

    let actor = request.actor.trim().to_string();
    if actor.is_empty() {
        return pull_updates_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_SYNC_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "pull_updates.actor must be non-empty".to_string(),
                source_id: None,
                entity_path: Some("request.actor".to_string()),
                hint: Some("Provide actor identity for sync apply operations".to_string()),
            }],
        );
    }
    let reason = sanitize_optional_text(request.reason);

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if !request.backend_connected && request.source != SyncApplySource::DirectPush {
        return pull_updates_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: crate::sync_transport::E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "Cannot pull updates while backend transport is offline".to_string(),
                source_id: None,
                entity_path: Some("request.backend_connected".to_string()),
                hint: Some("Reconnect backend transport before pull_updates".to_string()),
            }],
        );
    }

    snapshot.sync_status.sync_state = RuntimeSyncState::Pulling;
    emit_sync_state_event(&mut snapshot, "pulling", None);

    let base_identity = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
        }
    };

    let requested_base_configuration_id = sanitize_optional_text(request.base_configuration_id);
    let requested_target_configuration_id = sanitize_optional_text(request.target_configuration_id);

    if !request.full_snapshot && requested_base_configuration_id.is_none() {
        return pull_updates_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "Delta pull_updates payloads must include request.base_configuration_id".to_string(),
                source_id: None,
                entity_path: Some("request.base_configuration_id".to_string()),
                hint: Some(
                    "Include base/target configuration IDs for delta updates or set full_snapshot=true only for bootstrap/divergence recovery".to_string(),
                ),
            }],
        );
    }
    if !request.full_snapshot && requested_target_configuration_id.is_none() {
        return pull_updates_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "Delta pull_updates payloads must include request.target_configuration_id".to_string(),
                source_id: None,
                entity_path: Some("request.target_configuration_id".to_string()),
                hint: Some(
                    "Include base/target configuration IDs for delta updates or set full_snapshot=true only for bootstrap/divergence recovery".to_string(),
                ),
            }],
        );
    }
    if let Some(base_configuration_id) = &requested_base_configuration_id {
        if !is_sha256_hex(base_configuration_id) {
            return pull_updates_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: crate::sync_transport::E_RUNTIME_SYNC_PAYLOAD_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: "request.base_configuration_id must be a 64-char sha256 hex string".to_string(),
                    source_id: None,
                    entity_path: Some("request.base_configuration_id".to_string()),
                    hint: Some("Use configuration IDs returned by get_configuration_identity".to_string()),
                }],
            );
        }
        if !request.full_snapshot && *base_configuration_id != base_identity.committed_configuration_id {
            return pull_updates_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_SYNC_BASE_MISMATCH.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "base configuration mismatch (expected '{}', current '{}')",
                        base_configuration_id, base_identity.committed_configuration_id
                    ),
                    source_id: None,
                    entity_path: Some("request.base_configuration_id".to_string()),
                    hint: Some(
                        "Request a rebase delta or use full_snapshot=true only for bootstrap/divergence recovery".to_string(),
                    ),
                }],
            );
        }
    }
    if let Some(target_configuration_id) = &requested_target_configuration_id {
        if !is_sha256_hex(target_configuration_id) {
            return pull_updates_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: crate::sync_transport::E_RUNTIME_SYNC_PAYLOAD_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message:
                        "request.target_configuration_id must be a 64-char sha256 hex string".to_string(),
                    source_id: None,
                    entity_path: Some("request.target_configuration_id".to_string()),
                    hint: Some("Use configuration IDs from backend/direct-push delta manifests".to_string()),
                }],
            );
        }
    }

    let writes = request.writes;
    let pending_update_summary = sanitize_optional_text(request.pending_update_summary);
    if writes.is_empty() {
        if let Some(target_configuration_id) = requested_target_configuration_id {
            if target_configuration_id != base_identity.committed_configuration_id {
                return pull_updates_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: E_RUNTIME_SYNC_TARGET_HASH_MISMATCH.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "target configuration mismatch (expected '{}', got '{}')",
                            target_configuration_id, base_identity.committed_configuration_id
                        ),
                        source_id: None,
                        entity_path: Some("request.target_configuration_id".to_string()),
                        hint: Some("Use target IDs from the backend/direct-push delta payload".to_string()),
                    }],
                );
            }
        }
        snapshot.sync_status.sync_state = RuntimeSyncState::Idle;
        snapshot.sync_status.last_successful_sync_unix_ms = Some(now_unix_ms);
        snapshot.sync_status.pending_update_summary = pending_update_summary;
        snapshot.sync_status.sync_diagnostics.clear();
        let pending_summary = snapshot.sync_status.pending_update_summary.clone();
        emit_sync_state_event(
            &mut snapshot,
            "idle",
            pending_summary,
        );
        let diagnostics = DiagnosticsReport {
            schema_version: PRODUCT_SCHEMA_VERSION,
            diagnostics: Vec::new(),
            error_count: 0,
            warning_count: 0,
        };
        return PullUpdatesResult {
            schema_version: PRODUCT_SCHEMA_VERSION,
            status: OperationStatus::Ok,
            model_hash,
            resolve_hash,
            scope,
            runtime_snapshot: Some(snapshot.clone()),
            applied_paths: Vec::new(),
            conflict_paths: Vec::new(),
            base_configuration_id: Some(base_identity.committed_configuration_id.clone()),
            target_configuration_id: Some(base_identity.committed_configuration_id),
            sync_status: Some(snapshot.sync_status),
            audit_event_id: None,
            error_count: diagnostics.error_count,
            warning_count: diagnostics.warning_count,
            diagnostics_ref: None,
            diagnostics,
        };
    }

    struct PreparedPullUpdate {
        validated: ValidatedRuntimeWrite,
        value: crate::schema::Value,
        canonical_path: String,
        was_dirty: bool,
        generation: u64,
    }

    let mut prepared_updates = Vec::with_capacity(writes.len());
    let mut seen_paths = BTreeSet::new();
    for write in writes {
        let validated =
            match validate_runtime_write(&snapshot, &write.path, &write.value, "request.writes", false) {
                Ok(value) => value,
                Err(diagnostic) => {
                    return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
                }
            };
        let canonical_path = canonical_runtime_path(&validated.scope_root, &validated.path);
        if !seen_paths.insert(canonical_path.clone()) {
            return pull_updates_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_SYNC_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Duplicate changed path '{}' in request.writes is not allowed",
                        canonical_path
                    ),
                    source_id: None,
                    entity_path: Some("request.writes".to_string()),
                    hint: Some("Include each changed path at most once in delta payloads".to_string()),
                }],
            );
        }

        let expected_before_leaf_hash = sanitize_optional_text(write.before_leaf_hash);
        let expected_after_leaf_hash = sanitize_optional_text(write.after_leaf_hash);
        if !request.full_snapshot && expected_before_leaf_hash.is_none() {
            return pull_updates_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Delta payload for '{}' must include before_leaf_hash",
                        canonical_path
                    ),
                    source_id: None,
                    entity_path: Some("request.writes.before_leaf_hash".to_string()),
                    hint: Some("Include before_leaf_hash for all changed delta paths".to_string()),
                }],
            );
        }
        if let Some(before_leaf_hash) = &expected_before_leaf_hash {
            if !is_sha256_hex(before_leaf_hash) {
                return pull_updates_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: crate::sync_transport::E_RUNTIME_SYNC_PAYLOAD_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "before_leaf_hash for '{}' must be a 64-char sha256 hex string",
                            canonical_path
                        ),
                        source_id: None,
                        entity_path: Some("request.writes.before_leaf_hash".to_string()),
                        hint: Some("Use leaf hashes from upstream delta manifests".to_string()),
                    }],
                );
            }
        }
        if let Some(after_leaf_hash) = &expected_after_leaf_hash {
            if !is_sha256_hex(after_leaf_hash) {
                return pull_updates_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: crate::sync_transport::E_RUNTIME_SYNC_PAYLOAD_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "after_leaf_hash for '{}' must be a 64-char sha256 hex string",
                            canonical_path
                        ),
                        source_id: None,
                        entity_path: Some("request.writes.after_leaf_hash".to_string()),
                        hint: Some("Use leaf hashes from upstream delta manifests".to_string()),
                    }],
                );
            }
        }

        let before_value = committed_parameter_value(
            &snapshot,
            &validated.scope_root,
            &validated.path,
            &validated.baseline_parameter.value,
        );
        let before_leaf_hash = match identity_leaf_hash(
            &validated.scope_root,
            &validated.path,
            &validated.baseline_parameter,
            before_value,
        ) {
            Ok(hash) => hash,
            Err(diagnostic) => {
                return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
            }
        };
        if let Some(before_leaf_hash_expected) = expected_before_leaf_hash {
            if before_leaf_hash_expected != before_leaf_hash {
                return pull_updates_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "before leaf hash mismatch for '{}' (expected '{}', current '{}')",
                            canonical_path, before_leaf_hash_expected, before_leaf_hash
                        ),
                        source_id: None,
                        entity_path: Some("request.writes.before_leaf_hash".to_string()),
                        hint: Some("Request a rebase delta for the current base configuration".to_string()),
                    }],
                );
            }
        }

        let after_leaf_hash = match identity_leaf_hash(
            &validated.scope_root,
            &validated.path,
            &validated.baseline_parameter,
            write.value.clone(),
        ) {
            Ok(hash) => hash,
            Err(diagnostic) => {
                return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
            }
        };
        if let Some(after_leaf_hash_expected) = expected_after_leaf_hash {
            if after_leaf_hash_expected != after_leaf_hash {
                return pull_updates_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: E_RUNTIME_SYNC_TARGET_HASH_MISMATCH.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "after leaf hash mismatch for '{}' (expected '{}', computed '{}')",
                            canonical_path, after_leaf_hash_expected, after_leaf_hash
                        ),
                        source_id: None,
                        entity_path: Some("request.writes.after_leaf_hash".to_string()),
                        hint: Some("Fix changed path payload values to match target leaf hashes".to_string()),
                    }],
                );
            }
        }

        let was_dirty =
            overlay_value(&snapshot.dirty_overlay, &validated.scope_root, &validated.path).is_some();
        let generation = snapshot
            .dirty_generations
            .get(&validated.scope_root)
            .and_then(|entries| entries.get(&validated.path))
            .copied()
            .or_else(|| {
                snapshot
                    .dirty_metadata
                    .get(&validated.scope_root)
                    .and_then(|entries| entries.get(&validated.path))
                    .map(|metadata| metadata.generation)
            })
            .unwrap_or(1);
        prepared_updates.push(PreparedPullUpdate {
            validated,
            value: write.value,
            canonical_path,
            was_dirty,
            generation,
        });
    }

    snapshot.sync_status.sync_state = RuntimeSyncState::Applying;
    emit_sync_state_event(&mut snapshot, "applying", None);

    let mut applied_paths = Vec::new();
    let mut conflict_paths: Vec<String> = prepared_updates
        .iter()
        .filter(|update| update.was_dirty)
        .map(|update| update.canonical_path.clone())
        .collect();
    conflict_paths.sort();
    conflict_paths.dedup();
    if !conflict_paths.is_empty() {
        emit_runtime_event(
            &mut snapshot,
            RuntimeEventKind::SyncConflictDetected,
            now_unix_ms,
            Some(&actor),
            reason.as_deref(),
            None,
            None,
            RuntimeEventPayload::SyncConflictDetected {
                conflict_paths: conflict_paths.clone(),
            },
        );
    }

    for update in prepared_updates {
        let validated = update.validated;
        let value = update.value;
        let canonical_path = update.canonical_path;
        let was_dirty = update.was_dirty;
        let generation = update.generation;

        let previous_effective_value = effective_parameter_value(
            &snapshot,
            &validated.scope_root,
            &validated.path,
            &validated.baseline_parameter.value,
        );
        if value == validated.baseline_parameter.value {
            let remove_scope =
                if let Some(values) = snapshot.committed_overlay.get_mut(&validated.scope_root) {
                    values.remove(&validated.path);
                    values.is_empty()
                } else {
                    false
                };
            if remove_scope {
                snapshot.committed_overlay.remove(&validated.scope_root);
            }
        } else {
            snapshot
                .committed_overlay
                .entry(validated.scope_root.clone())
                .or_default()
                .insert(validated.path.clone(), value);
        }
        if was_dirty {
            clear_dirty_entry(
                &mut snapshot,
                &validated.scope_root,
                &validated.path,
                generation,
            );
            emit_runtime_event(
                &mut snapshot,
                RuntimeEventKind::DirtyStateChanged,
                now_unix_ms,
                Some(&actor),
                reason.as_deref(),
                None,
                None,
                RuntimeEventPayload::DirtyStateChanged {
                    scope_root: validated.scope_root.clone(),
                    path: validated.path.clone(),
                    dirty: false,
                    generation,
                },
            );
        }
        let updated_effective_value = effective_parameter_value(
            &snapshot,
            &validated.scope_root,
            &validated.path,
            &validated.baseline_parameter.value,
        );
        emit_runtime_event(
            &mut snapshot,
            RuntimeEventKind::ParameterChanged,
            now_unix_ms,
            Some(&actor),
            reason.as_deref(),
            Some(&previous_effective_value),
            Some(&updated_effective_value),
            RuntimeEventPayload::ParameterChanged {
                scope_root: validated.scope_root,
                path: validated.path,
                generation,
            },
        );
        applied_paths.push(canonical_path);
    }
    applied_paths.sort();
    applied_paths.dedup();

    let target_identity = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
        }
    };
    if let Some(target_configuration_id) = requested_target_configuration_id {
        if target_configuration_id != target_identity.committed_configuration_id {
            return pull_updates_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_SYNC_TARGET_HASH_MISMATCH.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "target configuration mismatch (expected '{}', got '{}')",
                        target_configuration_id, target_identity.committed_configuration_id
                    ),
                    source_id: None,
                    entity_path: Some("request.target_configuration_id".to_string()),
                    hint: Some("Use target IDs from the backend delta payload".to_string()),
                }],
            );
        }
    }

    snapshot.sync_status.sync_state = RuntimeSyncState::Idle;
    snapshot.sync_status.last_successful_sync_unix_ms = Some(now_unix_ms);
    snapshot.sync_status.pending_update_summary = pending_update_summary;
    let conflict_warning = if conflict_paths.is_empty() {
        None
    } else {
        Some(Diagnostic {
            code: E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN.to_string(),
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "Upstream values overrode {} locally dirty changed path(s)",
                conflict_paths.len()
            ),
            source_id: None,
            entity_path: Some("request.writes".to_string()),
            hint: Some("Review conflict_paths for commissioning/reconciliation follow-up".to_string()),
        })
    };
    snapshot.sync_status.sync_diagnostics = conflict_warning.clone().into_iter().collect();
    let pending_summary = snapshot.sync_status.pending_update_summary.clone();
    emit_sync_state_event(
        &mut snapshot,
        "idle",
        pending_summary,
    );
    if !conflict_paths.is_empty() {
        emit_runtime_event(
            &mut snapshot,
            RuntimeEventKind::SyncConflictResolved,
            now_unix_ms,
            Some(&actor),
            reason.as_deref(),
            None,
            None,
            RuntimeEventPayload::SyncConflictResolved {
                conflict_paths: conflict_paths.clone(),
                applied_paths: applied_paths.clone(),
                source: request.source,
            },
        );
    }
    emit_runtime_event(
        &mut snapshot,
        RuntimeEventKind::SyncApplyCompleted,
        now_unix_ms,
        Some(&actor),
        reason.as_deref(),
        None,
        None,
        RuntimeEventPayload::SyncApplyCompleted {
            source: request.source,
            base_configuration_id: base_identity.committed_configuration_id.clone(),
            target_configuration_id: target_identity.committed_configuration_id.clone(),
            applied_paths: applied_paths.clone(),
            conflict_paths: conflict_paths.clone(),
        },
    );

    let audit_event_id = append_audit_event(
        &mut snapshot,
        match request.source {
            SyncApplySource::Backend => RuntimeAuditEventKind::SyncApply,
            SyncApplySource::DirectPush => RuntimeAuditEventKind::DirectPush,
        },
        now_unix_ms,
        &actor,
        reason.as_deref(),
        applied_paths.clone(),
        &target_identity,
        Some(base_identity.committed_configuration_id.clone()),
        Some(target_identity.committed_configuration_id.clone()),
    );

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return pull_updates_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let diagnostics = diagnostics_report(conflict_warning.into_iter().collect());
    PullUpdatesResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: Some(snapshot.clone()),
        applied_paths,
        conflict_paths,
        base_configuration_id: Some(base_identity.committed_configuration_id),
        target_configuration_id: Some(target_identity.committed_configuration_id),
        sync_status: Some(snapshot.sync_status),
        audit_event_id: Some(audit_event_id),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn get_sync_status(request: GetSyncStatusRequest) -> GetSyncStatusResult {
    let snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return get_sync_status_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                "Set get_sync_status.schema_version to 1",
            )],
        );
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return get_sync_status_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    GetSyncStatusResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        sync_status: Some(snapshot.sync_status),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn push_audit_events(request: PushAuditEventsRequest) -> PushAuditEventsResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return push_audit_events_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                "Set push_audit_events.schema_version to 1",
            )],
        );
    }
    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return push_audit_events_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return push_audit_events_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let max_events = request.max_events.max(1) as usize;
    let pending_events: Vec<&RuntimeAuditEvent> = snapshot
        .audit_events
        .iter()
        .filter(|event| event.sequence > snapshot.audit_uploaded_sequence)
        .collect();
    if !request.backend_connected {
        let diagnostics = DiagnosticsReport {
            schema_version: PRODUCT_SCHEMA_VERSION,
            diagnostics: Vec::new(),
            error_count: 0,
            warning_count: 0,
        };
        return PushAuditEventsResult {
            schema_version: PRODUCT_SCHEMA_VERSION,
            status: OperationStatus::Ok,
            model_hash,
            resolve_hash,
            scope,
            runtime_snapshot: Some(snapshot.clone()),
            pushed_event_ids: Vec::new(),
            pushed_count: 0,
            pending_count: pending_events.len() as u32,
            last_uploaded_sequence: snapshot.audit_uploaded_sequence,
            error_count: diagnostics.error_count,
            warning_count: diagnostics.warning_count,
            diagnostics_ref: None,
            diagnostics,
        };
    }

    let mut pushed_event_ids = Vec::new();
    let mut last_uploaded_sequence = snapshot.audit_uploaded_sequence;
    for event in pending_events.into_iter().take(max_events) {
        pushed_event_ids.push(event.event_id.clone());
        last_uploaded_sequence = event.sequence;
    }
    if last_uploaded_sequence > snapshot.audit_uploaded_sequence {
        snapshot.audit_uploaded_sequence = last_uploaded_sequence;
    }
    let pending_count = snapshot
        .audit_events
        .iter()
        .filter(|event| event.sequence > snapshot.audit_uploaded_sequence)
        .count() as u32;

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    PushAuditEventsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: Some(snapshot),
        pushed_count: pushed_event_ids.len() as u32,
        pushed_event_ids,
        pending_count,
        last_uploaded_sequence,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn export_pending_sync_bundle(
    request: ExportPendingSyncBundleRequest,
) -> ExportPendingSyncBundleResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return export_pending_sync_bundle_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                "Set export_pending_sync_bundle.schema_version to 1",
            )],
        );
    }
    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return export_pending_sync_bundle_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return export_pending_sync_bundle_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let identity = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return export_pending_sync_bundle_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
        }
    };
    let dirty_paths: Vec<String> = collect_dirty_path_selections(&snapshot)
        .into_iter()
        .map(|entry| entry.canonical_path)
        .collect();
    let max_events = request.max_audit_events.max(1) as usize;
    let pending_audit_events: Vec<RuntimeAuditEvent> = snapshot
        .audit_events
        .iter()
        .filter(|event| event.sequence > snapshot.audit_uploaded_sequence)
        .take(max_events)
        .cloned()
        .collect();
    let pending_audit_count = snapshot
        .audit_events
        .iter()
        .filter(|event| event.sequence > snapshot.audit_uploaded_sequence)
        .count() as u32;
    let bundle_id = match stable_hash(&(
        PRODUCT_SCHEMA_VERSION,
        &scope,
        &identity.committed_configuration_id,
        &identity.working_configuration_id,
        snapshot.audit_uploaded_sequence,
        &dirty_paths,
        pending_audit_events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
    )) {
        Ok(hash) => format!("offline-sync-{}", &hash[..16]),
        Err(error) => {
            return export_pending_sync_bundle_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_SYNC_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!("Failed to canonicalize offline reconciliation bundle: {error}"),
                    source_id: None,
                    entity_path: Some("runtime_snapshot".to_string()),
                    hint: Some("Use canonical runtime snapshot payloads before export".to_string()),
                }],
            );
        }
    };

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    ExportPendingSyncBundleResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        bundle: Some(OfflineReconciliationBundle {
            schema_version: PRODUCT_SCHEMA_VERSION,
            bundle_id,
            generated_at_unix_ms: now_unix_ms,
            committed_configuration_id: identity.committed_configuration_id,
            working_configuration_id: identity.working_configuration_id,
            dirty_paths,
            audit_uploaded_sequence: snapshot.audit_uploaded_sequence,
            pending_audit_count,
            pending_audit_events,
            sync_status: snapshot.sync_status,
        }),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

