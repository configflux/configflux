// SPDX-License-Identifier: BUSL-1.1

pub fn resolve_from_selection(request: ResolveFromSelectionRequest) -> ResolveResult {
    let model_hash = request.model_handle.model_hash.clone();
    let scope = request.scope.clone();
    let selection_state_hash = request.selection_state.selection_state_hash.clone();
    let context_tags = request.selection_state.context_tags.clone();
    let choices = request.selection_state.choices.clone();
    // ADR-0057 §D6: what the SOLVER determined the constraints already decide,
    // supplied by `session_compose::resolve`. Cloned up front like the two
    // fields above so every failure envelope below can carry it without
    // borrowing from a partially-moved `request`.
    let implied_choices = request.implied_choices.clone();

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
                BTreeMap::new(),
                vec![diagnostic],
            );
        }
    };

    // configflux-8nhr: the package, read ONCE for this whole call. The screen
    // below, the resolve model after it and the closed-facet table at the end
    // each used to read the index and open every chunk for themselves — three
    // index reads and six opens of every chunk file, all of them returning the
    // same bytes, since a package cannot change under one call.
    //
    // A failure here is a failure of the prefix BOTH loaders share
    // (`ir::load_index` and the integrity walk), so it is the case the screen
    // stays silent for: its own loader would refuse the package, its second
    // opinion `load_resolve_model` would refuse it too, and it would return no
    // diagnostics for `load_resolve_model` to then render the canonical refusal
    // below. That is the refusal rendered here, under the resolve funnel's
    // wording because `load_resolve_model` is the loader whose message reached
    // the envelope.
    let package = match load_package(&request.model_handle, RESOLVE_FUNNEL) {
        Ok(package) => package,
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
                BTreeMap::new(),
                vec![Diagnostic {
                    code: E_RESOLVE_MODEL_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: Some(request.model_handle.index_ref.clone()),
                    entity_path: None,
                    hint: Some("Re-open a valid CMP model handle before resolve".to_string()),
                }],
            );
        }
    };

    // ADR-0030 Amendment 2 Rule 1: the state's OWN assignments, screened
    // against the model. This one operation reads a `ResolveModel` rather than
    // the `SelectionConstraintModel` the screen reads, so it screens through
    // the shared package; a model that will not build reports nothing here and
    // the load below renders `E_RESOLVE_MODEL_INVALID` for itself.
    //
    // Ahead of the resolve model, in the same position `apply_selection` and
    // `get_selection_options` put it: an inadmissible state is not a resolve
    // request, and nothing computed from it could be worth reporting.
    let state_rejections = screen_selection_state_from(&package, &request.selection_state);
    if !state_rejections.is_empty() {
        return resolve_failed(
            model_hash,
            scope,
            selection_state_hash,
            context_tags,
            choices,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            state_rejections,
        );
    }

    let model = match load_resolve_model_from(&package) {
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

    // configflux-v93p: fail closed on an `implied_choices` entry the model does
    // not declare, or whose value falls outside that facet's declared domain —
    // BEFORE the seeding below puts it into the assignment `evaluate_constraints`
    // reads. Nothing has been computed at this point, so the envelope carries the
    // same empty provenance maps as the failure arms above; `implied_choices`
    // itself rides along because it is the input the caller has to fix.
    let implied_rejections = screen_implied_choices(&model, &implied_choices);
    if !implied_rejections.is_empty() {
        return resolve_failed(
            model_hash,
            scope,
            selection_state_hash,
            context_tags,
            choices,
            BTreeMap::new(),
            implied_choices.clone(),
            BTreeMap::new(),
            BTreeMap::new(),
            implied_rejections,
        );
    }

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
            // ADR-0057 §D6: a facet the solver IMPLIED is not defaulted — the
            // model had an opinion, so the default never applied. Recording it
            // in both maps would count one binding twice in the resolve-hash
            // pre-image and report two different origins for one value.
            if !assignments.contains_key(name) && !implied_choices.contains_key(name) {
                defaulted_choices.insert(name.clone(), default.clone());
            }
        }
    }
    // ADR-0057 §D6, the middle rung of the precedence ladder: implied overlays
    // the declared defaults seeded above and is itself overlaid by `assignments`
    // below, giving defaults < implied < context tags < choices.
    //
    // `assignments` is ALREADY context_tags merged with choices
    // (`merge_assignments`), which also raises E_SELECTION_CONFLICT on a
    // tag/choice clash. Re-deriving that precedence here would be a second
    // implementation of a rule that already has one.
    for (facet, option) in &implied_choices {
        assignment.insert(facet.clone(), option.clone());
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
                implied_choices.clone(),
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
                implied_choices.clone(),
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
        &implied_choices,
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

    // ADR-0059 D3: the payload's own identity, computed over the SAME
    // canonicalized `resolved_output` that is hashed above and stored below —
    // so the hash covers exactly the bytes the consumer receives. Its
    // pre-image excludes `model_hash` and the selection, which is what lets a
    // deployment check tell an unrelated model edit from a real change.
    let resolved_output_hash = match compute_resolved_output_hash(&request.scope, &resolved_output) {
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
                BTreeMap::new(),
                vec![Diagnostic {
                    code: E_RESOLVE_FAILED.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: None,
                    entity_path: None,
                    hint: Some(
                        "Resolved output hash input must be canonically serializable".to_string(),
                    ),
                }],
            );
        }
    };

    let resolved_component_dependencies =
        extract_resolved_component_dependencies(&model.config, &resolved_scoped);
    let resolved_artifacts = sorted_artifact_catalog(&model.config.artifacts);

    // ADR-0060 D8.3: the resolver is the last party in the chain that holds a
    // `ModelHandle`, so it is where the closed-facet table is computed. It is
    // built from the package this call already read (configflux-8nhr), so it
    // costs no read at all and brings in no new inputs — where D8.3 was written
    // this was "one read on an already-I/O-bound path".
    //
    // Fails SOFT, deliberately: attribution quality may not gate a resolve. A
    // failure here degrades the runtime to asserted-only attribution
    // (ADR-0060 D7) — an empty table is honest, and costs a constraint NAME on
    // a later diagnostic; failing the resolve would cost the whole deployment.
    let closed_facet_domains = closed_facet_domains_from(&package).unwrap_or_default();

    resolve_ok(
        request.model_handle.model_hash,
        request.scope,
        request.selection_state.selection_state_hash,
        request.selection_state.context_tags,
        request.selection_state.choices,
        defaulted_choices,
        implied_choices,
        closed_facet_domains,
        resolved_component_dependencies,
        resolved_artifacts,
        resolve_hash,
        resolved_output_hash,
        resolved_output,
    )
}

