// SPDX-License-Identifier: BUSL-1.1

pub fn initialize_selection_state(
    request: InitializeSelectionStateRequest,
) -> InitializeSelectionStateResult {
    let model_hash = request.model_handle.model_hash.clone();
    let scope = request.scope.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return initialize_selection_state_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_LOADER_UNSUPPORTED_SCHEMA_VERSION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Unsupported schema_version {} (expected {})",
                    request.schema_version, PRODUCT_SCHEMA_VERSION
                ),
                source_id: None,
                entity_path: None,
                hint: Some(format!("Set request.schema_version to {} for selection APIs", PRODUCT_SCHEMA_VERSION)),
            }],
        );
    }

    if scope.trim().is_empty() {
        return initialize_selection_state_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_SELECTION_STATE_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "Selection scope is empty".to_string(),
                source_id: None,
                entity_path: None,
                hint: Some("Provide a non-empty scope for selection operations".to_string()),
            }],
        );
    }

    if let Err(err) = load_selection_constraint_model(&request.model_handle) {
        return initialize_selection_state_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_LOADER_INDEX_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: err.to_string(),
                source_id: Some(request.model_handle.index_ref),
                entity_path: None,
                hint: Some(
                    "Re-open a valid CMP model handle before initializing selection state"
                        .to_string(),
                ),
            }],
        );
    }

    match canonical_selection_state(
        request.model_handle.model_hash,
        request.scope,
        request.context_tags,
        BTreeMap::new(),
    ) {
        Ok(selection_state) => initialize_selection_state_ok(model_hash, scope, selection_state),
        Err(err) => initialize_selection_state_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_SELECTION_STATE_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: err.to_string(),
                source_id: None,
                entity_path: None,
                hint: Some("Recreate selection_state using canonical hash utility".to_string()),
            }],
        ),
    }
}

pub fn canonical_selection_state(
    model_hash: impl Into<String>,
    scope: impl Into<String>,
    context_tags: BTreeMap<String, String>,
    choices: BTreeMap<String, String>,
) -> Result<SelectionState> {
    let mut state = SelectionState {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash: model_hash.into(),
        scope: scope.into(),
        context_tags,
        choices,
        selection_state_hash: String::new(),
    };
    state.selection_state_hash = compute_selection_state_hash(&state)?;
    Ok(state)
}

pub fn compute_selection_state_hash(state: &SelectionState) -> Result<String> {
    let canonical = SelectionStateCanonical {
        schema_version: state.schema_version,
        model_hash: &state.model_hash,
        scope: &state.scope,
        context_tags: &state.context_tags,
        choices: &state.choices,
    };
    let bytes = serde_json::to_vec(&canonical).context("Failed to canonicalize selection state")?;
    Ok(sha256_hex(&bytes))
}

/// List every selection facet the model exposes, in sorted order.
///
/// The facet universe is exactly the set `get_selection_options`/`apply_selection`
/// validate against — the facets that appear in the model's typed conditions
/// (`register_facet_domains`). This is a thin, read-only accessor over the SAME
/// `load_selection_constraint_model` the per-facet ops use; it composes no
/// selection/resolution semantics of its own. `cfx options` (ADR-0042) needs the
/// facet list to iterate `get_selection_options` per facet, and there is no other
/// public way to enumerate facets (the per-facet ops each take a facet name).
///
/// Returns the `facet_domains` keys (already `BTreeMap`-sorted) so the caller's
/// listing is deterministic. Loader/integrity failures surface as an `Err`.
pub fn list_selection_facets(model_handle: &ModelHandle) -> Result<Vec<String>> {
    let model = load_selection_constraint_model(model_handle)?;
    Ok(model.facet_domains.keys().cloned().collect())
}

