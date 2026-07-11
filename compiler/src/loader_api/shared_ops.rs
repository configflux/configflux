// SPDX-License-Identifier: BUSL-1.1

fn validate_selection_state(
    model_handle: &ModelHandle,
    scope: &str,
    selection_state: &SelectionState,
) -> std::result::Result<BTreeMap<String, String>, Diagnostic> {
    if selection_state.schema_version != PRODUCT_SCHEMA_VERSION {
        return Err(Diagnostic {
            code: E_SELECTION_STATE_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Selection state schema_version {} is unsupported (expected {})",
                selection_state.schema_version, PRODUCT_SCHEMA_VERSION
            ),
            source_id: None,
            entity_path: None,
            hint: Some(format!("Set selection_state.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
        });
    }
    if scope.trim().is_empty() {
        return Err(Diagnostic {
            code: E_SELECTION_STATE_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "Selection scope is empty".to_string(),
            source_id: None,
            entity_path: None,
            hint: Some("Provide a non-empty scope for selection operations".to_string()),
        });
    }
    if selection_state.model_hash != model_handle.model_hash {
        return Err(Diagnostic {
            code: E_SELECTION_STATE_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Selection model_hash '{}' does not match model_handle model_hash '{}'",
                selection_state.model_hash, model_handle.model_hash
            ),
            source_id: None,
            entity_path: None,
            hint: Some("Rebuild selection_state for the current model_handle".to_string()),
        });
    }
    if selection_state.scope != scope {
        return Err(Diagnostic {
            code: E_SELECTION_STATE_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Selection scope '{}' does not match request scope '{}'",
                selection_state.scope, scope
            ),
            source_id: None,
            entity_path: None,
            hint: Some("Keep request.scope and selection_state.scope identical".to_string()),
        });
    }

    let expected_hash = match compute_selection_state_hash(selection_state) {
        Ok(hash) => hash,
        Err(err) => {
            return Err(Diagnostic {
                code: E_SELECTION_STATE_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: err.to_string(),
                source_id: None,
                entity_path: None,
                hint: Some("Recreate selection_state using canonical hash utility".to_string()),
            });
        }
    };
    if expected_hash != selection_state.selection_state_hash {
        return Err(Diagnostic {
            code: E_SELECTION_STATE_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Selection state hash '{}' does not match canonical hash '{}'",
                selection_state.selection_state_hash, expected_hash
            ),
            source_id: None,
            entity_path: None,
            hint: Some("Recreate selection_state using canonical hash utility".to_string()),
        });
    }

    merge_assignments(selection_state).map_err(|message| Diagnostic {
        code: E_SELECTION_STATE_INVALID.to_string(),
        severity: DiagnosticSeverity::Error,
        message: message.to_string(),
        source_id: None,
        entity_path: None,
        hint: Some("Ensure context_tags and choices do not conflict".to_string()),
    })
}

fn merge_assignments(selection_state: &SelectionState) -> Result<BTreeMap<String, String>> {
    let mut merged = selection_state.context_tags.clone();
    for (facet, option) in &selection_state.choices {
        if let Some(existing) = merged.get(facet) {
            if existing != option {
                anyhow::bail!(
                    "Selection choice '{}'='{}' conflicts with context tag value '{}'",
                    facet,
                    option,
                    existing
                );
            }
        }
        merged.insert(facet.clone(), option.clone());
    }
    Ok(merged)
}

fn load_selection_constraint_model(model_handle: &ModelHandle) -> Result<SelectionConstraintModel> {
    let index = ir::load_index(&model_handle.index_ref)
        .with_context(|| format!("Failed to load index '{}'", model_handle.index_ref))?;
    let chunk_dir = PathBuf::from(&model_handle.chunk_set_ref);
    ir::verify_index_integrity(&index, &chunk_dir).with_context(|| {
        format!(
            "Selection model integrity check failed for chunk directory '{}'",
            model_handle.chunk_set_ref
        )
    })?;

    let mut model = SelectionConstraintModel::default();
    for chunk_ref in &index.chunks {
        let chunk_path = chunk_dir.join(format!("chunk-{}.cfir", chunk_ref.chunk_hash));
        let chunk = ir::load_chunk(&chunk_path)
            .with_context(|| format!("Failed to load chunk '{}'", chunk_path.display()))?;

        // ADR-0047 §4: seed each declared facet's domain from its declaration
        // BEFORE the condition walk widens it. `facet_domains` is a set-union
        // structure, so seeding the declared values and then applying the
        // Eq-derived widening yields the correct effective domain for both
        // kinds: a closed facet's domain is exactly its declared values (a
        // condition value outside them is rejected upstream by
        // `E_FACET_VALUE_UNDECLARED`, so the union adds nothing), and an open
        // facet's domain is declared ∪ inferred. A declaration-only facet that
        // no condition references still gets a `facet_domains` key here, so
        // `list_selection_facets` lists it and the option-validity check
        // accepts its values — including a default arm that no condition names.
        seed_facet_domains_from_declarations(&chunk.facets, &mut model);

        for definition in chunk.definitions.values() {
            register_parameter_conditions(definition, &mut model);
        }
        for component in chunk.components.values() {
            if let Some(condition) = component.condition.as_deref() {
                register_condition(condition, &mut model);
            }
            for param in component.params.values() {
                register_parameter_conditions(param, &mut model);
            }
        }
    }

    Ok(model)
}

/// Seed `SelectionConstraintModel.facet_domains` from a chunk's declared
/// facets (ADR-0047 §4). Every declared value is inserted into the facet's
/// domain set; because `facet_domains` unions with the later Eq-derived
/// condition widening (`register_facet_domains`), this composes to the correct
/// effective domain without special-casing open vs closed here. Registering
/// even a facet that has no declared values would be a no-op — but declared
/// values are non-empty by ingest validation.
fn seed_facet_domains_from_declarations(
    facets: &std::collections::BTreeMap<String, crate::schema::Facet>,
    model: &mut SelectionConstraintModel,
) {
    for (name, facet) in facets {
        let domain = model.facet_domains.entry(name.clone()).or_default();
        for value in &facet.values {
            domain.insert(value.clone());
        }
        let declared = model.declared_values.entry(name.clone()).or_default();
        for value in &facet.values {
            declared.insert(value.clone());
        }
        // ADR-0047 §5/§6: record the declared default (ingest has already
        // re-validated it is a member of `values`). `cfx options` annotates it
        // and the resolve path auto-binds it. A facet with no default records
        // nothing here.
        if let Some(default) = &facet.default {
            model
                .facet_defaults
                .insert(name.clone(), default.clone());
        }
        // ADR-0047 §6 (Amendment 1): record the declared domain-openness so
        // `cfx options` can render the truthful `[closed]`/`[open]` schema-kind
        // token. Every declared facet records a value; undeclared facets are
        // absent, so the render label falls back to today's behavior.
        model.facet_open.insert(name.clone(), facet.open);
    }
}

fn load_resolve_model(model_handle: &ModelHandle) -> Result<crate::schema::Config> {
    let index = ir::load_index(&model_handle.index_ref)
        .with_context(|| format!("Failed to load index '{}'", model_handle.index_ref))?;
    let chunk_dir = PathBuf::from(&model_handle.chunk_set_ref);
    ir::verify_index_integrity(&index, &chunk_dir).with_context(|| {
        format!(
            "Resolve model integrity check failed for chunk directory '{}'",
            model_handle.chunk_set_ref
        )
    })?;

    let mut definitions = HashMap::new();
    let mut components = HashMap::new();
    let mut artifacts = HashMap::new();
    let mut facets: HashMap<String, crate::schema::Facet> = HashMap::new();

    for chunk_ref in &index.chunks {
        let chunk_path = chunk_dir.join(format!("chunk-{}.cfir", chunk_ref.chunk_hash));
        let chunk = ir::load_chunk(&chunk_path)
            .with_context(|| format!("Failed to load chunk '{}'", chunk_path.display()))?;

        // ADR-0047 §5: carry declared facets into the resolve-time `Config` so
        // the auto-bind step can seed each declared default. The at-most-one-
        // declarer invariant is enforced at ingest (`E_INGEST_DUPLICATE_FACET`),
        // so a duplicate here is a corrupted package — fail closed symmetrically
        // with the other namespaces below.
        for (facet_id, facet) in chunk.facets {
            if facets.insert(facet_id.clone(), facet).is_some() {
                anyhow::bail!("Duplicate facet '{}' appears across chunks", facet_id);
            }
        }

        for (definition_id, definition) in chunk.definitions {
            if definitions
                .insert(definition_id.clone(), definition)
                .is_some()
            {
                anyhow::bail!(
                    "Duplicate definition '{}' appears across chunks",
                    definition_id
                );
            }
        }
        for (component_id, component) in chunk.components {
            if components.insert(component_id.clone(), component).is_some() {
                anyhow::bail!(
                    "Duplicate component '{}' appears across chunks",
                    component_id
                );
            }
        }
        for (artifact_id, artifact) in chunk.artifacts {
            if artifacts.insert(artifact_id.clone(), artifact).is_some() {
                anyhow::bail!("Duplicate artifact '{}' appears across chunks", artifact_id);
            }
        }
    }

    Ok(crate::schema::Config {
        package: "merged_root".to_string(),
        version: "0.0.0".to_string(),
        definitions,
        components,
        artifacts,
        facets,
    })
}