/// configflux-v93p: screen caller-supplied `implied_choices` against the model
/// before any of it reaches the constraint-evaluation assignment. One
/// diagnostic per offending entry; an empty vector means every entry is
/// admissible.
///
/// This was the last unscreened input to that assignment. `context_tags` and
/// `choices` come through `validate_selection_state`, which also binds them to
/// the canonical `selection_state_hash`; the declared defaults come out of the
/// model. `implied_choices` had no declared-ness check and no hash binding, so
/// a compiler-direct caller could seed an undeclared facet or a value outside
/// the facet's declared domain.
///
/// Not a privilege bypass — `assignments` overlays implied (ADR-0057 §D6), so
/// it can never outrank a context tag or an explicit choice, and anything
/// reachable through it is reachable through a hand-crafted `SelectionState`
/// that grants strictly more. It is still worth failing closed on: whatever is
/// injected lands in the `resolve_hash` pre-image, and the mistake is one
/// `apply_selection` already names — hence the two REUSED codes, not a third.
///
/// **One lookup is the whole roster.** At this layer `config.facets` already
/// carries the bindings: `load_resolve_model` extends that very map with
/// `interface_summary::binding_facets` before moving it into the `Config` read
/// here (`shared_ops.rs`), because ADR-0057 §D3 makes a binding one more
/// declared closed facet whose domain is its catalogue's entry ids — and ingest
/// refuses a binding and a facet sharing an id, checked in both directions
/// (`compiler_core.rs`), so the two namespaces cannot collide in that map. The
/// union is load-bearing here: `infer_forced_bindings` draws its roster from
/// `closed_facet_domains`, seeded from both namespaces, and §D6 has it walk
/// "each still-unbound declared closed facet (bindings included)", so a binding
/// decision is ordinary producer output. Reading `config.facets` screens facets
/// and bindings alike through that one canonical projection, with no second
/// copy of it here to drift.
///
/// **Stricter than `apply_selection` — the codes match, the admissible sets do
/// not.** That surface screens against `facet_domains`: the declared values
/// UNIONED with the Eq-derived condition widening, plus a key for any facet a
/// condition merely mentions and no chunk declares. This screen reads the
/// declared `values` alone, so it additionally refuses an undeclared facet some
/// condition names, and a value that widened an OPEN facet's domain. Both are
/// right to refuse: §D6 is explicit that "open facets and undeclared facets are
/// never inferred", so no legitimate producer emits either — a deliberate
/// floor rather than a parity gap, deciding the open-facet case, not missing it.
fn screen_implied_choices(
    model: &ResolveModel,
    implied_choices: &BTreeMap<String, String>,
) -> Vec<Diagnostic> {
    if implied_choices.is_empty() {
        return Vec::new();
    }

    // `implied_choices` is a `BTreeMap`, so this walk is already sorted —
    // deterministic diagnostics without a second sort. Every offending entry is
    // reported, so a caller with two bad entries is not made to discover the
    // second only after fixing the first.
    let mut diagnostics = Vec::new();
    for (facet, option) in implied_choices {
        let Some(declared) = model.config.facets.get(facet) else {
            diagnostics.push(Diagnostic {
                code: E_SELECTION_UNKNOWN_FACET.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!("Unknown selection facet '{}' in implied_choices", facet),
                source_id: None,
                entity_path: None,
                hint: Some(
                    "Supply implied_choices only for facets and bindings the model declares"
                        .to_string(),
                ),
            });
            continue;
        };

        if !declared.values.contains(option) {
            // Sorted rather than declared order, so the text is stable
            // whatever order a chunk happened to declare the values in. Same
            // shape as the hint `apply_selection` renders for this code; the
            // SET is narrower, because it lists the declared `values` rather
            // than the widened `facet_domains` — see the note above.
            let mut options = declared.values.clone();
            options.sort();
            diagnostics.push(Diagnostic {
                code: E_SELECTION_INVALID_OPTION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Invalid option '{}' for facet '{}' in implied_choices",
                    option, facet
                ),
                source_id: None,
                entity_path: None,
                hint: Some(format!("Valid options: {}", options.join(", "))),
            });
        }
    }
    diagnostics
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

    // ADR-0057 §D4: the lowered `derive`/`accepts` conjuncts are root conjuncts
    // of the same model, so a resolve that violates one must fail exactly as it
    // does for an authored constraint — same code, same exit, same "no snapshot
    // on rejection" rule. Appended AFTER the authored block, in the emitter's
    // fold order, so the diagnostics vector reads in the order the model folds
    // them and a model that uses neither feature emits byte-identical output.
    //
    // Fail closed on an unparseable expression for the same reason the authored
    // loop does, and more strongly: this text was generated by
    // `lowering::lowered_root_conjuncts` from data link-verify accepted, so a
    // parse failure means the package cannot be trusted at all.
    for (id, condition) in &model.lowered_conjuncts {
        let expr = parse_condition_expr(condition).map_err(|err| Diagnostic {
            code: E_RESOLVE_MODEL_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Lowered conjunct '{}' has an unparseable expression '{}': {}",
                id, condition, err
            ),
            source_id: model.constraint_sources.get(id).cloned(),
            entity_path: Some(format!("{}{}", CONSTRAINT_ENTITY_PATH_PREFIX, id)),
            hint: Some("Recompile the model from sources the compiler accepts".to_string()),
        })?;
        parsed.push((id.as_str(), expr));
    }

    // The condition text a violated conjunct is reported with: the authored
    // `constraints` entry when the id names one, otherwise the lowered text.
    let lowered_text: BTreeMap<&str, &str> = model
        .lowered_conjuncts
        .iter()
        .map(|(id, condition)| (id.as_str(), condition.as_str()))
        .collect();

    Ok(
        violated_constraint_ids(parsed.iter().map(|(id, expr)| (*id, expr)), assignment)
            .into_iter()
            .map(|id| {
                let condition = model
                    .config
                    .constraints
                    .get(id)
                    .map(|constraint| constraint.condition.as_str())
                    .or_else(|| lowered_text.get(id).copied())
                    .unwrap_or_default();
                constraint_violation_diagnostic(
                    id,
                    condition,
                    model.constraint_sources.get(id).map(String::as_str),
                )
            })
            .collect(),
    )
}