pub fn get_selection_options(request: GetSelectionOptionsRequest) -> GetSelectionOptionsResult {
    let model_hash = request.model_handle.model_hash.clone();
    let scope = request.scope.clone();
    let facet = request.facet.clone();
    let selection_state_hash = request.selection_state.selection_state_hash.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return selection_options_failed(
            model_hash,
            scope,
            facet,
            selection_state_hash,
            vec![Diagnostic {
                code: E_LOADER_UNSUPPORTED_SCHEMA_VERSION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Unsupported schema_version {} (expected {})",
                    request.schema_version, PRODUCT_SCHEMA_VERSION
                ),
                source_id: None,
                entity_path: None,
                hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
            }],
        );
    }

    let assignments = match validate_selection_state(
        &request.model_handle,
        &request.scope,
        &request.selection_state,
    ) {
        Ok(assignments) => assignments,
        Err(diagnostic) => {
            return selection_options_failed(
                model_hash,
                scope,
                facet,
                selection_state_hash,
                vec![diagnostic],
            );
        }
    };

    if request.facet.trim().is_empty() {
        return selection_options_failed(
            model_hash,
            scope,
            facet,
            selection_state_hash,
            vec![Diagnostic {
                code: E_SELECTION_UNKNOWN_FACET.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "Selection facet is empty".to_string(),
                source_id: None,
                entity_path: None,
                hint: Some("Provide a non-empty facet ID".to_string()),
            }],
        );
    }

    let model = match load_selection_constraint_model(&request.model_handle) {
        Ok(model) => model,
        Err(err) => {
            return selection_options_failed(
                model_hash,
                scope,
                facet,
                selection_state_hash,
                vec![Diagnostic {
                    code: E_LOADER_INDEX_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: Some(request.model_handle.index_ref),
                    entity_path: None,
                    hint: Some("Re-open a valid CMP model handle before selection".to_string()),
                }],
            );
        }
    };

    let domain = match facet_domain(&model, &assignments, &request.facet) {
        Some(domain) => domain,
        None => {
            return selection_options_failed(
                model_hash,
                scope,
                facet,
                selection_state_hash,
                vec![Diagnostic {
                    code: E_SELECTION_UNKNOWN_FACET.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!("Unknown selection facet '{}'", request.facet),
                    source_id: None,
                    entity_path: None,
                    hint: Some(
                        "Use get_selection_options on a known facet from model conditions"
                            .to_string(),
                    ),
                }],
            );
        }
    };

    let valid_options = valid_options_for_facet(&model, &assignments, &request.facet, &domain);
    let pruned_options = if request.include_pruned_reasons {
        let valid_set: BTreeSet<String> = valid_options.iter().cloned().collect();
        let mut pruned = Vec::new();
        for option in &domain {
            if valid_set.contains(option) {
                continue;
            }
            pruned.push(PrunedOptionReason {
                option: option.clone(),
                reason: format!(
                    "No satisfiable condition branch remains for facet '{}' option '{}' under current selection",
                    request.facet, option
                ),
            });
        }
        Some(pruned)
    } else {
        None
    };

    // ADR-0047 §6: annotate the facet's declared default arm, when it has one.
    // `facet_defaults` is empty for undeclared / default-less facets, so this is
    // `None` for every pre-ADR-0047 model (skip-if-none keeps them byte-stable).
    let default = model.facet_defaults.get(&request.facet).cloned();
    // ADR-0047 §6 (Amendment 1): the declared domain-openness, `None` for an
    // undeclared facet so its render label and JSON stay byte-identical.
    let declared_open = model.facet_open.get(&request.facet).copied();

    selection_options_ok(
        request.model_handle.model_hash,
        request.scope,
        request.facet,
        valid_options,
        default,
        declared_open,
        pruned_options,
        request.selection_state.selection_state_hash,
    )
}