fn sorted_artifact_catalog(
    artifacts: &HashMap<String, crate::schema::Artifact>,
) -> BTreeMap<String, crate::schema::Artifact> {
    let mut sorted = BTreeMap::new();
    for (artifact_id, artifact) in artifacts {
        sorted.insert(artifact_id.clone(), artifact.clone());
    }
    sorted
}

fn extract_resolved_component_dependencies(
    model: &crate::schema::Config,
    resolved_scoped: &HashMap<String, crate::resolved_models::ResolvedConfig>,
) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
    let mut by_scope = BTreeMap::new();

    let mut scope_roots: Vec<&String> = resolved_scoped.keys().collect();
    scope_roots.sort();

    for scope_root in scope_roots {
        let Some(resolved_config) = resolved_scoped.get(scope_root) else {
            continue;
        };

        let active_component_ids: BTreeSet<String> =
            resolved_config.components.keys().cloned().collect();
        let mut per_component = BTreeMap::new();

        let mut sorted_component_ids: Vec<String> = active_component_ids.iter().cloned().collect();
        sorted_component_ids.sort();
        for component_id in sorted_component_ids {
            let mut dependency_ids = model
                .components
                .get(&component_id)
                .map(|component| {
                    component
                        .depends_on
                        .iter()
                        .filter(|dep| active_component_ids.contains(*dep))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            dependency_ids.sort();
            dependency_ids.dedup();
            per_component.insert(component_id, dependency_ids);
        }

        by_scope.insert(scope_root.clone(), per_component);
    }

    by_scope
}

fn canonicalize_resolved_output(
    resolved_output: &HashMap<String, crate::resolved_models::ResolvedConfig>,
) -> Result<serde_json::Value> {
    let value = serde_json::to_value(resolved_output)
        .context("Failed to serialize scoped resolved output payload")?;
    Ok(canonicalize_json_value(value))
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

fn compute_resolve_hash(
    model_hash: &str,
    scope: &str,
    selection_state: &SelectionState,
    resolved_output: &serde_json::Value,
    defaulted_choices: &BTreeMap<String, String>,
) -> Result<String> {
    let canonical = ResolveHashCanonical {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash,
        scope,
        selection_state: SelectionStateCanonical {
            schema_version: selection_state.schema_version,
            model_hash: &selection_state.model_hash,
            scope: &selection_state.scope,
            context_tags: &selection_state.context_tags,
            choices: &selection_state.choices,
        },
        resolved_output,
        defaulted_choices,
    };

    let bytes =
        serde_json::to_vec(&canonical).context("Failed to canonicalize resolve hash payload")?;
    Ok(sha256_hex(&bytes))
}

fn extract_early_binding_payload(
    resolved_output: &serde_json::Value,
) -> std::result::Result<EarlyBindingExtraction, ExportGenerationError> {
    let scoped_configs: BTreeMap<String, crate::resolved_models::ResolvedConfig> =
        serde_json::from_value(resolved_output.clone()).map_err(|err| ExportGenerationError {
            code: E_EXPORT_RESOLVE_INVALID,
            message: format!("resolve_result.resolved_output cannot be decoded: {err}"),
            entity_path: Some("resolve_result.resolved_output".to_string()),
            hint: Some("Use resolve_result generated by resolve_from_selection".to_string()),
        })?;

    let mut construction_bindings = Vec::new();
    let mut artifact_paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut seen_header_symbols: BTreeMap<String, String> = BTreeMap::new();
    let mut seen_cmake_vars: BTreeMap<String, String> = BTreeMap::new();

    for (scope_root, resolved_config) in scoped_configs {
        let mut components: Vec<(String, crate::resolved_models::ResolvedComponent)> =
            resolved_config.components.into_iter().collect();
        components.sort_by(|a, b| a.0.cmp(&b.0));

        for (component_id, component) in components {
            let mut params: Vec<(String, crate::resolved_models::ResolvedParameter)> =
                component.params.into_iter().collect();
            params.sort_by(|a, b| a.0.cmp(&b.0));

            for (param_key, param) in params {
                if param.lifecycle != crate::schema::Lifecycle::Construction {
                    continue;
                }

                let entity_path =
                    format!("{scope_root}.component.{component_id}.param.{param_key}");
                let symbol_seed = format!("{component_id}_{param_key}");
                let header_symbol = format!("k{}", normalize_upper_camel_identifier(&symbol_seed));
                if !is_cpp_identifier(&header_symbol) {
                    return Err(ExportGenerationError {
                        code: E_EXPORT_SYMBOL_INVALID,
                        message: format!(
                            "Generated header symbol '{header_symbol}' is not a valid C++ identifier"
                        ),
                        entity_path: Some(entity_path),
                        hint: Some("Use snake_case component/parameter IDs for stable symbol names".to_string()),
                    });
                }
                if let Some(existing_path) =
                    seen_header_symbols.insert(header_symbol.clone(), entity_path.clone())
                {
                    return Err(ExportGenerationError {
                        code: E_EXPORT_SYMBOL_INVALID,
                        message: format!(
                            "Header symbol collision for '{}' between '{}' and '{}'",
                            header_symbol, existing_path, entity_path
                        ),
                        entity_path: Some(entity_path),
                        hint: Some(
                            "Rename conflicting components/parameters to avoid symbol collisions"
                                .to_string(),
                        ),
                    });
                }

                let cmake_var = format!("CFG_{}", normalize_upper_snake_identifier(&symbol_seed));
                if !is_cpp_identifier(&cmake_var) {
                    return Err(ExportGenerationError {
                        code: E_EXPORT_SYMBOL_INVALID,
                        message: format!(
                            "Generated compile variable '{cmake_var}' is not a valid identifier"
                        ),
                        entity_path: Some(entity_path),
                        hint: Some(
                            "Use snake_case component/parameter IDs for stable macro names"
                                .to_string(),
                        ),
                    });
                }
                if let Some(existing_path) =
                    seen_cmake_vars.insert(cmake_var.clone(), entity_path.clone())
                {
                    return Err(ExportGenerationError {
                        code: E_EXPORT_SYMBOL_INVALID,
                        message: format!(
                            "Compile variable collision for '{}' between '{}' and '{}'",
                            cmake_var, existing_path, entity_path
                        ),
                        entity_path: Some(entity_path),
                        hint: Some(
                            "Rename conflicting components/parameters to avoid macro collisions"
                                .to_string(),
                        ),
                    });
                }

                let value_token_source = value_token_source(&param.value)?;
                let value_token = normalize_upper_snake_identifier(&value_token_source);
                let compile_definition = format!("{cmake_var}_{value_token}=1");

                if param.r#type == "artifact" {
                    let artifact_id = match &param.value {
                        crate::schema::Value::String(value) if !value.trim().is_empty() => value.trim().to_string(),
                        _ => {
                            return Err(ExportGenerationError {
                                code: E_EXPORT_ARTIFACT_INVALID,
                                message: format!(
                                    "Artifact parameter at '{}' must contain a non-empty string artifact ID",
                                    entity_path
                                ),
                                entity_path: Some(entity_path),
                                hint: Some("Resolve output artifact parameters must serialize as string IDs".to_string()),
                            })
                        }
                    };
                    artifact_paths
                        .entry(artifact_id)
                        .or_default()
                        .insert(format!("component.{component_id}.param.{param_key}"));
                }

                construction_bindings.push(ConstructionParamBinding {
                    entity_path,
                    value: param.value,
                    header_symbol,
                    cmake_var,
                    compile_definition,
                });
            }
        }
    }

    construction_bindings.sort_by(|a, b| a.entity_path.cmp(&b.entity_path));

    let artifact_manifest_entries = artifact_paths
        .into_iter()
        .map(|(artifact_id, paths)| ArtifactManifestEntry {
            artifact_id,
            bound_paths: paths.into_iter().collect(),
        })
        .collect();

    Ok(EarlyBindingExtraction {
        construction_bindings,
        artifact_manifest_entries,
    })
}

fn emit_config_hpp(
    construction_bindings: &[ConstructionParamBinding],
) -> std::result::Result<String, ExportGenerationError> {
    let mut out = String::new();
    out.push_str("#pragma once\n\n");
    out.push_str("namespace configflux::buildcfg {\n");

    if construction_bindings.is_empty() {
        out.push_str("// No construction-lifecycle parameters available.\n");
    } else {
        for binding in construction_bindings {
            let literal = cpp_literal(&binding.value)?;
            let type_name = cpp_type_name(&binding.value);
            out.push_str(&format!(
                "inline constexpr {} {} = {};\n",
                type_name, binding.header_symbol, literal
            ));
        }
    }

    out.push_str("}  // namespace configflux::buildcfg\n");
    Ok(out)
}

fn emit_config_build_flags(
    construction_bindings: &[ConstructionParamBinding],
) -> std::result::Result<String, ExportGenerationError> {
    let mut out = String::new();
    out.push_str("# Generated by ConfigFlux profile cpp_early_binding_v1\n");

    for binding in construction_bindings {
        let literal = cmake_literal(&binding.value)?;
        out.push_str(&format!("set({} {})\n", binding.cmake_var, literal));
    }

    let mut compile_definitions = BTreeSet::new();
    for binding in construction_bindings {
        compile_definitions.insert(binding.compile_definition.clone());
    }

    out.push_str("add_compile_definitions(\n");
    for definition in compile_definitions {
        out.push_str(&format!("  {definition}\n"));
    }
    out.push_str(")\n");
    Ok(out)
}

fn emit_config_artifact_manifest(
    profile: &str,
    model_hash: &str,
    resolve_hash: &str,
    artifact_entries: &[ArtifactManifestEntry],
) -> std::result::Result<String, ExportGenerationError> {
    let manifest = ArtifactManifest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        profile: profile.to_string(),
        model_hash: model_hash.to_string(),
        resolve_hash: resolve_hash.to_string(),
        artifacts: artifact_entries.to_vec(),
    };

    let value = serde_json::to_value(&manifest).map_err(|err| ExportGenerationError {
        code: E_EXPORT_FAILED,
        message: format!("Failed to serialize artifact manifest: {err}"),
        entity_path: Some("generated/config_artifact_manifest.json".to_string()),
        hint: Some("Artifact manifest content must be serializable".to_string()),
    })?;
    let canonical = canonicalize_json_value(value);

    serde_json::to_string_pretty(&canonical).map_err(|err| ExportGenerationError {
        code: E_EXPORT_FAILED,
        message: format!("Failed to render artifact manifest JSON: {err}"),
        entity_path: Some("generated/config_artifact_manifest.json".to_string()),
        hint: Some("Artifact manifest content must be valid JSON".to_string()),
    })
}

