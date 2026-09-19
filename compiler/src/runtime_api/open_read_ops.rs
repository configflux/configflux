// SPDX-License-Identifier: BUSL-1.1

pub fn runtime_open(request: RuntimeOpenRequest) -> RuntimeOpenResult {
    let model_hash = request.model_hash.clone();
    let resolve_hash = request.resolve_hash.clone();
    let scope = request.scope.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return runtime_open_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set runtime_open_request.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if !is_sha256_hex(&request.model_hash) {
        return runtime_open_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_OPEN_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_open_request.model_hash must be a 64-char lowercase sha256 hex string"
                    .to_string(),
                source_id: None,
                entity_path: Some("request.model_hash".to_string()),
                hint: Some("Use model_hash from resolve_result/model manifest".to_string()),
            }],
        );
    }
    if !is_sha256_hex(&request.resolve_hash) {
        return runtime_open_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_OPEN_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "runtime_open_request.resolve_hash must be a 64-char lowercase sha256 hex string"
                    .to_string(),
                source_id: None,
                entity_path: Some("request.resolve_hash".to_string()),
                hint: Some("Use resolve_hash from resolve_result".to_string()),
            }],
        );
    }

    let scope_root = match normalize_scope_root(&request.scope) {
        Some(scope_root) => scope_root,
        None => {
            return runtime_open_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_OPEN_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Unsupported scope '{}' (expected component:<component_id>)",
                        request.scope
                    ),
                    source_id: None,
                    entity_path: Some("request.scope".to_string()),
                    hint: Some("Use scope format component:<component_id>".to_string()),
                }],
            );
        }
    };

    let resolved_output: BTreeMap<String, crate::resolved_models::ResolvedConfig> =
        match serde_json::from_value(request.resolved_output.clone()) {
            Ok(value) => value,
            Err(err) => {
                return runtime_open_failed(
                    model_hash,
                    resolve_hash,
                    scope,
                    vec![Diagnostic {
                        code: E_RUNTIME_OPEN_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("request.resolved_output cannot be decoded: {err}"),
                        source_id: None,
                        entity_path: Some("request.resolved_output".to_string()),
                        hint: Some(
                            "Use resolved_output emitted by resolve_from_selection".to_string(),
                        ),
                    }],
                );
            }
        };

    if !resolved_output.contains_key(&scope_root) {
        return runtime_open_failed(
            model_hash,
            resolve_hash,
            scope,
            vec![Diagnostic {
                code: E_RUNTIME_UNKNOWN_SCOPE.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Scope root '{}' is not present in runtime snapshot",
                    scope_root
                ),
                source_id: None,
                entity_path: Some("request.scope".to_string()),
                hint: Some("Use a scope root that exists in resolved_output".to_string()),
            }],
        );
    }

    if let Err(diagnostic) = validate_resolved_component_dependencies(
        &resolved_output,
        &request.resolved_component_dependencies,
    ) {
        return runtime_open_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    if let Err(diagnostic) =
        validate_artifact_references(&resolved_output, &request.resolved_artifacts)
    {
        return runtime_open_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    // Cross-validation is UNCONDITIONAL (configflux-zr6m). It used to run only
    // when at least one of the four provenance maps was non-empty, which left
    // the all-empty snapshot unchecked: `request.resolve_hash` was never
    // recomputed, so a caller could bind a pre-existing legitimate hash to a
    // DIFFERENT `resolved_output` simply by stripping every map. That is
    // provenance FORGERY rather than privilege escalation — the recompute is
    // keyless over a public recipe, so it was never an authenticity check — but
    // `resolve_hash` is consumed downstream as an identity token, so a snapshot
    // that opens carrying a hash it cannot account for is exactly the thing
    // this guard exists to refuse. A snapshot with genuine `implied_choices`
    // could be DOWNGRADED the same way, by zeroing all four maps at once.
    //
    // Recomputing over all-empty provenance is safe, and that is the whole
    // reason the fast path could go: the recipe skip-serializes
    // `defaulted_choices` and `implied_choices` when empty (ADR-0047 §5,
    // ADR-0057 §D6 — the rule that keeps facet-free models byte-identical to
    // their pre-feature pre-image), the loader proves its inner selection_state
    // schema_version/model_hash/scope equal the outer ones this side substitutes
    // (`validate_selection_state`), and `resolved_output` is canonicalized on
    // the way in through an idempotent pass. So a legitimate provenance-free
    // snapshot reproduces its own hash exactly. The cost is one SHA-256 over the
    // resolved output.
    //
    // configflux-y2ai: the pre-image and the hashing now live in ONE place
    // (`crate::resolve_hash`), reached from here through the local
    // `compute_resolve_hash` adapter and from the loader through its own. What
    // this check establishes is unchanged and was always the real property: the
    // caller forwarded the same six inputs the loader hashed. It no longer also
    // depends on two transcribed copies of the recipe having stayed identical.
    match compute_resolve_hash(
        &request.model_hash,
        &request.scope,
        &request.context_tags,
        &request.choices,
        &request.resolved_output,
        &request.defaulted_choices,
        &request.implied_choices,
    ) {
        Ok(computed_hash) if computed_hash != request.resolve_hash => {
            return runtime_open_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_HASH_MISMATCH.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "resolve_hash mismatch (expected '{}', got '{}')",
                        computed_hash, request.resolve_hash
                    ),
                    source_id: None,
                    entity_path: Some("request.resolve_hash".to_string()),
                    // Built from the ONE list that also defines the pre-image,
                    // so the remediation can never again name a narrower set
                    // than the hash actually covers — the drift
                    // configflux-j2jj records, where this hint kept naming only
                    // context_tags and choices after ADR-0047 §5 had added
                    // defaulted_choices to the recipe. The const moved next to
                    // the pre-image in configflux-y2ai; its VALUE is unchanged,
                    // so this hint's text is byte-identical to what it was.
                    hint: Some(format!(
                        "Pass {} from resolve_result unmodified, or use a matching resolve_hash",
                        crate::resolve_hash::RESOLVE_HASH_SELECTION_FIELDS
                    )),
                }],
            );
        }
        Ok(_) => {}
        Err(err) => {
            return runtime_open_failed(
                model_hash,
                resolve_hash,
                scope,
                vec![Diagnostic {
                    code: E_RUNTIME_OPEN_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!("Failed to canonicalize resolve hash for runtime_open: {err}"),
                    source_id: None,
                    entity_path: Some("request.resolve_hash".to_string()),
                    hint: Some(
                        "Use deterministic resolve payloads for runtime handoff".to_string(),
                    ),
                }],
            );
        }
    }

    let opened_at_unix_ms = current_time_unix_ms();
    let mut snapshot = RuntimeSnapshot {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash: request.model_hash,
        // configflux-9hi2: surface the .ccm path on the runtime open envelope
        // so g3f.3 can load it later. Surfacing only — no runtime constraint
        // wiring consumes it yet.
        ccm_ref: request.ccm_ref,
        resolve_hash: request.resolve_hash,
        scope: request.scope,
        context_tags: request.context_tags,
        choices: request.choices,
        // ADR-0060 D3: copied VERBATIM, with no derivation and no fabrication —
        // exactly as `ccm_ref` is. The runtime never guesses a domain from the
        // symbol table and never infers closed-ness; validation of a SUPPLIED
        // table is D6's and lives in the runtime crate, because the compiler may
        // not import `solver` (ADR-0003 §2).
        closed_facet_domains: request.closed_facet_domains,
        resolved_output,
        resolved_component_dependencies: request.resolved_component_dependencies,
        resolved_artifacts: request.resolved_artifacts,
        committed_overlay: request.committed_overlay,
        dirty_overlay: request.dirty_overlay,
        dirty_generations: request.dirty_generations,
        dirty_metadata: request.dirty_metadata,
        auto_reset_policy: request.auto_reset_policy,
        auto_reset_scheduler: request.auto_reset_scheduler,
        event_bus: request.event_bus,
        sync_status: request.sync_status,
        audit_events: request.audit_events,
        audit_next_sequence: request.audit_next_sequence,
        audit_uploaded_sequence: request.audit_uploaded_sequence,
        persistence_format_version: request.persistence_format_version,
        persistence_journal_sequence: request.persistence_journal_sequence,
    };

    if let Err(diagnostic) = normalize_runtime_snapshot_state(&mut snapshot, opened_at_unix_ms) {
        return runtime_open_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return runtime_open_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    if let Err(diagnostic) = apply_due_auto_resets(&mut snapshot, opened_at_unix_ms) {
        return runtime_open_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    emit_runtime_event(
        &mut snapshot,
        RuntimeEventKind::RuntimeOpened,
        opened_at_unix_ms,
        None,
        None,
        None,
        None,
        RuntimeEventPayload::RuntimeOpened {
            scope_root: scope_root.clone(),
        },
    );

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return runtime_open_failed(model_hash, resolve_hash, scope, vec![diagnostic]);
    }

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };

    RuntimeOpenResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash: snapshot.model_hash.clone(),
        resolve_hash: snapshot.resolve_hash.clone(),
        scope: snapshot.scope.clone(),
        runtime_snapshot: Some(snapshot),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// The `resolve_hash` [`runtime_open`] will recompute for `request`, i.e. the