pub fn apply_selection(request: ApplySelectionRequest) -> ApplySelectionResult {
    let model_hash = request.model_handle.model_hash.clone();
    let scope = request.scope.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return apply_selection_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_LOADER_UNSUPPORTED_SCHEMA_VERSION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Unsupported schema_version {} (expected {})",
                    request.schema_version, PRODUCT_SCHEMA_VERSION
                ),
                source_id: None,
                entity_path: None,
                hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
            }],
        );
    }

    if request.selection_delta.facet.trim().is_empty() {
        return apply_selection_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_SELECTION_UNKNOWN_FACET.to_string(),
                severity: DiagnosticSeverity::Error,
                message: "Selection delta facet is empty".to_string(),
                source_id: None,
                entity_path: None,
                hint: Some("Provide a non-empty facet ID".to_string()),
            }],
        );
    }
    if request.selection_delta.option.trim().is_empty() {
        return apply_selection_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_SELECTION_INVALID_OPTION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Selection delta option is empty for facet '{}'",
                    request.selection_delta.facet
                ),
                source_id: None,
                entity_path: None,
                hint: Some("Provide a non-empty option value".to_string()),
            }],
        );
    }

    let assignments = match validate_selection_state(
        &request.model_handle,
        &request.scope,
        &request.selection_state,
    ) {
        Ok(assignments) => assignments,
        Err(diagnostic) => return apply_selection_failed(model_hash, scope, vec![diagnostic]),
    };

    if let Some(existing) = request
        .selection_state
        .context_tags
        .get(&request.selection_delta.facet)
    {
        if existing != &request.selection_delta.option {
            let mut blocking = BTreeMap::new();
            blocking.insert(request.selection_delta.facet.clone(), existing.clone());
            return apply_selection_failed(
                model_hash,
                scope,
                vec![rejection_to_diagnostic(
                    request.selection_delta.facet.clone(),
                    request.selection_delta.option.clone(),
                    RejectionReason {
                        code: E_SELECTION_CONFLICT.to_string(),
                        message: format!(
                            "Selection '{}'='{}' conflicts with immutable context tag value '{}'",
                            request.selection_delta.facet, request.selection_delta.option, existing
                        ),
                        blocking_choices: blocking,
                        hint: Some("Update context_tags or choose a compatible option".to_string()),
                        unsat_core: None,
                    },
                )],
            );
        }
    }

    if let Some(existing) = request
        .selection_state
        .choices
        .get(&request.selection_delta.facet)
    {
        if existing == &request.selection_delta.option {
            return apply_selection_ok(
                request.model_handle.model_hash,
                request.scope,
                request.selection_state,
            );
        }
        let mut blocking = BTreeMap::new();
        blocking.insert(request.selection_delta.facet.clone(), existing.clone());
        return apply_selection_failed(
            model_hash,
            scope,
            vec![rejection_to_diagnostic(
                request.selection_delta.facet.clone(),
                request.selection_delta.option.clone(),
                RejectionReason {
                    code: E_SELECTION_CONFLICT.to_string(),
                    message: format!(
                        "Facet '{}' is already selected as '{}'",
                        request.selection_delta.facet, existing
                    ),
                    blocking_choices: blocking,
                    hint: Some("Clear the existing choice or keep the current value".to_string()),
                    unsat_core: None,
                },
            )],
        );
    }

    let model = match load_selection_constraint_model(&request.model_handle) {
        Ok(model) => model,
        Err(err) => {
            return apply_selection_failed(
                model_hash,
                scope,
                vec![Diagnostic {
                    code: E_LOADER_INDEX_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: Some(request.model_handle.index_ref),
                    entity_path: None,
                    hint: Some("Re-open a valid CMP model handle before selection".to_string()),
                }],
            );
        }
    };

    let domain = match model.facet_domains.get(&request.selection_delta.facet) {
        Some(domain) => domain,
        None => {
            return apply_selection_failed(
                model_hash,
                scope,
                vec![Diagnostic {
                    code: E_SELECTION_UNKNOWN_FACET.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Unknown selection facet '{}'",
                        request.selection_delta.facet
                    ),
                    source_id: None,
                    entity_path: None,
                    hint: Some("Choose a facet discovered from model conditions".to_string()),
                }],
            );
        }
    };

    if !domain.contains(&request.selection_delta.option) {
        let mut options: Vec<String> = domain.iter().cloned().collect();
        options.sort();
        return apply_selection_failed(
            model_hash,
            scope,
            vec![Diagnostic {
                code: E_SELECTION_INVALID_OPTION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Invalid option '{}' for facet '{}'",
                    request.selection_delta.option, request.selection_delta.facet
                ),
                source_id: None,
                entity_path: None,
                hint: Some(format!("Valid options: {}", options.join(", "))),
            }],
        );
    }

    let valid_options =
        valid_options_for_facet(&model, &assignments, &request.selection_delta.facet, domain);
    if !valid_options.contains(&request.selection_delta.option) {
        let reason = build_unsat_reason(
            &request.selection_state,
            request.selection_delta.facet.clone(),
            request.selection_delta.option.clone(),
        );
        return apply_selection_failed(
            model_hash,
            scope,
            vec![rejection_to_diagnostic(
                request.selection_delta.facet,
                request.selection_delta.option,
                reason,
            )],
        );
    }

    let mut next_choices = request.selection_state.choices.clone();
    next_choices.insert(
        request.selection_delta.facet.clone(),
        request.selection_delta.option.clone(),
    );

    let next_state = match canonical_selection_state(
        request.selection_state.model_hash,
        request.selection_state.scope,
        request.selection_state.context_tags,
        next_choices,
    ) {
        Ok(state) => state,
        Err(err) => {
            return apply_selection_failed(
                model_hash,
                scope,
                vec![Diagnostic {
                    code: E_SELECTION_STATE_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: None,
                    entity_path: None,
                    hint: Some(
                        "Recreate selection_state from canonical payload fields".to_string(),
                    ),
                }],
            );
        }
    };

    apply_selection_ok(request.model_handle.model_hash, request.scope, next_state)
}