fn compute_generator_hash(
    profile: &str,
    model_hash: &str,
    scope: &str,
    resolve_hash: &str,
    files: &[GeneratedArtifact],
) -> Result<String> {
    let canonical_files = files
        .iter()
        .map(|file| GeneratedArtifactHashCanonical {
            path: &file.path,
            content_hash: &file.content_hash,
        })
        .collect::<Vec<_>>();
    let canonical = GeneratorHashCanonical {
        schema_version: PRODUCT_SCHEMA_VERSION,
        profile,
        model_hash,
        scope,
        resolve_hash,
        files: &canonical_files,
    };
    let bytes =
        serde_json::to_vec(&canonical).context("Failed to canonicalize generator hash payload")?;
    Ok(sha256_hex(&bytes))
}

fn build_software_bom_payload(
    profile: &str,
    resolve_result: &ResolveResult,
    resolve_hash: &str,
    resolved_output: serde_json::Value,
) -> std::result::Result<SoftwareBomV1, SoftwareBomGenerationError> {
    let scoped_configs = decode_scoped_resolved_configs_for_bom(resolved_output)?;
    if scoped_configs.is_empty() {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_RESOLVE_INVALID,
            message: "resolve_result.resolved_output is empty".to_string(),
            entity_path: Some("resolve_result.resolved_output".to_string()),
            hint: Some("Resolve a non-empty scope before exporting software BOM".to_string()),
        });
    }

    let mut components = BTreeMap::<String, SoftwareBomComponentEntry>::new();
    let mut parameters = BTreeMap::<String, SoftwareBomParameterEntry>::new();
    let mut artifact_bindings = BTreeMap::<String, BTreeSet<String>>::new();

    let mut scope_roots: Vec<String> = scoped_configs.keys().cloned().collect();
    scope_roots.sort();
    for scope_root in &scope_roots {
        let Some(resolved_config) = scoped_configs.get(scope_root) else {
            continue;
        };

        let dependencies_for_scope = resolve_result
            .resolved_component_dependencies
            .get(scope_root);
        let mut component_ids: Vec<String> = resolved_config.components.keys().cloned().collect();
        component_ids.sort();

        for component_id in component_ids {
            let Some(component) = resolved_config.components.get(&component_id) else {
                continue;
            };

            let mut dependency_ids = dependencies_for_scope
                .and_then(|per_component| per_component.get(&component_id).cloned())
                .unwrap_or_default();
            dependency_ids.sort();
            dependency_ids.dedup();

            if let Some(existing) = components.get_mut(&component_id) {
                if existing.r#type != component.r#type {
                    return Err(SoftwareBomGenerationError {
                        code: E_SBOM_RESOLVE_INVALID,
                        message: format!(
                            "Component '{}' has conflicting resolved types '{}' and '{}'",
                            component_id, existing.r#type, component.r#type
                        ),
                        entity_path: Some(format!("component.{component_id}")),
                        hint: Some(
                            "Resolve output must be consistent when multiple roots are exported"
                                .to_string(),
                        ),
                    });
                }
                existing.dependency_ids.extend(dependency_ids);
                existing.dependency_ids.sort();
                existing.dependency_ids.dedup();
            } else {
                components.insert(
                    component_id.clone(),
                    SoftwareBomComponentEntry {
                        component_id: component_id.clone(),
                        r#type: component.r#type.clone(),
                        dependency_ids,
                    },
                );
            }

            let mut param_keys: Vec<String> = component.params.keys().cloned().collect();
            param_keys.sort();
            for param_key in param_keys {
                let Some(param) = component.params.get(&param_key) else {
                    continue;
                };

                let path = expected_parameter_path(&component_id, &param_key);
                let parameter_entry = SoftwareBomParameterEntry {
                    path: path.clone(),
                    component_id: component_id.clone(),
                    param_key: param_key.clone(),
                    r#type: param.r#type.clone(),
                    value: param.value.clone(),
                    unit: param.unit.clone(),
                    safety: param.safety.clone(),
                    lifecycle: param.lifecycle.clone(),
                    binding_phase: binding_phase_for_lifecycle(&param.lifecycle),
                    access: param.access.clone(),
                    req_id: param.req_id.clone(),
                    doc: param.doc.clone(),
                    limits: param.limits.clone(),
                };

                if let Some(existing) = parameters.insert(path.clone(), parameter_entry.clone()) {
                    if existing != parameter_entry {
                        return Err(SoftwareBomGenerationError {
                            code: E_SBOM_RESOLVE_INVALID,
                            message: format!(
                                "Parameter '{}' has conflicting values across resolved roots",
                                path
                            ),
                            entity_path: Some(path),
                            hint: Some(
                                "Use a single canonical resolved scope for software BOM export"
                                    .to_string(),
                            ),
                        });
                    }
                }

                if parameter_entry.r#type == "artifact" {
                    let artifact_id = match &parameter_entry.value {
                        crate::schema::Value::String(value) if !value.trim().is_empty() => {
                            value.trim().to_string()
                        }
                        _ => {
                            return Err(SoftwareBomGenerationError {
                                code: E_SBOM_ARTIFACT_INVALID,
                                message: format!(
                                    "Artifact parameter '{}' must contain a non-empty string artifact ID",
                                    parameter_entry.path
                                ),
                                entity_path: Some(parameter_entry.path.clone()),
                                hint: Some("Resolve output artifact values must be string IDs".to_string()),
                            })
                        }
                    };
                    artifact_bindings
                        .entry(artifact_id)
                        .or_default()
                        .insert(parameter_entry.path.clone());
                }
            }
        }
    }

    let components = components.into_values().collect::<Vec<_>>();
    let parameters = parameters.into_values().collect::<Vec<_>>();

    let mut artifacts = Vec::new();
    for (artifact_id, bound_paths) in artifact_bindings {
        let Some(metadata) = resolve_result.resolved_artifacts.get(&artifact_id) else {
            let path = bound_paths.iter().next().cloned().unwrap_or_default();
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!(
                    "Artifact '{}' is referenced by '{}' but missing from resolved artifact catalog",
                    artifact_id, path
                ),
                entity_path: Some(path),
                hint: Some(
                    "Resolve output artifact references must map to known artifact metadata"
                        .to_string(),
                ),
            });
        };
        if metadata.name.trim().is_empty() {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!("Artifact '{}' has an empty name", artifact_id),
                entity_path: Some(format!("artifact.{}", artifact_id)),
                hint: Some("Artifact metadata must include a non-empty name".to_string()),
            });
        }

        artifacts.push(SoftwareBomArtifactEntry {
            artifact_id,
            bound_paths: bound_paths.into_iter().collect(),
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            hash: metadata.hash.clone(),
            source: metadata.source.clone(),
            target: metadata.target.clone(),
            doc: metadata.doc.clone(),
        });
    }
    artifacts.sort_by(|a, b| a.artifact_id.cmp(&b.artifact_id));

    let scope_root = if scope_roots.len() == 1 {
        scope_roots[0].clone()
    } else {
        resolve_result.scope.clone()
    };

    let mut software_bom = SoftwareBomV1 {
        schema_version: PRODUCT_SCHEMA_VERSION,
        bom_version: SOFTWARE_BOM_VERSION,
        bom_hash: String::new(),
        hash_algo: SOFTWARE_BOM_HASH_ALGO.to_string(),
        canonicalization_version: SOFTWARE_BOM_CANONICALIZATION_VERSION,
        model_hash: resolve_result.model_hash.clone(),
        resolve_hash: resolve_hash.to_string(),
        selection_state_hash: if resolve_result.selection_state_hash.trim().is_empty() {
            None
        } else {
            Some(resolve_result.selection_state_hash.clone())
        },
        scope_root,
        generated_at: SOFTWARE_BOM_GENERATED_AT_RFC3339.to_string(),
        generator: SoftwareBomGeneratorMetadata {
            name: SOFTWARE_BOM_GENERATOR_NAME.to_string(),
            version: SOFTWARE_BOM_GENERATOR_VERSION.to_string(),
        },
        context_tags: resolve_result.context_tags.clone(),
        choices: resolve_result.choices.clone(),
        components,
        parameters,
        artifacts,
        stats: SoftwareBomStats {
            component_count: 0,
            parameter_count: 0,
            artifact_count: 0,
        },
    };
    software_bom.stats = SoftwareBomStats {
        component_count: software_bom.components.len() as u32,
        parameter_count: software_bom.parameters.len() as u32,
        artifact_count: software_bom.artifacts.len() as u32,
    };

    apply_software_bom_profile(profile, &mut software_bom)?;
    Ok(software_bom)
}

