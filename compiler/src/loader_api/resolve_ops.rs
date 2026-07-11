// SPDX-License-Identifier: BUSL-1.1

pub fn resolve_from_selection(request: ResolveFromSelectionRequest) -> ResolveResult {
    let model_hash = request.model_handle.model_hash.clone();
    let scope = request.scope.clone();
    let selection_state_hash = request.selection_state.selection_state_hash.clone();
    let context_tags = request.selection_state.context_tags.clone();
    let choices = request.selection_state.choices.clone();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return resolve_failed(
            model_hash,
            scope,
            selection_state_hash,
            context_tags,
            choices,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
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
            return resolve_failed(
                model_hash,
                scope,
                selection_state_hash,
                context_tags,
                choices,
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![diagnostic],
            );
        }
    };

    let model = match load_resolve_model(&request.model_handle) {
        Ok(model) => model,
        Err(err) => {
            return resolve_failed(
                model_hash,
                scope,
                selection_state_hash,
                context_tags,
                choices,
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![Diagnostic {
                    code: E_RESOLVE_MODEL_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: Some(request.model_handle.index_ref),
                    entity_path: None,
                    hint: Some("Re-open a valid CMP model handle before resolve".to_string()),
                }],
            );
        }
    };

    // ADR-0047 §5: auto-bind every declared facet that has a declared default,
    // UNCONDITIONALLY (even one no active condition reads). Precedence, highest
    // wins: explicit choice > context tag > declared default. `assignments` is
    // already context_tags overlaid with choices (`merge_assignments`), so we
    // seed the declared defaults FIRST and let `assignments` overlay them — the
    // "Missing tag" failure no longer fires for a defaulted facet. A declared
    // default that survives (its facet is absent from `assignments`) is recorded
    // in `defaulted_choices` as first-class provenance; `SelectionState` is left
    // untouched (its hash stays pure user input).
    let mut tags: HashMap<String, String> = HashMap::new();
    let mut defaulted_choices: BTreeMap<String, String> = BTreeMap::new();
    for (name, facet) in &model.facets {
        if let Some(default) = &facet.default {
            tags.insert(name.clone(), default.clone());
            if !assignments.contains_key(name) {
                defaulted_choices.insert(name.clone(), default.clone());
            }
        }
    }
    for (facet, option) in &assignments {
        tags.insert(facet.clone(), option.clone());
    }

    let context = ResolutionContext { tags };
    let resolved_scoped = match resolver::resolve_scoped(&model, &context, &request.scope) {
        Ok(resolved) => resolved,
        Err(err) => {
            return resolve_failed(
                model_hash,
                scope,
                selection_state_hash,
                context_tags,
                choices,
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![map_resolve_error_with_facets(err, &model.facets)],
            );
        }
    };

    let resolved_output = match canonicalize_resolved_output(&resolved_scoped) {
        Ok(value) => value,
        Err(err) => {
            return resolve_failed(
                model_hash,
                scope,
                selection_state_hash,
                context_tags,
                choices,
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![Diagnostic {
                    code: E_RESOLVE_FAILED.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: None,
                    entity_path: None,
                    hint: Some(
                        "Resolve output must be serializable for deterministic hashing".to_string(),
                    ),
                }],
            );
        }
    };

    let resolve_hash = match compute_resolve_hash(
        &request.model_handle.model_hash,
        &request.scope,
        &request.selection_state,
        &resolved_output,
        &defaulted_choices,
    ) {
        Ok(hash) => hash,
        Err(err) => {
            return resolve_failed(
                model_hash,
                scope,
                selection_state_hash,
                context_tags,
                choices,
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![Diagnostic {
                    code: E_RESOLVE_FAILED.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: None,
                    entity_path: None,
                    hint: Some("Resolve hash input must be canonically serializable".to_string()),
                }],
            );
        }
    };

    let resolved_component_dependencies =
        extract_resolved_component_dependencies(&model, &resolved_scoped);
    let resolved_artifacts = sorted_artifact_catalog(&model.artifacts);

    resolve_ok(
        request.model_handle.model_hash,
        request.scope,
        request.selection_state.selection_state_hash,
        request.selection_state.context_tags,
        request.selection_state.choices,
        defaulted_choices,
        resolved_component_dependencies,
        resolved_artifacts,
        resolve_hash,
        resolved_output,
    )
}