/// only value its cross-validation accepts.
///
/// Exists because that cross-validation became unconditional (configflux-zr6m):
/// a caller that ASSEMBLES a `RuntimeOpenRequest` rather than forwarding one
/// the loader produced can no longer present an arbitrary hash, and the recipe
/// it must satisfy is crate-private. Exposing it concedes nothing — the recipe
/// is keyless and public (ADR-0047 §5, ADR-0057 §D6), so the check is a
/// consistency check and never an authenticity one, and anyone able to call this
/// could already reimplement it from the ADRs.
///
/// It is deliberately NOT a way to make a mismatched payload open: the hash it
/// returns is a function of the payload, so a caller that substitutes the
/// resolved output gets a different hash rather than a free pass. The honest
/// use is the reverse — assemble the request, then stamp the hash it earns.
///
/// Callers holding a real `ResolveResult` should keep forwarding
/// `resolve_result.resolve_hash` unmodified. Going through this function instead
/// would recompute what the loader already computed and, worse, would make the
/// open's check vacuous for that caller: the hash would be derived from the very
/// payload it is supposed to vouch for, so a payload edited after the resolve
/// would sail through. Forwarding the loader's hash is what makes the check able
/// to say anything at all.
///
/// `Err` only where the payload cannot be canonically serialized, the same
/// condition that raises `E_RUNTIME_OPEN_INVALID` inside the open.
pub fn expected_resolve_hash(request: &RuntimeOpenRequest) -> Result<String> {
    compute_resolve_hash(
        &request.model_hash,
        &request.scope,
        &request.context_tags,
        &request.choices,
        &request.resolved_output,
        &request.defaulted_choices,
        &request.implied_choices,
    )
}