fn decode_scoped_resolved_configs_for_bom(
    resolved_output: serde_json::Value,
) -> std::result::Result<
    BTreeMap<String, crate::resolved_models::ResolvedConfig>,
    SoftwareBomGenerationError,
> {
    serde_json::from_value(resolved_output).map_err(|err| {
        let message = err.to_string();
        let lowered = message.to_ascii_lowercase();
        let code = if lowered.contains("lifecycle") || lowered.contains("unknown variant") {
            E_SBOM_BINDING_INVALID
        } else {
            E_SBOM_RESOLVE_INVALID
        };
        let hint = if code == E_SBOM_BINDING_INVALID {
            Some("Resolved lifecycle values must be construction, startup, or runtime".to_string())
        } else {
            Some("Use resolve_result generated by resolve_from_selection".to_string())
        };
        SoftwareBomGenerationError {
            code,
            message: format!("resolve_result.resolved_output cannot be decoded: {message}"),
            entity_path: Some("resolve_result.resolved_output".to_string()),
            hint,
        }
    })
}

fn apply_software_bom_profile(
    profile: &str,
    software_bom: &mut SoftwareBomV1,
) -> std::result::Result<(), SoftwareBomGenerationError> {
    match profile {
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT => Ok(()),
        EXPORT_SOFTWARE_BOM_PROFILE_VALUE_REDACTED => {
            for parameter in &mut software_bom.parameters {
                let should_redact = parameter.binding_phase != SoftwareBomBindingPhase::Early
                    && parameter.r#type != "artifact";
                if should_redact {
                    parameter.value = crate::schema::Value::String("<redacted>".to_string());
                }
            }
            Ok(())
        }
        _ => Err(SoftwareBomGenerationError {
            code: E_SBOM_PROFILE_INVALID,
            message: format!("Unsupported software BOM profile '{}'", profile),
            entity_path: Some("export.profile".to_string()),
            hint: Some("Use 'full_audit' or 'value_redacted' profile".to_string()),
        }),
    }
}

fn binding_phase_for_lifecycle(lifecycle: &crate::schema::Lifecycle) -> SoftwareBomBindingPhase {
    match lifecycle {
        crate::schema::Lifecycle::Construction => SoftwareBomBindingPhase::Early,
        crate::schema::Lifecycle::Startup => SoftwareBomBindingPhase::Late,
        crate::schema::Lifecycle::Runtime => SoftwareBomBindingPhase::Runtime,
    }
}

fn expected_parameter_path(component_id: &str, param_key: &str) -> String {
    format!("component.{component_id}.param.{param_key}")
}

fn is_sorted_unique_strings(values: &[String]) -> bool {
    values.windows(2).all(|window| window[0] < window[1])
}

fn canonical_software_bom_bytes(software_bom: &SoftwareBomV1) -> Result<Vec<u8>> {
    let mut canonical_payload = software_bom.clone();
    canonical_payload.bom_hash.clear();

    let value = serde_json::to_value(&canonical_payload)
        .context("Failed to serialize software BOM payload for canonical hashing")?;
    let canonical = canonicalize_json_value(value);
    serde_json::to_vec(&canonical)
        .context("Failed to serialize canonical software BOM payload for hashing")
}

fn compute_software_bom_hash(software_bom: &SoftwareBomV1) -> Result<String> {
    let bytes = canonical_software_bom_bytes(software_bom)?;
    Ok(sha256_hex(&bytes))
}

