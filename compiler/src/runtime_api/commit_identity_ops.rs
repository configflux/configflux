// SPDX-License-Identifier: BUSL-1.1

pub fn rollback_dirty(request: RollbackDirtyRequest) -> RollbackDirtyResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return rollback_dirty_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set rollback_dirty.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    let actor = request.actor.trim().to_string();
    if actor.is_empty() {
        return rollback_dirty_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_DIRTY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "rollback_dirty.actor must be non-empty".to_string(),
                source_id: None,
                entity_path: Some("request.actor".to_string()),
                hint: Some("Provide actor identity for rollback operations".to_string()),
            }],
        );
    }
    let reason = sanitize_optional_text(request.reason);

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return rollback_dirty_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return rollback_dirty_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return rollback_dirty_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let selections = match request.mode {
        RollbackMode::All => collect_dirty_path_selections(&snapshot),
        RollbackMode::Subset => {
            if request.paths.is_empty() {
                return rollback_dirty_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: E_RUNTIME_DIRTY_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: "rollback_dirty.paths must be non-empty when mode=subset".to_string(),
                        source_id: None,
                        entity_path: Some("request.paths".to_string()),
                        hint: Some("Provide one or more dirty paths when mode is subset".to_string()),
                    }],
                );
            }
            let mut by_path: BTreeMap<String, DirtyPathSelection> = BTreeMap::new();
            for raw_path in request.paths {
                let resolved =
                    match resolve_runtime_path(&snapshot, &raw_path, "request.paths") {
                        Ok(value) => value,
                        Err(diagnostic) => {
                            return rollback_dirty_failed(
                                model_hash,
                                resolve_hash,
                                scope,
                                vec![diagnostic],
                            );
                        }
                    };
                let Some(dirty_value) =
                    overlay_value(&snapshot.dirty_overlay, &resolved.scope_root, &resolved.path)
                else {
                    return rollback_dirty_failed(
                        model_hash,
                        resolve_hash,
                        scope,
                        vec![Diagnostic {
                            code: E_RUNTIME_DIRTY_INVALID.to_string(),
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "Path '{}' is not dirty and cannot be rolled back",
                                resolved.canonical_path
                            ),
                            source_id: None,
                            entity_path: Some("request.paths".to_string()),
                            hint: Some("Use list_dirty_parameters/get_dirty_metadata before rollback".to_string()),
                        }],
                    );
                };
                let generation = snapshot
                    .dirty_generations
                    .get(&resolved.scope_root)
                    .and_then(|entries| entries.get(&resolved.path))
                    .copied()
                    .or_else(|| {
                        snapshot
                            .dirty_metadata
                            .get(&resolved.scope_root)
                            .and_then(|entries| entries.get(&resolved.path))
                            .map(|metadata| metadata.generation)
                    })
                    .unwrap_or(0);
                if generation == 0 {
                    return rollback_dirty_failed(
                        model_hash,
                        resolve_hash,
                        scope,
                        vec![Diagnostic {
                            code: E_RUNTIME_DIRTY_INVALID.to_string(),
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "Path '{}' has no valid dirty generation",
                                resolved.canonical_path
                            ),
                            source_id: None,
                            entity_path: Some("runtime_snapshot.dirty_generations".to_string()),
                            hint: Some("Normalize runtime snapshot before rollback".to_string()),
                        }],
                    );
                }

                let _ = dirty_value;
                by_path.insert(
                    resolved.canonical_path.clone(),
                    DirtyPathSelection {
                        scope_root: resolved.scope_root,
                        path: resolved.path,
                        canonical_path: resolved.canonical_path,
                        generation,
                    },
                );
            }
            by_path.into_values().collect()
        }
    };

    let mut rolled_back_paths = Vec::new();
    for selection in selections {
        let Some(old_value) =
            overlay_value(&snapshot.dirty_overlay, &selection.scope_root, &selection.path).cloned()
        else {
            continue;
        };
        let baseline_parameter =
            match find_parameter_in_scope(&snapshot, &selection.scope_root, &selection.path) {
                Ok(parameter) => parameter.clone(),
                Err(diagnostic) => {
                    return rollback_dirty_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
                }
            };
        clear_dirty_entry(
            &mut snapshot,
            &selection.scope_root,
            &selection.path,
            selection.generation,
        );
        let committed_or_baseline = effective_parameter_value(
            &snapshot,
            &selection.scope_root,
            &selection.path,
            &baseline_parameter.value,
        );
        emit_runtime_event(
            &mut snapshot,
            RuntimeEventKind::ResetApplied,
            now_unix_ms,
            Some(&actor),
            reason.as_deref(),
            Some(&old_value),
            Some(&committed_or_baseline),
            RuntimeEventPayload::ResetApplied {
                scope_root: selection.scope_root.clone(),
                path: selection.path.clone(),
                cause: "rollback".to_string(),
                generation: selection.generation,
            },
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
                scope_root: selection.scope_root.clone(),
                path: selection.path.clone(),
                dirty: false,
                generation: selection.generation,
            },
        );
        rolled_back_paths.push(selection.canonical_path);
    }
    rolled_back_paths.sort();
    rolled_back_paths.dedup();

    let rollback_event_id = if rolled_back_paths.is_empty() {
        None
    } else {
        snapshot.persistence_journal_sequence =
            snapshot.persistence_journal_sequence.saturating_add(1);
        Some(emit_runtime_event(
            &mut snapshot,
            RuntimeEventKind::RollbackApplied,
            now_unix_ms,
            Some(&actor),
            reason.as_deref(),
            None,
            None,
            RuntimeEventPayload::RollbackApplied {
                rolled_back_paths: rolled_back_paths.clone(),
            },
        ))
    };
    if !rolled_back_paths.is_empty() {
        let identity = match compute_configuration_identity(&snapshot) {
            Ok(identity) => identity,
            Err(diagnostic) => {
                return rollback_dirty_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
            }
        };
        append_audit_event(
            &mut snapshot,
            RuntimeAuditEventKind::Reset,
            now_unix_ms,
            &actor,
            reason.as_deref(),
            rolled_back_paths.clone(),
            &identity,
            None,
            None,
        );
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return rollback_dirty_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let remaining_dirty_paths = collect_dirty_path_selections(&snapshot)
        .into_iter()
        .map(|entry| entry.canonical_path)
        .collect();

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    RollbackDirtyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: Some(snapshot),
        rolled_back_paths,
        remaining_dirty_paths,
        rollback_event_id,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn commit_configuration(request: CommitConfigurationRequest) -> CommitConfigurationResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return commit_configuration_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set commit_configuration.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    let actor = request.actor.trim().to_string();
    if actor.is_empty() {
        return commit_configuration_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_COMMIT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "commit_configuration.actor must be non-empty".to_string(),
                source_id: None,
                entity_path: Some("request.actor".to_string()),
                hint: Some("Provide actor identity for commit operations".to_string()),
            }],
        );
    }
    let reason = sanitize_optional_text(request.reason);

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return commit_configuration_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return commit_configuration_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return commit_configuration_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let base_identity = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return commit_configuration_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
        }
    };
    let base_configuration_id = base_identity.committed_configuration_id;
    if let Some(expected_base) = request.expected_base_configuration_id.as_deref() {
        // A field the caller SENT is an expectation, exactly as sent. There is no
        // trim and no blank filter: a blank used to mean "no expectation at all",
        // so a request assembled from an unset variable committed with no
        // compare-and-swap enforced and reported ok, and a padded digest was
        // trimmed into a match the caller never sent (configflux-8gah). Only an
        // omitted or null field is absent. The gate then runs ahead of the
        // comparison, because an unmatchable value reported as a base mismatch
        // tells the operator the configuration moved under them when the fault
        // is the value they sent (configflux-11wx).
        if !is_sha256_hex(expected_base) {
            return commit_configuration_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_COMMIT_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message:
                        "request.expected_base_configuration_id must be a 64-char lowercase sha256 hex string"
                            .to_string(),
                    source_id: None,
                    entity_path: Some("request.expected_base_configuration_id".to_string()),
                    hint: Some("Use configuration IDs returned by get_configuration_identity".to_string()),
                }],
            );
        }
        if expected_base != base_configuration_id {
            return commit_configuration_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_COMMIT_BASE_MISMATCH.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "commit base mismatch (expected '{}', current '{}')",
                        expected_base, base_configuration_id
                    ),
                    source_id: None,
                    entity_path: Some("request.expected_base_configuration_id".to_string()),
                    hint: Some("Refresh runtime identity and retry commit".to_string()),
                }],
            );
        }
    }

    let dirty_paths = collect_dirty_path_selections(&snapshot);
    if !request.changed_paths_hint.is_empty() {
        let dirty_path_set: BTreeSet<String> =
            dirty_paths.iter().map(|entry| entry.canonical_path.clone()).collect();
        for raw_path in request.changed_paths_hint {
            let resolved = match resolve_runtime_path(&snapshot, &raw_path, "request.changed_paths_hint") {
                Ok(value) => value,
                Err(diagnostic) => {
                    return commit_configuration_failed(
                        model_hash,
                        resolve_hash,
                        scope,
                        vec![diagnostic],
                    );
                }
            };
            if !dirty_path_set.contains(&resolved.canonical_path) {
                return commit_configuration_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: E_RUNTIME_COMMIT_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "changed path hint '{}' is not currently dirty",
                            resolved.canonical_path
                        ),
                        source_id: None,
                        entity_path: Some("request.changed_paths_hint".to_string()),
                        hint: Some("Provide only dirty paths in commit hints".to_string()),
                    }],
                );
            }
        }
    }

    let mut changed_paths = Vec::new();
    for entry in &dirty_paths {
        let baseline_parameter = match find_parameter_in_scope(&snapshot, &entry.scope_root, &entry.path) {
            Ok(parameter) => parameter.clone(),
            Err(diagnostic) => {
                return commit_configuration_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
            }
        };
        let before_value =
            committed_parameter_value(&snapshot, &entry.scope_root, &entry.path, &baseline_parameter.value);
        let Some(after_value) =
            overlay_value(&snapshot.dirty_overlay, &entry.scope_root, &entry.path).cloned()
        else {
            continue;
        };

        if after_value == baseline_parameter.value {
            let remove_scope = if let Some(values) = snapshot.committed_overlay.get_mut(&entry.scope_root) {
                values.remove(&entry.path);
                values.is_empty()
            } else {
                false
            };
            if remove_scope {
                snapshot.committed_overlay.remove(&entry.scope_root);
            }
        } else {
            snapshot
                .committed_overlay
                .entry(entry.scope_root.clone())
                .or_default()
                .insert(entry.path.clone(), after_value.clone());
        }

        clear_dirty_entry(&mut snapshot, &entry.scope_root, &entry.path, entry.generation);
        emit_runtime_event(
            &mut snapshot,
            RuntimeEventKind::DirtyStateChanged,
            now_unix_ms,
            Some(&actor),
            reason.as_deref(),
            None,
            None,
            RuntimeEventPayload::DirtyStateChanged {
                scope_root: entry.scope_root.clone(),
                path: entry.path.clone(),
                dirty: false,
                generation: entry.generation,
            },
        );

        if before_value == after_value {
            continue;
        }

        let before_leaf_hash =
            match identity_leaf_hash(&entry.scope_root, &entry.path, &baseline_parameter, before_value.clone()) {
                Ok(hash) => hash,
                Err(diagnostic) => {
                    return commit_configuration_failed(
                        model_hash,
                        resolve_hash,
                        scope,
                        vec![diagnostic],
                    );
                }
            };
        let after_leaf_hash =
            match identity_leaf_hash(&entry.scope_root, &entry.path, &baseline_parameter, after_value.clone()) {
                Ok(hash) => hash,
                Err(diagnostic) => {
                    return commit_configuration_failed(
                        model_hash,
                        resolve_hash,
                        scope,
                        vec![diagnostic],
                    );
                }
            };
        changed_paths.push(RuntimeDeltaPathChange {
            path: entry.canonical_path.clone(),
            change_kind: RuntimeDeltaChangeKind::Set,
            before_leaf_hash: Some(before_leaf_hash),
            after_leaf_hash: Some(after_leaf_hash),
            before_value: Some(before_value),
            after_value: Some(after_value),
        });
    }
    changed_paths.sort_by(|left, right| left.path.cmp(&right.path));

    snapshot.persistence_journal_sequence = snapshot.persistence_journal_sequence.saturating_add(1);
    let target_identity = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return commit_configuration_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
        }
    };
    if target_identity.committed_configuration_id != target_identity.working_configuration_id {
        return commit_configuration_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_COMMIT_TARGET_HASH_MISMATCH.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "commit produced divergent committed and working configuration IDs".to_string(),
                source_id: None,
                entity_path: Some("runtime_snapshot".to_string()),
                hint: Some("Ensure all dirty paths are committed atomically".to_string()),
            }],
        );
    }

    let target_configuration_id = target_identity.committed_configuration_id.clone();
    let manifest_id = match stable_hash(&(
        PRODUCT_SCHEMA_VERSION,
        snapshot.persistence_journal_sequence,
        &base_configuration_id,
        &target_configuration_id,
        &changed_paths,
        &actor,
        &reason,
    )) {
        Ok(hash) => hash,
        Err(error) => {
            return commit_configuration_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_COMMIT_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!("Failed to canonicalize commit manifest payload: {error}"),
                    source_id: None,
                    entity_path: Some("runtime_snapshot".to_string()),
                    hint: Some("Use canonical runtime state before commit".to_string()),
                }],
            );
        }
    };
    let commit_id = format!("commit-{:016x}", snapshot.persistence_journal_sequence);
    let delta_manifest = RuntimeDeltaManifest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        manifest_id,
        base_configuration_id: base_configuration_id.clone(),
        target_configuration_id: target_configuration_id.clone(),
        changed_paths: changed_paths.clone(),
        created_at_unix_ms: now_unix_ms,
        actor: actor.clone(),
        reason: reason.clone(),
    };
    emit_commit_event(
        &mut snapshot,
        Some(actor.clone()),
        reason.clone(),
        commit_id.clone(),
        changed_paths
            .iter()
            .map(|entry| entry.path.clone())
            .collect(),
    );
    append_audit_event(
        &mut snapshot,
        RuntimeAuditEventKind::Commit,
        now_unix_ms,
        &actor,
        reason.as_deref(),
        changed_paths
            .iter()
            .map(|entry| entry.path.clone())
            .collect(),
        &target_identity,
        Some(base_configuration_id.clone()),
        Some(target_configuration_id.clone()),
    );

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return commit_configuration_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    CommitConfigurationResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: Some(snapshot),
        commit_id: Some(commit_id),
        base_configuration_id: Some(base_configuration_id),
        target_configuration_id: Some(target_configuration_id),
        changed_paths,
        delta_manifest: Some(delta_manifest),
        unsat_core: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn get_configuration_identity(
    request: GetConfigurationIdentityRequest,
) -> GetConfigurationIdentityResult {
    let snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return get_configuration_identity_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set get_configuration_identity.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return get_configuration_identity_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let identity = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return get_configuration_identity_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![diagnostic],
            );
        }
    };

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };

    GetConfigurationIdentityResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        identity: Some(identity),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn subscribe_events(request: SubscribeEventsRequest) -> SubscribeEventsResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return subscribe_events_failed(
            model_hash,
            resolve_hash,
            scope,
            request.from_sequence,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set subscribe_events.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return subscribe_events_failed(
            model_hash,
            resolve_hash,
            scope,
            request.from_sequence,
            vec![diagnostic],
        );
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return subscribe_events_failed(
            model_hash,
            resolve_hash,
            scope,
            request.from_sequence,
            vec![diagnostic],
        );
    }

    let allowed_kinds: BTreeSet<RuntimeEventKind> = request.event_kinds.into_iter().collect();
    let max_events = request.max_events.max(1) as usize;
    let mut events = Vec::new();
    for event in &snapshot.event_bus.events {
        if event.sequence <= request.from_sequence {
            continue;
        }
        if !allowed_kinds.is_empty() && !allowed_kinds.contains(&event.event_kind) {
            continue;
        }
        events.push(event.clone());
        if events.len() >= max_events {
            break;
        }
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };

    SubscribeEventsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        from_sequence: request.from_sequence,
        next_sequence: snapshot.event_bus.next_sequence,
        dropped_events: snapshot.event_bus.dropped_events,
        events,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn emit_commit_event(
    snapshot: &mut RuntimeSnapshot,
    actor: Option<String>,
    reason: Option<String>,
    commit_id: String,
    mut changed_paths: Vec<String>,
) {
    changed_paths.sort();
    changed_paths.dedup();
    emit_runtime_event(
        snapshot,
        RuntimeEventKind::CommitApplied,
        current_time_unix_ms(),
        actor.as_deref(),
        reason.as_deref(),
        None,
        None,
        RuntimeEventPayload::CommitApplied {
            commit_id,
            changed_paths,
        },
    );
}

pub fn emit_sync_state_event(snapshot: &mut RuntimeSnapshot, state: impl Into<String>, summary: Option<String>) {
    emit_runtime_event(
        snapshot,
        RuntimeEventKind::SyncStateChanged,
        current_time_unix_ms(),
        None,
        None,
        None,
        None,
        RuntimeEventPayload::SyncStateChanged {
            state: state.into(),
            summary,
        },
    );
}

