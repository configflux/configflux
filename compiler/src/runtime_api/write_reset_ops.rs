// SPDX-License-Identifier: BUSL-1.1

#[derive(Debug, Clone)]
struct ValidatedRuntimeWrite {
    scope_root: String,
    component_id: String,
    param_key: String,
    path: String,
    baseline_parameter: crate::resolved_models::ResolvedParameter,
}

fn validate_runtime_write(
    snapshot: &RuntimeSnapshot,
    path: &str,
    value: &crate::schema::Value,
    entity_path: &str,
    enforce_runtime_lifecycle: bool,
) -> std::result::Result<ValidatedRuntimeWrite, Diagnostic> {
    let (component_id, param_key) = parse_parameter_path(path).ok_or_else(|| Diagnostic {
        code: E_RUNTIME_UNKNOWN_PATH.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Unknown parameter path '{}'", path),
        source_id: None,
        entity_path: Some(entity_path.to_string()),
        hint: Some("Use path format component.<component_id>.param.<param_key>".to_string()),
    })?;

    let scope_root = find_parameter_scope_root(snapshot, component_id, param_key)?;
    let baseline_parameter = find_parameter_in_scope(snapshot, &scope_root, path)?.clone();

    if enforce_runtime_lifecycle && baseline_parameter.lifecycle != crate::schema::Lifecycle::Runtime {
        return Err(Diagnostic {
            code: E_RUNTIME_LIFECYCLE_IMMUTABLE.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Parameter '{}' is not writable at runtime (lifecycle={:?})",
                path, baseline_parameter.lifecycle
            ),
            source_id: None,
            entity_path: Some(path.to_string()),
            hint: Some("Only lifecycle=runtime parameters are writable".to_string()),
        });
    }

    if !is_value_compatible_with_type(&baseline_parameter.r#type, value) {
        return Err(Diagnostic {
            code: E_RUNTIME_TYPE_MISMATCH.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Parameter '{}' expects type '{}' but received '{}'",
                path,
                baseline_parameter.r#type,
                value_kind(value)
            ),
            source_id: None,
            entity_path: Some(path.to_string()),
            hint: Some("Write values that match resolved parameter type".to_string()),
        });
    }

    validate_limits(path, value, baseline_parameter.limits.as_ref())?;

    if baseline_parameter.r#type == "artifact" {
        let artifact_id = match value {
            crate::schema::Value::String(current) if !current.trim().is_empty() => current.trim(),
            _ => {
                return Err(Diagnostic {
                    code: E_RUNTIME_TYPE_MISMATCH.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Artifact parameter '{}' must be updated with a non-empty string artifact ID",
                        path
                    ),
                    source_id: None,
                    entity_path: Some(path.to_string()),
                    hint: Some("Set artifact parameters using known artifact IDs".to_string()),
                });
            }
        };

        if !snapshot.resolved_artifacts.contains_key(artifact_id) {
            return Err(Diagnostic {
                code: E_RUNTIME_ARTIFACT_UNKNOWN.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Artifact '{}' referenced by '{}' is not present in runtime snapshot",
                    artifact_id, path
                ),
                source_id: None,
                entity_path: Some(path.to_string()),
                hint: Some("Use artifact IDs from runtime_snapshot.resolved_artifacts".to_string()),
            });
        }
    }

    Ok(ValidatedRuntimeWrite {
        scope_root,
        component_id: component_id.to_string(),
        param_key: param_key.to_string(),
        path: path.to_string(),
        baseline_parameter,
    })
}

