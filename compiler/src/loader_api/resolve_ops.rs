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
    // Built as a `BTreeMap` first: this IS the total post-default assignment
    // ADR-0054 §2 evaluates constraints against, and the constraint evaluator
    // (`not_contradicted`) reads a `BTreeMap`. `ResolutionContext` takes the
    // same pairs as a `HashMap` below — one assignment, two views, never two
    // constructions that could drift.
    let mut assignment: BTreeMap<String, String> = BTreeMap::new();
    let mut defaulted_choices: BTreeMap<String, String> = BTreeMap::new();
    for (name, facet) in &model.config.facets {
        if let Some(default) = &facet.default {
            assignment.insert(name.clone(), default.clone());
            if !assignments.contains_key(name) {
                defaulted_choices.insert(name.clone(), default.clone());
            }
        }
    }
    for (facet, option) in &assignments {
        assignment.insert(facet.clone(), option.clone());
    }

    // ADR-0054 §2/§6 (configflux-4sjk): FAIL CLOSED on a selection that violates
    // a declared policy. This is the resolve surface's half of "every declared
    // constraint must hold in every resolved configuration" — evaluated HERE,
    // after default auto-bind (ADR-0047 §5) and before `resolve_scoped`, because
    // this is the first and only point at which the assignment is TOTAL. Failing
    // before the resolver runs is what guarantees "no snapshot on rejection":
    // there is no resolved output to partially emit.
    match evaluate_constraints(&model, &assignment) {
        Ok(violations) if !violations.is_empty() => {
            return resolve_failed(
                model_hash,
                scope,
                selection_state_hash,
                context_tags,
                choices,
                // Carried, unlike every other resolve failure, because a policy
                // can be violated by a DEFAULT the user never typed; without
                // this the rejection would not show where the value came from.
                defaulted_choices,
                BTreeMap::new(),
                BTreeMap::new(),
                violations,
            );
        }
        Ok(_) => {}
        Err(diagnostic) => {
            return resolve_failed(
                model_hash,
                scope,
                selection_state_hash,
                context_tags,
                choices,
                defaulted_choices,
                BTreeMap::new(),
                BTreeMap::new(),
                vec![diagnostic],
            );
        }
    }

    let tags: HashMap<String, String> = assignment.into_iter().collect();
    let context = ResolutionContext { tags };
    let resolved_scoped = match resolver::resolve_scoped(&model.config, &context, &request.scope) {
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
                vec![map_resolve_error_with_facets(err, &model.config.facets)],
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
        extract_resolved_component_dependencies(&model.config, &resolved_scoped);
    let resolved_artifacts = sorted_artifact_catalog(&model.config.artifacts);

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

/// Evaluate every declared constraint against the total post-default
/// `assignment` (ADR-0054 §2), returning ONE ADR-0054 §6 diagnostic per
/// violated constraint in constraint-id-ascending order — or a single
/// fail-closed diagnostic if the package cannot be trusted.
///
/// An unparseable constraint is an `Err`, never a skip. `link_verify::
/// validate_constraints` proved every expression parses at ingest, so reaching
/// that branch means the package is corrupt or was written by an incompatible
/// toolchain. Dropping the policy there would fail OPEN — silently resolving a
/// configuration no one screened — which is precisely the failure mode
/// ADR-0054 exists to remove, so it is reported as an invalid resolve model.
fn evaluate_constraints(
    model: &ResolveModel,
    assignment: &BTreeMap<String, String>,
) -> std::result::Result<Vec<Diagnostic>, Diagnostic> {
    // Id-ascending, which `violated_constraint_ids` preserves and which §6
    // makes the emission order of the diagnostics vector.
    let mut parsed: Vec<(&str, ConditionExpr)> = Vec::with_capacity(model.config.constraints.len());
    let mut ids: Vec<&String> = model.config.constraints.keys().collect();
    ids.sort();
    for id in ids {
        let constraint = &model.config.constraints[id];
        let expr = parse_condition_expr(&constraint.condition).map_err(|err| Diagnostic {
            code: E_RESOLVE_MODEL_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Constraint '{}' has an unparseable expression '{}': {}",
                id, constraint.condition, err
            ),
            source_id: model.constraint_sources.get(id).cloned(),
            entity_path: Some(format!("{}{}", CONSTRAINT_ENTITY_PATH_PREFIX, id)),
            hint: Some("Recompile the model from sources the compiler accepts".to_string()),
        })?;
        parsed.push((id.as_str(), expr));
    }

    Ok(
        violated_constraint_ids(parsed.iter().map(|(id, expr)| (*id, expr)), assignment)
            .into_iter()
            .map(|id| {
                constraint_violation_diagnostic(
                    id,
                    &model.config.constraints[id].condition,
                    model.constraint_sources.get(id).map(String::as_str),
                )
            })
            .collect(),
    )
}