pub(crate) fn validate_software_bom_payload(
    software_bom: &SoftwareBomV1,
) -> std::result::Result<(), SoftwareBomGenerationError> {
    if software_bom.hash_algo != SOFTWARE_BOM_HASH_ALGO {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_HASH_INVALID,
            message: format!(
                "Unsupported hash_algo '{}' (expected '{}')",
                software_bom.hash_algo, SOFTWARE_BOM_HASH_ALGO
            ),
            entity_path: Some("hash_algo".to_string()),
            hint: Some("Set hash_algo to sha256 for SoftwareBomV1".to_string()),
        });
    }
    if software_bom.canonicalization_version != SOFTWARE_BOM_CANONICALIZATION_VERSION {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_HASH_INVALID,
            message: format!(
                "Unsupported canonicalization_version {} (expected {})",
                software_bom.canonicalization_version, SOFTWARE_BOM_CANONICALIZATION_VERSION
            ),
            entity_path: Some("canonicalization_version".to_string()),
            hint: Some("Set canonicalization_version to 1 for SoftwareBomV1".to_string()),
        });
    }
    if software_bom.bom_version != SOFTWARE_BOM_VERSION {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_HASH_INVALID,
            message: format!(
                "Unsupported bom_version {} (expected {})",
                software_bom.bom_version, SOFTWARE_BOM_VERSION
            ),
            entity_path: Some("bom_version".to_string()),
            hint: Some("Set bom_version to 1 for SoftwareBomV1".to_string()),
        });
    }
    if software_bom.model_hash.trim().is_empty() {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_RESOLVE_INVALID,
            message: "Software BOM model_hash is empty".to_string(),
            entity_path: Some("model_hash".to_string()),
            hint: Some("Use resolve_result with a valid model_hash".to_string()),
        });
    }
    if software_bom.resolve_hash.trim().is_empty() {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_RESOLVE_INVALID,
            message: "Software BOM resolve_hash is empty".to_string(),
            entity_path: Some("resolve_hash".to_string()),
            hint: Some("Use resolve_result with a valid resolve_hash".to_string()),
        });
    }

    let component_ids = software_bom
        .components
        .iter()
        .map(|component| component.component_id.clone())
        .collect::<Vec<_>>();
    if !is_sorted_unique_strings(&component_ids) {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_PATH_INVALID,
            message: "components must be sorted by unique component_id".to_string(),
            entity_path: Some("components".to_string()),
            hint: Some("Sort components lexicographically by component_id".to_string()),
        });
    }
    let component_id_set: BTreeSet<String> = component_ids.iter().cloned().collect();
    for component in &software_bom.components {
        if !is_sorted_unique_strings(&component.dependency_ids) {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_PATH_INVALID,
                message: format!(
                    "Component '{}' dependency_ids must be sorted and unique",
                    component.component_id
                ),
                entity_path: Some(format!(
                    "component.{}.dependency_ids",
                    component.component_id
                )),
                hint: Some("Sort dependency_ids lexicographically and deduplicate".to_string()),
            });
        }
        for dependency_id in &component.dependency_ids {
            if !component_id_set.contains(dependency_id) {
                return Err(SoftwareBomGenerationError {
                    code: E_SBOM_PATH_INVALID,
                    message: format!(
                        "Component '{}' dependency '{}' is not present in BOM components",
                        component.component_id, dependency_id
                    ),
                    entity_path: Some(format!(
                        "component.{}.dependency_ids",
                        component.component_id
                    )),
                    hint: Some("Only include active dependencies present in this BOM".to_string()),
                });
            }
        }
    }

    let parameter_paths = software_bom
        .parameters
        .iter()
        .map(|parameter| parameter.path.clone())
        .collect::<Vec<_>>();
    if !is_sorted_unique_strings(&parameter_paths) {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_PATH_INVALID,
            message: "parameters must be sorted by unique path".to_string(),
            entity_path: Some("parameters".to_string()),
            hint: Some("Sort parameters lexicographically by path".to_string()),
        });
    }

    let mut artifact_parameters = BTreeMap::<String, String>::new();
    let mut artifact_parameter_ids = BTreeSet::<String>::new();
    for parameter in &software_bom.parameters {
        let expected_path = expected_parameter_path(&parameter.component_id, &parameter.param_key);
        if parameter.path != expected_path {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_PATH_INVALID,
                message: format!(
                    "Parameter path '{}' does not match component/param '{}'",
                    parameter.path, expected_path
                ),
                entity_path: Some(parameter.path.clone()),
                hint: Some(
                    "Use path format component.<component_id>.param.<param_key>".to_string(),
                ),
            });
        }

        let expected_binding = binding_phase_for_lifecycle(&parameter.lifecycle);
        if parameter.binding_phase != expected_binding {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_BINDING_INVALID,
                message: format!(
                    "Parameter '{}' binding_phase '{:?}' does not match lifecycle '{:?}'",
                    parameter.path, parameter.binding_phase, parameter.lifecycle
                ),
                entity_path: Some(parameter.path.clone()),
                hint: Some(
                    "Use mapping construction->early, startup->late, runtime->runtime".to_string(),
                ),
            });
        }

        if parameter.r#type == "artifact" {
            let artifact_id = match &parameter.value {
                crate::schema::Value::String(value) if !value.trim().is_empty() => {
                    value.trim().to_string()
                }
                _ => {
                    return Err(SoftwareBomGenerationError {
                        code: E_SBOM_ARTIFACT_INVALID,
                        message: format!(
                            "Artifact parameter '{}' must contain a non-empty string artifact ID",
                            parameter.path
                        ),
                        entity_path: Some(parameter.path.clone()),
                        hint: Some("Artifact parameter values must be string IDs".to_string()),
                    })
                }
            };
            artifact_parameter_ids.insert(artifact_id.clone());
            artifact_parameters.insert(parameter.path.clone(), artifact_id);
        }
    }
    let parameter_path_set: BTreeSet<String> = parameter_paths.iter().cloned().collect();

    let artifact_ids = software_bom
        .artifacts
        .iter()
        .map(|artifact| artifact.artifact_id.clone())
        .collect::<Vec<_>>();
    if !is_sorted_unique_strings(&artifact_ids) {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_ARTIFACT_INVALID,
            message: "artifacts must be sorted by unique artifact_id".to_string(),
            entity_path: Some("artifacts".to_string()),
            hint: Some("Sort artifacts lexicographically by artifact_id".to_string()),
        });
    }

    let artifact_id_set: BTreeSet<String> = artifact_ids.iter().cloned().collect();
    let mut path_to_artifact = BTreeMap::<String, String>::new();
    for artifact in &software_bom.artifacts {
        if artifact.name.trim().is_empty() {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!("Artifact '{}' has an empty name", artifact.artifact_id),
                entity_path: Some(format!("artifact.{}", artifact.artifact_id)),
                hint: Some("Artifact metadata must include a non-empty name".to_string()),
            });
        }
        if artifact.bound_paths.is_empty() {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!("Artifact '{}' has no bound_paths", artifact.artifact_id),
                entity_path: Some(format!("artifact.{}.bound_paths", artifact.artifact_id)),
                hint: Some(
                    "Artifact entries must reference at least one parameter path".to_string(),
                ),
            });
        }
        if !is_sorted_unique_strings(&artifact.bound_paths) {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!(
                    "Artifact '{}' bound_paths must be sorted and unique",
                    artifact.artifact_id
                ),
                entity_path: Some(format!("artifact.{}.bound_paths", artifact.artifact_id)),
                hint: Some("Sort artifact bound_paths lexicographically".to_string()),
            });
        }
        for bound_path in &artifact.bound_paths {
            if !parameter_path_set.contains(bound_path) {
                return Err(SoftwareBomGenerationError {
                    code: E_SBOM_ARTIFACT_INVALID,
                    message: format!(
                        "Artifact '{}' bound path '{}' does not exist in parameters",
                        artifact.artifact_id, bound_path
                    ),
                    entity_path: Some(format!("artifact.{}.bound_paths", artifact.artifact_id)),
                    hint: Some(
                        "Artifact bound_paths must reference existing parameter paths".to_string(),
                    ),
                });
            }
            if let Some(existing) =
                path_to_artifact.insert(bound_path.clone(), artifact.artifact_id.clone())
            {
                if existing != artifact.artifact_id {
                    return Err(SoftwareBomGenerationError {
                        code: E_SBOM_ARTIFACT_INVALID,
                        message: format!(
                            "Parameter path '{}' is bound to multiple artifacts ('{}', '{}')",
                            bound_path, existing, artifact.artifact_id
                        ),
                        entity_path: Some(bound_path.clone()),
                        hint: Some(
                            "Each artifact parameter path must map to exactly one artifact entry"
                                .to_string(),
                        ),
                    });
                }
            }
        }
    }

    for (path, artifact_id) in &artifact_parameters {
        let Some(bound_artifact_id) = path_to_artifact.get(path) else {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!(
                    "Artifact parameter '{}' with value '{}' is missing from artifact bound_paths",
                    path, artifact_id
                ),
                entity_path: Some(path.clone()),
                hint: Some("Add matching artifact entry with this parameter path".to_string()),
            });
        };
        if bound_artifact_id != artifact_id {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!(
                    "Artifact parameter '{}' value '{}' does not match bound artifact '{}'",
                    path, artifact_id, bound_artifact_id
                ),
                entity_path: Some(path.clone()),
                hint: Some(
                    "Keep artifact parameter value and bound artifact_id identical".to_string(),
                ),
            });
        }
        if !artifact_id_set.contains(artifact_id) {
            return Err(SoftwareBomGenerationError {
                code: E_SBOM_ARTIFACT_INVALID,
                message: format!(
                    "Artifact '{}' referenced by '{}' is missing",
                    artifact_id, path
                ),
                entity_path: Some(path.clone()),
                hint: Some("Add matching artifact metadata entry to artifacts list".to_string()),
            });
        }
    }

    if software_bom.stats.component_count != software_bom.components.len() as u32
        || software_bom.stats.parameter_count != software_bom.parameters.len() as u32
        || software_bom.stats.artifact_count != software_bom.artifacts.len() as u32
    {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_STATS_INVALID,
            message: format!(
                "Software BOM stats mismatch (stats: c={} p={} a={}; actual: c={} p={} a={})",
                software_bom.stats.component_count,
                software_bom.stats.parameter_count,
                software_bom.stats.artifact_count,
                software_bom.components.len(),
                software_bom.parameters.len(),
                software_bom.artifacts.len()
            ),
            entity_path: Some("stats".to_string()),
            hint: Some("Set stats counts to exact list lengths".to_string()),
        });
    }

    if !artifact_parameter_ids.is_subset(&artifact_id_set) {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_ARTIFACT_INVALID,
            message: "Artifact parameter references are not fully covered by artifacts list"
                .to_string(),
            entity_path: Some("artifacts".to_string()),
            hint: Some(
                "Ensure every artifact parameter has exactly one artifact entry".to_string(),
            ),
        });
    }

    let expected_hash =
        compute_software_bom_hash(software_bom).map_err(|err| SoftwareBomGenerationError {
            code: E_SBOM_HASH_INVALID,
            message: format!("Failed to recompute software BOM hash: {err}"),
            entity_path: Some("bom_hash".to_string()),
            hint: Some("Software BOM payload must be canonically serializable".to_string()),
        })?;
    if software_bom.bom_hash != expected_hash {
        return Err(SoftwareBomGenerationError {
            code: E_SBOM_HASH_INVALID,
            message: format!(
                "Software BOM hash '{}' does not match canonical hash '{}'",
                software_bom.bom_hash, expected_hash
            ),
            entity_path: Some("bom_hash".to_string()),
            hint: Some("Set bom_hash to sha256 hash of canonical BOM payload".to_string()),
        });
    }

    Ok(())
}

