// SPDX-License-Identifier: BUSL-1.1

pub fn open_model(request: OpenModelRequest) -> OpenModelResult {
    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return open_model_failed(
            None,
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

    let manifest_path = PathBuf::from(&request.cmp_manifest_ref);
    let manifest = match ir::load_cmp_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(err) => {
            return open_model_failed(
                None,
                vec![Diagnostic {
                    code: E_LOADER_MANIFEST_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: Some(request.cmp_manifest_ref),
                    entity_path: None,
                    hint: Some("Provide a valid compiled model package manifest JSON".to_string()),
                }],
            );
        }
    };

    if let Err(message) = validate_manifest(&manifest) {
        return open_model_failed(
            Some(manifest.model_hash.clone()),
            vec![Diagnostic {
                code: E_LOADER_MANIFEST_INCONSISTENT.to_string(),
                severity: DiagnosticSeverity::Error,
                message,
                source_id: Some(request.cmp_manifest_ref),
                entity_path: None,
                hint: Some("Re-emit the CMP so manifest/index metadata are aligned".to_string()),
            }],
        );
    }

    let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));
    let index_path = resolve_ref(manifest_dir, &manifest.index_ref);
    let chunk_dir = resolve_ref(manifest_dir, &manifest.chunk_set_ref);

    let index = match ir::load_index(&index_path) {
        Ok(index) => index,
        Err(err) => {
            return open_model_failed(
                Some(manifest.model_hash.clone()),
                vec![Diagnostic {
                    code: E_LOADER_INDEX_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: Some(index_path.to_string_lossy().into_owned()),
                    entity_path: None,
                    hint: Some(
                        "Ensure CMP index_ref points at a valid index.cfir.json".to_string(),
                    ),
                }],
            );
        }
    };

    if index.format_version != manifest.ir_format_version {
        return open_model_failed(
            Some(manifest.model_hash.clone()),
            vec![Diagnostic {
                code: E_LOADER_MANIFEST_INCONSISTENT.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Manifest ir_format_version {} does not match index format_version {}",
                    manifest.ir_format_version, index.format_version
                ),
                source_id: Some(request.cmp_manifest_ref),
                entity_path: None,
                hint: Some("Re-emit the CMP so manifest/index metadata are aligned".to_string()),
            }],
        );
    }

    match index.compute_config_hash() {
        Ok(computed_hash) => {
            if computed_hash != index.config_hash {
                return open_model_failed(
                    Some(manifest.model_hash.clone()),
                    vec![Diagnostic {
                        code: E_LOADER_INDEX_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Index config_hash '{}' does not match computed hash '{}'",
                            index.config_hash, computed_hash
                        ),
                        source_id: Some(index_path.to_string_lossy().into_owned()),
                        entity_path: None,
                        hint: Some(
                            "Do not mutate emitted index files; recompile instead".to_string(),
                        ),
                    }],
                );
            }
        }
        Err(err) => {
            return open_model_failed(
                Some(manifest.model_hash.clone()),
                vec![Diagnostic {
                    code: E_LOADER_INDEX_INVALID.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: err.to_string(),
                    source_id: Some(index_path.to_string_lossy().into_owned()),
                    entity_path: None,
                    hint: Some("Ensure index.cfir.json is valid and complete".to_string()),
                }],
            );
        }
    }

    if index.config_hash != manifest.config_hash || index.config_hash != manifest.model_hash {
        return open_model_failed(
            Some(manifest.model_hash.clone()),
            vec![Diagnostic {
                code: E_LOADER_MANIFEST_INCONSISTENT.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Manifest model/config hash '{}'/'{}' does not match index config_hash '{}'",
                    manifest.model_hash, manifest.config_hash, index.config_hash
                ),
                source_id: Some(request.cmp_manifest_ref),
                entity_path: None,
                hint: Some("Re-emit the CMP so manifest/index metadata are aligned".to_string()),
            }],
        );
    }

    if let Err(err) = ir::verify_index_integrity(&index, &chunk_dir) {
        // A chunk whose content no longer hashes to its own name is a package
        // that was EDITED, and the remedy differs from the one an incomplete or
        // self-contradictory package needs — restoring a file that is already
        // there fixes nothing. The code stays the same: both are
        // E_LOADER_INDEX_INVALID.
        let hint = if err
            .downcast_ref::<ir::ChunkContentAddressMismatch>()
            .is_some()
        {
            "Do not mutate emitted chunk files; recompile instead"
        } else {
            "Ensure all chunk-<hash>.cfir files are present and unmodified"
        };
        return open_model_failed(
            Some(manifest.model_hash.clone()),
            vec![Diagnostic {
                code: E_LOADER_INDEX_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: err.to_string(),
                source_id: Some(chunk_dir.to_string_lossy().into_owned()),
                entity_path: None,
                hint: Some(hint.to_string()),
            }],
        );
    }

    if let Some(stats) = &manifest.stats {
        let expected = (
            index.chunks.len() as u32,
            index.chunks.len() as u32,
            index.definition_index.len() as u32,
            index.component_index.len() as u32,
            index.artifact_index.len() as u32,
        );
        let observed = (
            stats.source_count,
            stats.chunk_count,
            stats.definition_count,
            stats.component_count,
            stats.artifact_count,
        );
        if expected != observed {
            return open_model_failed(
                Some(manifest.model_hash.clone()),
                vec![Diagnostic {
                    code: E_LOADER_MANIFEST_INCONSISTENT.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Manifest stats {:?} do not match index stats {:?}",
                        observed, expected
                    ),
                    source_id: Some(request.cmp_manifest_ref),
                    entity_path: None,
                    hint: Some(
                        "Re-emit the CMP so manifest/index metadata are aligned".to_string(),
                    ),
                }],
            );
        }
    }

    // configflux-9hi2: advertise the sibling `.ccm` artifact directory the
    // product compile path now emits (ADR-0005 Amendment 1 §11 v2 multi-part;
    // ADR-0017 §2 load path). It is a sibling of the CMP manifest, mirroring
    // how `index_ref` / `chunk_set_ref` resolve relative to `manifest_dir`.
    // The path is advertised even if the dir is absent (a CMP emitted before
    // this change); the loader treats a missing dir as "no `.ccm`" per
    // `Ccm::load_from_cmp`'s nonexistent-path fallback.
    let ccm_ref = manifest_dir.join("ccm");

    open_model_ok(
        manifest.model_hash.clone(),
        ModelHandle {
            model_hash: manifest.model_hash,
            cmp_manifest_ref: request.cmp_manifest_ref,
            index_ref: index_path.to_string_lossy().into_owned(),
            chunk_set_ref: chunk_dir.to_string_lossy().into_owned(),
            ccm_ref: ccm_ref.to_string_lossy().into_owned(),
        },
    )
}