pub fn set_parameter(request: SetParameterRequest) -> SetParameterResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let path = request.path.clone();
    let operation_now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return set_parameter_failed(
            model_hash,
            resolve_hash,
            scope,
            path,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set set_parameter.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, operation_now_unix_ms) {
        return set_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return set_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }

    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, operation_now_unix_ms) {
        return set_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }

    let validated = match validate_runtime_write(
        &snapshot,
        &request.path,
        &request.value,
        "request.path",
        true,
    ) {
        Ok(value) => value,
        Err(diagnostic) => {
            return set_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
        }
    };
    let scope_root = validated.scope_root;
    let component_id = validated.component_id;
    let param_key = validated.param_key;
    let baseline_parameter = validated.baseline_parameter;

    let previous_effective_value =
        effective_parameter_value(&snapshot, &scope_root, &request.path, &baseline_parameter.value);
    let requested_value = request.value;
    // Operator identity/justification for this write (configflux-irid). When the
    // caller omits `actor`, fall back to the default dirty actor so the emitted
    // events and audit entry keep their prior identity. `reason` is sanitized by
    // `apply_dirty_write`; sanitize the event/audit copy the same way so an empty
    // reason is treated as absent consistently.
    let write_actor = request
        .actor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_DIRTY_ACTOR)
        .to_string();
    let write_reason = sanitize_optional_text(request.reason.clone());
    let dirty_metadata = apply_dirty_write(
        &mut snapshot,
        &scope_root,
        &request.path,
        requested_value,
        operation_now_unix_ms,
        &write_actor,
        write_reason.as_deref(),
        request.intent,
    );
    let updated_effective_value =
        effective_parameter_value(&snapshot, &scope_root, &request.path, &baseline_parameter.value);

    emit_runtime_event(
        &mut snapshot,
        RuntimeEventKind::ParameterChanged,
        operation_now_unix_ms,
        Some(&write_actor),
        write_reason.as_deref(),
        Some(&previous_effective_value),
        Some(&updated_effective_value),
        RuntimeEventPayload::ParameterChanged {
            scope_root: scope_root.clone(),
            path: request.path.clone(),
            generation: dirty_metadata.generation,
        },
    );
    emit_runtime_event(
        &mut snapshot,
        RuntimeEventKind::DirtyStateChanged,
        operation_now_unix_ms,
        Some(&write_actor),
        write_reason.as_deref(),
        None,
        None,
        RuntimeEventPayload::DirtyStateChanged {
            scope_root: scope_root.clone(),
            path: request.path.clone(),
            dirty: true,
            generation: dirty_metadata.generation,
        },
    );
    let identity = match compute_configuration_identity(&snapshot) {
        Ok(value) => value,
        Err(diagnostic) => {
            return set_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
        }
    };
    append_audit_event(
        &mut snapshot,
        RuntimeAuditEventKind::Write,
        operation_now_unix_ms,
        &write_actor,
        write_reason.as_deref(),
        vec![canonical_runtime_path(&scope_root, &request.path)],
        &identity,
        None,
        None,
    );

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return set_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }

    let parameter = match parameter_payload(
        &component_id,
        &param_key,
        &effective_parameter(&baseline_parameter, &snapshot, &scope_root, &request.path),
        &snapshot.resolved_artifacts,
    ) {
        Ok(parameter) => parameter,
        Err(diagnostic) => {
            return set_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
        }
    };

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };

    SetParameterResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        path: request.path,
        runtime_snapshot: Some(snapshot),
        parameter: Some(parameter),
        unsat_core: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn set_parameters_atomically(
    request: SetParametersAtomicallyRequest,
) -> SetParametersAtomicallyResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return set_parameters_atomically_failed(
            model_hash,
            resolve_hash,
            scope,
            Vec::new(),
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set set_parameters_atomically.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    let actor = request.actor.trim().to_string();
    if actor.is_empty() {
        return set_parameters_atomically_failed(
            model_hash,
            resolve_hash,
            scope,
            Vec::new(),
            vec![Diagnostic {
                code: E_RUNTIME_DIRTY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "set_parameters_atomically.actor must be non-empty".to_string(),
                source_id: None,
                entity_path: Some("request.actor".to_string()),
                hint: Some("Provide actor identity for atomic write operations".to_string()),
            }],
        );
    }

    let reason = sanitize_optional_text(request.reason);
    let intent = request.intent;
    let writes = request.writes;
    if writes.is_empty() {
        return set_parameters_atomically_failed(
            model_hash,
            resolve_hash,
            scope,
            Vec::new(),
            vec![Diagnostic {
                code: E_RUNTIME_DIRTY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "set_parameters_atomically.writes must be non-empty".to_string(),
                source_id: None,
                entity_path: Some("request.writes".to_string()),
                hint: Some("Provide one or more writes for atomic mutation".to_string()),
            }],
        );
    }

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return set_parameters_atomically_failed(
            model_hash,
            resolve_hash,
            scope,
            Vec::new(),
            vec![diagnostic],
        );
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return set_parameters_atomically_failed(
            model_hash,
            resolve_hash,
            scope,
            Vec::new(),
            vec![diagnostic],
        );
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return set_parameters_atomically_failed(
            model_hash,
            resolve_hash,
            scope,
            Vec::new(),
            vec![diagnostic],
        );
    }

    let identity_before = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return set_parameters_atomically_failed(
                model_hash,
                resolve_hash,
                scope,
                Vec::new(),
                vec![diagnostic],
            );
        }
    };
    if let Some(expected_working_configuration_id) = request
        .expected_working_configuration_id
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        if expected_working_configuration_id != identity_before.working_configuration_id {
            return set_parameters_atomically_failed(
                model_hash,
                resolve_hash,
                scope,
                Vec::new(),
                vec![Diagnostic {
                    code: E_RUNTIME_DIRTY_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "working configuration mismatch (expected '{}', current '{}')",
                        expected_working_configuration_id, identity_before.working_configuration_id
                    ),
                    source_id: None,
                    entity_path: Some("request.expected_working_configuration_id".to_string()),
                    hint: Some("Refresh runtime identity and retry atomic set".to_string()),
                }],
            );
        }
    }

    let mut validated_writes = Vec::with_capacity(writes.len());
    for write in writes {
        let validated =
            match validate_runtime_write(&snapshot, &write.path, &write.value, "request.writes", true) {
                Ok(value) => value,
                Err(diagnostic) => {
                    return set_parameters_atomically_failed(
                        model_hash,
                        resolve_hash,
                        scope,
                        vec![write.path],
                        vec![diagnostic],
                    );
                }
            };
        validated_writes.push((validated, write.value));
    }

    let applied_count = validated_writes.len() as u32;
    let mut dirty_generation_max = 0_u64;
    let mut changed_paths = Vec::new();
    for (validated, value) in validated_writes {
        let previous_effective_value = effective_parameter_value(
            &snapshot,
            &validated.scope_root,
            &validated.path,
            &validated.baseline_parameter.value,
        );
        let dirty_metadata = apply_dirty_write(
            &mut snapshot,
            &validated.scope_root,
            &validated.path,
            value,
            now_unix_ms,
            &actor,
            reason.as_deref(),
            intent,
        );
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
                scope_root: validated.scope_root.clone(),
                path: validated.path.clone(),
                generation: dirty_metadata.generation,
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
                scope_root: validated.scope_root.clone(),
                path: validated.path.clone(),
                dirty: true,
                generation: dirty_metadata.generation,
            },
        );
        dirty_generation_max = dirty_generation_max.max(dirty_metadata.generation);
        changed_paths.push(canonical_runtime_path(&validated.scope_root, &validated.path));
    }
    changed_paths.sort();
    changed_paths.dedup();

    let identity_after = match compute_configuration_identity(&snapshot) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            return set_parameters_atomically_failed(
                model_hash,
                resolve_hash,
                scope,
                Vec::new(),
                vec![diagnostic],
            );
        }
    };
    append_audit_event(
        &mut snapshot,
        RuntimeAuditEventKind::Write,
        now_unix_ms,
        &actor,
        reason.as_deref(),
        changed_paths,
        &identity_after,
        None,
        None,
    );

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return set_parameters_atomically_failed(
            model_hash,
            resolve_hash,
            scope,
            Vec::new(),
            vec![diagnostic],
        );
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    SetParametersAtomicallyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: Some(snapshot),
        applied_count,
        rejected_paths: Vec::new(),
        dirty_generation_max,
        unsat_core: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn list_dirty_parameters(request: ListDirtyParametersRequest) -> ListDirtyParametersResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let scope_root = request.scope_root.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return list_dirty_parameters_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set list_dirty_parameters.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return list_dirty_parameters_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![diagnostic],
        );
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return list_dirty_parameters_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![diagnostic],
        );
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return list_dirty_parameters_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![diagnostic],
        );
    }

    let normalized_scope_root = match normalize_scope_root(&request.scope_root) {
        Some(scope_root) => scope_root,
        None => {
            return list_dirty_parameters_failed(
                model_hash,
                resolve_hash,
                scope,
                scope_root,
                vec![Diagnostic {
                    code: E_RUNTIME_UNKNOWN_SCOPE.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!("Unknown scope root '{}'", request.scope_root),
                    source_id: None,
                    entity_path: Some("request.scope_root".to_string()),
                    hint: Some(
                        "Use a scope root present in runtime_snapshot.resolved_output".to_string(),
                    ),
                }],
            );
        }
    };
    if !snapshot.resolved_output.contains_key(&normalized_scope_root) {
        return list_dirty_parameters_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![Diagnostic {
                code: E_RUNTIME_UNKNOWN_SCOPE.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Scope root '{}' is not present in runtime snapshot",
                    normalized_scope_root
                ),
                source_id: None,
                entity_path: Some("request.scope_root".to_string()),
                hint: Some("Use a scope root that exists in runtime_snapshot".to_string()),
            }],
        );
    }

    let mut dirty_paths: Vec<String> = collect_dirty_path_selections(&snapshot)
        .into_iter()
        .filter(|entry| entry.scope_root == normalized_scope_root)
        .map(|entry| entry.canonical_path)
        .collect();
    dirty_paths.sort();
    dirty_paths.dedup();

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    ListDirtyParametersResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        scope_root: normalized_scope_root,
        dirty_count: dirty_paths.len() as u32,
        dirty_paths,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn get_dirty_metadata(request: GetDirtyMetadataRequest) -> GetDirtyMetadataResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let path = request.path.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return get_dirty_metadata_failed(
            model_hash,
            resolve_hash,
            scope,
            path,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set get_dirty_metadata.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return get_dirty_metadata_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return get_dirty_metadata_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return get_dirty_metadata_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }

    let resolved = match resolve_runtime_path(&snapshot, &request.path, "request.path") {
        Ok(value) => value,
        Err(diagnostic) => {
            return get_dirty_metadata_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
        }
    };
    let metadata = snapshot
        .dirty_metadata
        .get(&resolved.scope_root)
        .and_then(|entries| entries.get(&resolved.path))
        .cloned();
    let dirty = metadata.is_some();

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    GetDirtyMetadataResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        path: resolved.canonical_path,
        dirty,
        scope_root: Some(resolved.scope_root),
        metadata,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn set_auto_reset_policy(request: SetAutoResetPolicyRequest) -> SetAutoResetPolicyResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let now_unix_ms = current_time_unix_ms();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return set_auto_reset_policy_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set set_auto_reset_policy.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }
    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return set_auto_reset_policy_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return set_auto_reset_policy_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, now_unix_ms) {
        return set_auto_reset_policy_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let mut policy = request.auto_reset_policy;
    if policy.default_timeout_ms == 0 {
        return set_auto_reset_policy_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_DIRTY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "auto_reset_policy.default_timeout_ms must be >= 1".to_string(),
                source_id: None,
                entity_path: Some("request.auto_reset_policy.default_timeout_ms".to_string()),
                hint: Some("Use timeout_ms >= 1 for reset policy".to_string()),
            }],
        );
    }
    policy.policy_revision = snapshot.auto_reset_policy.policy_revision.saturating_add(1);
    snapshot.auto_reset_policy = policy.clone();

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, now_unix_ms) {
        return set_auto_reset_policy_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return set_auto_reset_policy_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    SetAutoResetPolicyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        runtime_snapshot: Some(snapshot),
        auto_reset_policy: Some(policy),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn get_auto_reset_policy(request: GetAutoResetPolicyRequest) -> GetAutoResetPolicyResult {
    let snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return get_auto_reset_policy_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set get_auto_reset_policy.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }
    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return get_auto_reset_policy_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    GetAutoResetPolicyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        auto_reset_policy: Some(snapshot.auto_reset_policy),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