fn cpp_type_name(value: &crate::schema::Value) -> &'static str {
    match value {
        crate::schema::Value::Integer(_) => "long long",
        crate::schema::Value::Float(_) => "double",
        crate::schema::Value::Boolean(_) => "bool",
        crate::schema::Value::String(_) => "const char*",
    }
}

fn cpp_literal(value: &crate::schema::Value) -> std::result::Result<String, ExportGenerationError> {
    match value {
        crate::schema::Value::Integer(value) => Ok(value.to_string()),
        crate::schema::Value::Boolean(value) => {
            Ok(if *value { "true" } else { "false" }.to_string())
        }
        crate::schema::Value::String(value) => Ok(format!("\"{}\"", escape_cpp_string(value))),
        crate::schema::Value::Float(value) => {
            let mut text = float_literal(*value)?;
            if !text.contains('.') && !text.contains('e') && !text.contains('E') {
                text.push_str(".0");
            }
            Ok(text)
        }
    }
}

fn cmake_literal(
    value: &crate::schema::Value,
) -> std::result::Result<String, ExportGenerationError> {
    let token = value_token_source(value)?;
    Ok(format!("\"{}\"", escape_cmake_string(&token)))
}

fn value_token_source(
    value: &crate::schema::Value,
) -> std::result::Result<String, ExportGenerationError> {
    match value {
        crate::schema::Value::Integer(value) => Ok(value.to_string()),
        crate::schema::Value::Boolean(value) => {
            Ok(if *value { "true" } else { "false" }.to_string())
        }
        crate::schema::Value::String(value) => Ok(value.clone()),
        crate::schema::Value::Float(value) => float_literal(*value),
    }
}

fn float_literal(value: f64) -> std::result::Result<String, ExportGenerationError> {
    let Some(number) = serde_json::Number::from_f64(value) else {
        return Err(ExportGenerationError {
            code: E_EXPORT_FAILED,
            message: format!(
                "Non-finite float value '{value}' cannot be emitted deterministically"
            ),
            entity_path: None,
            hint: Some("Use finite numeric values for early-binding constants".to_string()),
        });
    };
    Ok(number.to_string())
}

fn escape_cpp_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_cmake_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn normalize_upper_snake_identifier(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_underscore = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            if out.is_empty() && ch.is_ascii_digit() {
                out.push('X');
                out.push('_');
            }
            out.push(ch.to_ascii_uppercase());
            last_was_underscore = false;
            continue;
        }

        if !out.is_empty() && !last_was_underscore {
            out.push('_');
            last_was_underscore = true;
        }
    }

    while out.ends_with('_') {
        out.pop();
    }

    if out.is_empty() {
        out.push('X');
    }
    out
}

fn normalize_upper_camel_identifier(input: &str) -> String {
    let upper_snake = normalize_upper_snake_identifier(input);
    let mut out = String::new();

    for segment in upper_snake.split('_').filter(|part| !part.is_empty()) {
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            continue;
        };

        if first.is_ascii_digit() {
            out.push('X');
            out.push(first);
        } else {
            out.push(first.to_ascii_uppercase());
        }
        for ch in chars {
            out.push(ch.to_ascii_lowercase());
        }
    }

    if out.is_empty() {
        out.push('X');
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, 'X');
    }
    out
}

fn is_cpp_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn register_parameter_conditions(
    parameter: &crate::schema::Parameter,
    model: &mut SelectionConstraintModel,
) {
    for override_block in &parameter.overrides {
        register_condition(&override_block.condition, model);
        register_parameter_conditions(override_block.payload.as_ref(), model);
    }
}

fn register_condition(condition: &str, model: &mut SelectionConstraintModel) {
    // configflux-ccs.7: parse into the typed AST and store the node. A
    // condition that does not parse under grammar v2 widens no facet domain
    // and forms no group, matching the legacy fall-through where both
    // string-scanning helpers (the former `parse_condition_conjunction` /
    // `scan_condition_predicates`, removed in configflux-uiyo) declined.
    let Ok(expr) = parse_condition_expr(condition) else {
        return;
    };
    register_facet_domains(&expr, model);
    model.conditions.push(expr);
}

fn register_facet_domains(expr: &ConditionExpr, model: &mut SelectionConstraintModel) {
    // Only equality atoms widen a facet's domain, exactly as the legacy
    // `register_facet_domains` did for `ConditionPredicateOp::Eq`.
    for_each_eq_predicate(expr, |tag, value| {
        model
            .facet_domains
            .entry(tag.to_string())
            .or_default()
            .insert(value.to_string());
    });
}

fn facet_domain(
    model: &SelectionConstraintModel,
    assignments: &BTreeMap<String, String>,
    facet: &str,
) -> Option<BTreeSet<String>> {
    if let Some(domain) = model.facet_domains.get(facet) {
        let mut domain = domain.clone();
        if let Some(selected) = assignments.get(facet) {
            domain.insert(selected.clone());
        }
        return Some(domain);
    }

    assignments.get(facet).map(|selected| {
        let mut domain = BTreeSet::new();
        domain.insert(selected.clone());
        domain
    })
}

fn valid_options_for_facet(
    model: &SelectionConstraintModel,
    assignments: &BTreeMap<String, String>,
    facet: &str,
    domain: &BTreeSet<String>,
) -> Vec<String> {
    if !model.facet_domains.contains_key(facet) {
        return domain.iter().cloned().collect();
    }

    let mut valid = Vec::new();
    for option in domain {
        if option_is_valid(model, assignments, facet, option) {
            valid.push(option.clone());
        }
    }
    valid
}

fn option_is_valid(
    model: &SelectionConstraintModel,
    assignments: &BTreeMap<String, String>,
    facet: &str,
    option: &str,
) -> bool {
    let mut candidate = assignments.clone();
    candidate.insert(facet.to_string(), option.to_string());

    // configflux-ccs.7: walk the stored typed conditions. As in the legacy
    // path, only pure conjunctions of atoms (the conditions the former
    // `parse_condition_conjunction` accepted as groups) participate, and a
    // condition is consulted only when it mentions `facet == option`. The
    // typed evaluator's `not_contradicted` reproduces `group_compatible`:
    // unassigned tags are skipped, a condition fails only on a false
    // assigned atom.
    for condition in &model.conditions {
        if !is_pure_conjunction(condition) {
            continue;
        }
        if !mentions_eq(condition, facet, option) {
            continue;
        }
        if not_contradicted(condition, &candidate) {
            return true;
        }
    }

    // ADR-0047 §4: a DECLARED value is a first-class domain member — valid even
    // when no condition names it (the F2 default arm that appears in no
    // `facet == value` atom), unless the current selection already binds this
    // facet to a different value (one facet holds at most one value). Undeclared
    // (condition-inferred) facets never populate `declared_values`, so the loop
    // above is the sole authority for them and their behavior is byte-identical
    // to the pre-ADR path.
    if model
        .declared_values
        .get(facet)
        .map_or(false, |values| values.contains(option))
    {
        return assignments
            .get(facet)
            .map_or(true, |bound| bound.as_str() == option);
    }

    false
}