pub fn explain_rejection(request: ExplainRejectionRequest) -> ExplainRejectionResult {
    let model_hash = request.model_handle.model_hash.clone();
    let scope = request.scope.clone();
    let facet = request.rejected_option.facet.clone();
    let option = request.rejected_option.option.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return explain_rejection_failed(
            model_hash,
            scope,
            facet,
            option,
            RejectionReason {
                code: E_LOADER_UNSUPPORTED_SCHEMA_VERSION.to_string(),
                message: format!(
                    "Unsupported schema_version {} (expected {})",
                    request.schema_version, PRODUCT_SCHEMA_VERSION
                ),
                blocking_choices: BTreeMap::new(),
                hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
                unsat_core: None,
            },
        );
    }

    let assignments = match validate_selection_state(
        &request.model_handle,
        &request.scope,
        &request.selection_state,
    ) {
        Ok(assignments) => assignments,
        Err(diagnostic) => {
            return explain_rejection_failed(
                model_hash,
                scope,
                facet,
                option,
                RejectionReason {
                    code: diagnostic.code,
                    message: diagnostic.message,
                    blocking_choices: BTreeMap::new(),
                    hint: diagnostic.hint,
                    unsat_core: None,
                },
            );
        }
    };

    let model = match load_selection_constraint_model(&request.model_handle) {
        Ok(model) => model,
        Err(err) => {
            return explain_rejection_failed(
                model_hash,
                scope,
                facet,
                option,
                RejectionReason {
                    code: E_LOADER_INDEX_INVALID.to_string(),
                    message: err.to_string(),
                    blocking_choices: BTreeMap::new(),
                    hint: Some("Re-open a valid CMP model handle before selection".to_string()),
                    unsat_core: None,
                },
            );
        }
    };

    let reason = classify_rejection_reason(
        &model,
        &request.selection_state,
        &assignments,
        &request.rejected_option,
    );
    explain_rejection_failed(
        request.model_handle.model_hash,
        request.scope,
        request.rejected_option.facet,
        request.rejected_option.option,
        reason,
    )
}