pub fn get_scope_metadata(request: GetScopeMetadataRequest) -> GetScopeMetadataResult {
    let mut snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let scope_root = request.scope_root.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return get_scope_metadata_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set get_scope_metadata.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return get_scope_metadata_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![diagnostic],
        );
    }

    let normalized_scope_root = match normalize_scope_root(&request.scope_root) {
        Some(value) => value,
        None => {
            return get_scope_metadata_failed(
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

    let Some(resolved_scope) = snapshot.resolved_output.remove(&normalized_scope_root) else {
        return get_scope_metadata_failed(
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
                hint: Some("Use list of available scope roots from runtime snapshot".to_string()),
            }],
        );
    };

    let mut artifact_ids = BTreeSet::new();
    let mut parameter_count = 0_u32;
    for (component_id, component) in &resolved_scope.components {
        parameter_count += component.params.len() as u32;
        for (param_key, parameter) in &component.params {
            if parameter.r#type == "artifact" {
                let path = format!("component.{component_id}.param.{param_key}");
                let effective_value =
                    effective_parameter_value(&snapshot, &normalized_scope_root, &path, &parameter.value);
                if let crate::schema::Value::String(artifact_id) = effective_value {
                    let trimmed = artifact_id.trim();
                    if !trimmed.is_empty() {
                        artifact_ids.insert(trimmed.to_string());
                    }
                }
            }
        }
    }

    let metadata = ScopeMetadata {
        component_count: resolved_scope.components.len() as u32,
        parameter_count,
        artifact_count: artifact_ids.len() as u32,
    };

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    GetScopeMetadataResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        scope_root: normalized_scope_root,
        metadata: Some(metadata),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn list_parameters(request: ListParametersRequest) -> ListParametersResult {
    let snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let scope_root = request.scope_root.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return list_parameters_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set list_parameters.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return list_parameters_failed(
            model_hash,
            resolve_hash,
            scope,
            scope_root,
            vec![diagnostic],
        );
    }

    let normalized_scope_root = match normalize_scope_root(&request.scope_root) {
        Some(value) => value,
        None => {
            return list_parameters_failed(
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

    let Some(resolved_scope) = snapshot.resolved_output.get(&normalized_scope_root) else {
        return list_parameters_failed(
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
                hint: Some(
                    "Use a scope root present in runtime_snapshot.resolved_output".to_string(),
                ),
            }],
        );
    };

    let parameter_paths = sorted_parameter_paths(resolved_scope);
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };

    ListParametersResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        scope_root: normalized_scope_root,
        parameter_paths,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

pub fn get_parameter(request: GetParameterRequest) -> GetParameterResult {
    let snapshot = request.runtime_snapshot;
    let model_hash = snapshot.model_hash.clone();
    let resolve_hash = snapshot.resolve_hash.clone();
    let scope = snapshot.scope.clone();
    let path = request.path.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return get_parameter_failed(
            model_hash,
            resolve_hash,
            scope,
            path,
            vec![schema_version_diagnostic(
                E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
                request.schema_version,
                "request.schema_version",
                &format!("Set get_parameter.schema_version to {}", PRODUCT_SCHEMA_VERSION),
            )],
        );
    }

    if let Err(diagnostic) = validate_runtime_snapshot(&snapshot) {
        return get_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
    }

    let parameter = match find_parameter_payload(&snapshot, &request.path) {
        Ok(parameter) => parameter,
        Err(diagnostic) => {
            return get_parameter_failed(model_hash, resolve_hash, scope, path, vec![diagnostic]);
        }
    };

    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };

    GetParameterResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        resolve_hash,
        scope,
        path: request.path,
        parameter: Some(parameter),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