fn classify_rejection_reason(
    model: &SelectionConstraintModel,
    selection_state: &SelectionState,
    assignments: &BTreeMap<String, String>,
    rejected_option: &SelectionDelta,
) -> RejectionReason {
    if rejected_option.facet.trim().is_empty() {
        return RejectionReason {
            code: E_SELECTION_UNKNOWN_FACET.to_string(),
            message: "Rejected facet is empty".to_string(),
            blocking_choices: BTreeMap::new(),
            hint: Some("Provide a non-empty facet ID".to_string()),
            unsat_core: None,
        };
    }
    if rejected_option.option.trim().is_empty() {
        return RejectionReason {
            code: E_SELECTION_INVALID_OPTION.to_string(),
            message: format!(
                "Rejected option is empty for facet '{}'",
                rejected_option.facet
            ),
            blocking_choices: BTreeMap::new(),
            hint: Some("Provide a non-empty option value".to_string()),
            unsat_core: None,
        };
    }

    if let Some(existing) = selection_state.context_tags.get(&rejected_option.facet) {
        if existing != &rejected_option.option {
            let mut blocking = BTreeMap::new();
            blocking.insert(rejected_option.facet.clone(), existing.clone());
            return RejectionReason {
                code: E_SELECTION_CONFLICT.to_string(),
                message: format!(
                    "Rejected option '{}' conflicts with immutable context value '{}'",
                    rejected_option.option, existing
                ),
                blocking_choices: blocking,
                hint: Some("Update context_tags or choose a compatible option".to_string()),
                unsat_core: None,
            };
        }
    }

    if let Some(existing) = selection_state.choices.get(&rejected_option.facet) {
        if existing != &rejected_option.option {
            let mut blocking = BTreeMap::new();
            blocking.insert(rejected_option.facet.clone(), existing.clone());
            return RejectionReason {
                code: E_SELECTION_CONFLICT.to_string(),
                message: format!(
                    "Facet '{}' is already selected as '{}'",
                    rejected_option.facet, existing
                ),
                blocking_choices: blocking,
                hint: Some("Clear the existing choice or keep the current value".to_string()),
                unsat_core: None,
            };
        }
    }

    let Some(domain) = model.facet_domains.get(&rejected_option.facet) else {
        return RejectionReason {
            code: E_SELECTION_UNKNOWN_FACET.to_string(),
            message: format!("Unknown selection facet '{}'", rejected_option.facet),
            blocking_choices: BTreeMap::new(),
            hint: Some("Choose a facet discovered from model conditions".to_string()),
            unsat_core: None,
        };
    };

    if !domain.contains(&rejected_option.option) {
        let mut options: Vec<String> = domain.iter().cloned().collect();
        options.sort();
        return RejectionReason {
            code: E_SELECTION_INVALID_OPTION.to_string(),
            message: format!(
                "Invalid option '{}' for facet '{}'",
                rejected_option.option, rejected_option.facet
            ),
            blocking_choices: BTreeMap::new(),
            hint: Some(format!("Valid options: {}", options.join(", "))),
            unsat_core: None,
        };
    }

    let valid_options = valid_options_for_facet(model, assignments, &rejected_option.facet, domain);
    if valid_options.contains(&rejected_option.option) {
        return RejectionReason {
            code: E_SELECTION_CONFLICT.to_string(),
            message: format!(
                "Option '{}' for facet '{}' is currently valid and should not be rejected",
                rejected_option.option, rejected_option.facet
            ),
            blocking_choices: BTreeMap::new(),
            hint: Some("Use apply_selection directly for valid options".to_string()),
            unsat_core: None,
        };
    }

    build_unsat_reason(
        selection_state,
        rejected_option.facet.clone(),
        rejected_option.option.clone(),
    )
}

fn build_unsat_reason(
    selection_state: &SelectionState,
    facet: String,
    option: String,
) -> RejectionReason {
    RejectionReason {
        code: E_SELECTION_UNSATISFIABLE.to_string(),
        message: format!(
            "Selection '{}'='{}' is unsatisfiable under current constraints",
            facet, option
        ),
        blocking_choices: selection_state.choices.clone(),
        hint: Some("Use get_selection_options to choose a compatible option first".to_string()),
        // Division-of-labor / legacy compiler-owned rejection: no solver core.
        // The labeled unsat core is populated by the interpreter/runtime solver
        // wrappers from the solver-owned MUS (ADR-0031 D3), not here.
        unsat_core: None,
    }
}

fn rejection_to_diagnostic(facet: String, option: String, reason: RejectionReason) -> Diagnostic {
    Diagnostic {
        code: reason.code,
        severity: DiagnosticSeverity::Error,
        message: reason.message,
        source_id: None,
        entity_path: Some(format!("selection.{}={}", facet, option)),
        hint: reason.hint,
    }
}

fn selection_options_ok(
    model_hash: String,
    scope: String,
    facet: String,
    valid_options: Vec<String>,
    default: Option<String>,
    declared_open: Option<bool>,
    pruned_options: Option<Vec<PrunedOptionReason>>,
    selection_state_hash: String,
) -> GetSelectionOptionsResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    GetSelectionOptionsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        facet,
        valid_options,
        default,
        declared_open,
        pruned_options,
        selection_state_hash,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn selection_options_failed(
    model_hash: String,
    scope: String,
    facet: String,
    selection_state_hash: String,
    diagnostics: Vec<Diagnostic>,
) -> GetSelectionOptionsResult {
    let diagnostics = diagnostics_report(diagnostics);
    GetSelectionOptionsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        scope,
        facet,
        valid_options: Vec::new(),
        default: None,
        declared_open: None,
        pruned_options: None,
        selection_state_hash,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn initialize_selection_state_ok(
    model_hash: String,
    scope: String,
    selection_state: SelectionState,
) -> InitializeSelectionStateResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    InitializeSelectionStateResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        selection_state: Some(selection_state),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn initialize_selection_state_failed(
    model_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> InitializeSelectionStateResult {
    let diagnostics = diagnostics_report(diagnostics);
    InitializeSelectionStateResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        scope,
        selection_state: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn apply_selection_ok(
    model_hash: String,
    scope: String,
    selection_state: SelectionState,
) -> ApplySelectionResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    ApplySelectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        selection_state: Some(selection_state),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn apply_selection_failed(
    model_hash: String,
    scope: String,
    diagnostics: Vec<Diagnostic>,
) -> ApplySelectionResult {
    let diagnostics = diagnostics_report(diagnostics);
    ApplySelectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        scope,
        selection_state: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn explain_rejection_failed(
    model_hash: String,
    scope: String,
    facet: String,
    option: String,
    rejection: RejectionReason,
) -> ExplainRejectionResult {
    let diagnostics = diagnostics_report(vec![Diagnostic {
        code: rejection.code.clone(),
        severity: DiagnosticSeverity::Error,
        message: rejection.message.clone(),
        source_id: None,
        entity_path: Some(format!("selection.{}={}", facet, option)),
        hint: rejection.hint.clone(),
    }]);
    ExplainRejectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        scope,
        facet,
        option,
        rejection,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn resolve_ok(
    model_hash: String,
    scope: String,
    selection_state_hash: String,
    context_tags: BTreeMap<String, String>,
    choices: BTreeMap<String, String>,
    defaulted_choices: BTreeMap<String, String>,
    resolved_component_dependencies: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    resolved_artifacts: BTreeMap<String, crate::schema::Artifact>,
    resolve_hash: String,
    resolved_output: serde_json::Value,
) -> ResolveResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    ResolveResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        selection_state_hash,
        resolve_hash: Some(resolve_hash),
        resolved_output: Some(resolved_output),
        context_tags,
        choices,
        defaulted_choices,
        resolved_component_dependencies,
        resolved_artifacts,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn resolve_failed(
    model_hash: String,
    scope: String,
    selection_state_hash: String,
    context_tags: BTreeMap<String, String>,
    choices: BTreeMap<String, String>,
    defaulted_choices: BTreeMap<String, String>,
    resolved_component_dependencies: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    resolved_artifacts: BTreeMap<String, crate::schema::Artifact>,
    diagnostics: Vec<Diagnostic>,
) -> ResolveResult {
    let diagnostics = diagnostics_report(diagnostics);
    ResolveResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        scope,
        selection_state_hash,
        resolve_hash: None,
        resolved_output: None,
        context_tags,
        choices,
        defaulted_choices,
        resolved_component_dependencies,
        resolved_artifacts,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn export_resolved_ok(
    model_hash: String,
    scope: String,
    resolve_hash: Option<String>,
    generated_artifacts: GeneratedArtifacts,
) -> ExportResolvedResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    ExportResolvedResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        resolve_hash,
        generated_artifacts: Some(generated_artifacts),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
        tool_version: Some(crate::provenance_sidecar::tool_version().to_string()),
    }
}

fn export_resolved_failed(
    model_hash: String,
    scope: String,
    resolve_hash: Option<String>,
    diagnostics: Vec<Diagnostic>,
) -> ExportResolvedResult {
    let diagnostics = diagnostics_report(diagnostics);
    ExportResolvedResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        scope,
        resolve_hash,
        generated_artifacts: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
        tool_version: Some(crate::provenance_sidecar::tool_version().to_string()),
    }
}

fn export_software_bom_ok(
    model_hash: String,
    scope: String,
    resolve_hash: Option<String>,
    bom_hash: String,
    software_bom: SoftwareBomV1,
) -> ExportSoftwareBomResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    ExportSoftwareBomResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        resolve_hash,
        bom_hash: Some(bom_hash),
        software_bom: Some(software_bom),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
        tool_version: Some(crate::provenance_sidecar::tool_version().to_string()),
    }
}

fn export_software_bom_failed(
    model_hash: String,
    scope: String,
    resolve_hash: Option<String>,
    bom_hash: Option<String>,
    diagnostics: Vec<Diagnostic>,
) -> ExportSoftwareBomResult {
    let diagnostics = diagnostics_report(diagnostics);
    ExportSoftwareBomResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        scope,
        resolve_hash,
        bom_hash,
        software_bom: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
        tool_version: Some(crate::provenance_sidecar::tool_version().to_string()),
    }
}

/// Pull the tag name out of a condition-eval "Missing tag '<tag>' referenced in
/// condition" cause (`compiler/src/conditions/eval.rs`). The message is wrapped
/// by the resolver, so scan the whole `anyhow` chain and return the first tag
/// found between the single quotes. `None` when no such cause exists.
fn extract_missing_tag(err: &anyhow::Error) -> Option<String> {
    const MARKER: &str = "Missing tag '";
    for cause in err.chain() {
        let text = cause.to_string();
        if let Some(start) = text.find(MARKER) {
            let rest = &text[start + MARKER.len()..];
            if let Some(end) = rest.find('\'') {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Resolve-error mapper that is aware of declared facets (ADR-0047 §5). When the
/// underlying failure is an unbound tag an active condition needs, AND that tag
/// is a DECLARED facet with NO default, emit the precise `E_RESOLVE_FACET_UNBOUND`
/// naming the facet and its declared domain — the model is satisfiable once the
/// facet is bound, so this is a usage error, not the generic "unsatisfiable"
/// (`E_RESOLVE_CONTEXT_UNSATISFIED`) fold. Every other failure — including an
/// unbound *undeclared* facet, or a declared facet that DOES have a default
/// (which the auto-bind seeds, so it never reaches here) — defers to the legacy
/// `map_resolve_error`, keeping pre-ADR behavior byte-identical.
fn map_resolve_error_with_facets(
    err: anyhow::Error,
    facets: &HashMap<String, crate::schema::Facet>,
) -> Diagnostic {
    if let Some(tag) = extract_missing_tag(&err) {
        if let Some(facet) = facets.get(&tag) {
            if facet.default.is_none() {
                let domain = facet.values.join(", ");
                return Diagnostic {
                    code: E_RESOLVE_FACET_UNBOUND.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Declared facet '{tag}' is unbound and has no default, but an active \
                         condition requires it; declared domain: [{domain}]"
                    ),
                    source_id: None,
                    entity_path: None,
                    hint: Some(format!(
                        "Select facet '{tag}' (or set it in context_tags), or add a default to \
                         its declaration"
                    )),
                };
            }
        }
    }
    map_resolve_error(err)
}

fn map_resolve_error(err: anyhow::Error) -> Diagnostic {
    let message = err.to_string();
    let lowered = message.to_ascii_lowercase();

    if lowered.contains("scope selector") || lowered.contains("unrecognized scope selector") {
        return Diagnostic {
            code: E_RESOLVE_SCOPE_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message,
            source_id: None,
            entity_path: None,
            hint: Some(
                "Use scope values like component:<id>, platform:<id>, platform:all, or all"
                    .to_string(),
            ),
        };
    }

    let context_unsatisfied = err.chain().any(|cause| {
        let text = cause.to_string();
        text.contains("Failed to evaluate condition")
            || text.contains("Variable identifier is not bound")
            || text.contains("Unknown variable identifier")
    });
    if context_unsatisfied {
        return Diagnostic {
            code: E_RESOLVE_CONTEXT_UNSATISFIED.to_string(),
            severity: DiagnosticSeverity::Error,
            message,
            source_id: None,
            entity_path: None,
            hint: Some(
                "Provide all required selection choices/context_tags for active conditions"
                    .to_string(),
            ),
        };
    }

    Diagnostic {
        code: E_RESOLVE_FAILED.to_string(),
        severity: DiagnosticSeverity::Error,
        message,
        source_id: None,
        entity_path: None,
        hint: Some(
            "Ensure model handle, scope, and selection state are valid before resolve".to_string(),
        ),
    }
}

fn validate_manifest(manifest: &ir::CmpManifest) -> std::result::Result<(), String> {
    if manifest.schema_version != CMP_MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "Manifest schema_version {} is unsupported (expected {})",
            manifest.schema_version, CMP_MANIFEST_SCHEMA_VERSION
        ));
    }
    if manifest.ir_format_version != IR_FORMAT_VERSION {
        return Err(format!(
            "Manifest ir_format_version {} is unsupported (expected {})",
            manifest.ir_format_version, IR_FORMAT_VERSION
        ));
    }
    if manifest.hash_algo != CMP_HASH_ALGO {
        return Err(format!(
            "Manifest hash_algo '{}' is unsupported (expected '{}')",
            manifest.hash_algo, CMP_HASH_ALGO
        ));
    }
    if manifest.canonicalization_version != CMP_CANONICALIZATION_VERSION {
        return Err(format!(
            "Manifest canonicalization_version {} is unsupported (expected {})",
            manifest.canonicalization_version, CMP_CANONICALIZATION_VERSION
        ));
    }
    if manifest.model_hash != manifest.config_hash {
        return Err(format!(
            "Manifest model_hash '{}' does not match config_hash '{}'",
            manifest.model_hash, manifest.config_hash
        ));
    }
    if manifest.index_ref.trim().is_empty() {
        return Err("Manifest index_ref is empty".to_string());
    }
    if manifest.chunk_set_ref.trim().is_empty() {
        return Err("Manifest chunk_set_ref is empty".to_string());
    }
    Ok(())
}

fn resolve_ref(base_dir: &Path, reference: &str) -> PathBuf {
    let path = Path::new(reference);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push(hex_char(b >> 4));
        out.push(hex_char(b & 0x0f));
    }
    out
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => unreachable!("hex nibble out of range"),
    }
}

fn open_model_ok(model_hash: String, model_handle: ModelHandle) -> OpenModelResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    OpenModelResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash: Some(model_hash),
        model_handle: Some(model_handle),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn open_model_failed(model_hash: Option<String>, diagnostics: Vec<Diagnostic>) -> OpenModelResult {
    let diagnostics = diagnostics_report(diagnostics);
    OpenModelResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        model_handle: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn diagnostics_report(diagnostics: Vec<Diagnostic>) -> DiagnosticsReport {
    let error_count = diagnostics
        .iter()
        .filter(|diag| diag.severity == DiagnosticSeverity::Error)
        .count() as u32;
    let warning_count = diagnostics
        .iter()
        .filter(|diag| diag.severity == DiagnosticSeverity::Warning)
        .count() as u32;

    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics,
        error_count,
        warning_count,
    }
}
