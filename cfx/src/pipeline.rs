// SPDX-License-Identifier: BUSL-1.1
//
// The one-shot `cfx resolve` pipeline (ADR-0042 §1/§2).
//
// This module composes the SAME `compiler::loader_api` entry points the
// interpreter's `src/cli_adapter.rs` dispatches to (`open_model`,
// `initialize_selection_state`, `apply_selection`, `resolve_from_selection`,
// `export_resolved`) into a single in-process open->select->resolve->export
// sequence, then writes the exported snapshot to a directory. It reimplements
// none of the resolution/selection/export semantics (ADR-0042 §2).
//
// Byte-identical to the interpreter envelope path (ADR-0042 §5): the
// interpreter routes `select`/`resolve` through `solver_session`, whose only
// role is to GATE satisfiability — on a satisfiable selection the composed
// output comes verbatim from `apply_selection`/`resolve_from_selection`. `cfx`
// calls those same compiler functions directly (no solver/CUDD dependency; it
// depends on the cudd-free `compiler` default library), so for any satisfiable
// input the exported snapshot and `resolve_hash` match the interpreter's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use compiler::loader_api::{
    apply_selection, export_resolved, initialize_selection_state, open_model, resolve_from_selection,
    ApplySelectionRequest, ExportResolvedRequest, InitializeSelectionStateRequest, ModelHandle,
    OpenModelRequest, ResolveFromSelectionRequest, ResolveResult, SelectionDelta, SelectionState,
    EXPORT_PROFILE_CPP_EARLY_BINDING_V1, E_RESOLVE_CONTEXT_UNSATISFIED, E_SELECTION_CONFLICT,
    E_SELECTION_UNSATISFIABLE,
};
use compiler::product_api::{DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION};

/// Scope used when no `--selection-file` pins one: `all` is the compiler
/// resolver's whole-model selector (`compiler::resolver::parse_scope_selectors`).
const DEFAULT_SCOPE: &str = "all";

/// A `cfx resolve` failure, carrying the exit code the CLI must surface and a
/// single human line. `code` follows the ADR-0042 §3 contract: `2` for
/// usage/IO/loader errors, `3` for valid-input-but-unsatisfiable.
#[derive(Debug)]
pub struct PipelineError {
    pub exit_code: u8,
    pub message: String,
    /// Whether this is the unsatisfiable (exit `3`) case, so the caller can
    /// print the canonical "run: cfx explain ..." guidance line.
    pub unsatisfiable: bool,
}

impl PipelineError {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: crate::EXIT_USAGE,
            message: message.into(),
            unsatisfiable: false,
        }
    }

    fn unsat(message: impl Into<String>) -> Self {
        Self {
            exit_code: crate::EXIT_UNSAT,
            message: message.into(),
            unsatisfiable: true,
        }
    }
}

/// The successful outcome of a resolve: the hash lineage, the final selection
/// state hash, the resolved payload (for `--format json`), and the relative
/// paths written under `--out` (sorted).
pub struct ResolveOutcome {
    pub model_hash: String,
    pub selection_state_hash: String,
    pub resolve_hash: String,
    pub resolve_result: ResolveResult,
    pub written: Vec<String>,
}

/// A parsed `--select facet=option` pair.
#[derive(Debug)]
pub struct SelectPair {
    pub facet: String,
    pub option: String,
}

/// Parse a single `facet=option` argument. Rejects a missing `=`, an empty
/// facet, or an empty option — each an exit-`2` usage error naming the bad
/// argument (ADR-0042 §2 "reject malformed pairs with exit 2").
pub fn parse_select_pair(raw: &str) -> Result<SelectPair, PipelineError> {
    let Some((facet, option)) = raw.split_once('=') else {
        return Err(PipelineError::usage(format!(
            "invalid --select '{raw}': expected FACET=OPTION"
        )));
    };
    if facet.trim().is_empty() || option.trim().is_empty() {
        return Err(PipelineError::usage(format!(
            "invalid --select '{raw}': FACET and OPTION must both be non-empty"
        )));
    }
    Ok(SelectPair {
        facet: facet.to_string(),
        option: option.to_string(),
    })
}

/// Read and parse the `--selection-file` as the existing `SelectionState` JSON
/// shape (ADR-0042 §3). Only `scope`, `context_tags`, and `choices` are
/// consumed; the file's `selection_state_hash` is IGNORED and re-derived
/// canonically by the pipeline, so a hand-written file need not compute it.
fn read_selection_file(path: &Path) -> Result<SelectionState, PipelineError> {
    let bytes = std::fs::read(path).map_err(|err| {
        PipelineError::usage(format!(
            "unable to read --selection-file '{}' ({err})",
            path.display()
        ))
    })?;
    serde_json::from_slice::<SelectionState>(&bytes).map_err(|err| {
        PipelineError::usage(format!(
            "malformed --selection-file '{}' ({err})",
            path.display()
        ))
    })
}

/// The first diagnostic code in a failed operation's report, or a generic
/// fallback. Used to classify a compiler rejection into an exit code.
fn first_code(diagnostics: &DiagnosticsReport) -> &str {
    diagnostics
        .diagnostics
        .first()
        .map(|d| d.code.as_str())
        .unwrap_or("E_UNKNOWN")
}

/// The first diagnostic message, or a generic fallback.
fn first_message(diagnostics: &DiagnosticsReport) -> String {
    diagnostics
        .diagnostics
        .first()
        .map(|d| d.message.clone())
        .unwrap_or_else(|| "operation failed".to_string())
}

/// Classify a selection/resolve rejection: the constraint-conflict family
/// (`E_SELECTION_CONFLICT` / `E_SELECTION_UNSATISFIABLE` /
/// `E_RESOLVE_CONTEXT_UNSATISFIED`) is the valid-input-but-unsatisfiable case
/// (exit `3`, ADR-0042 §3 — the "run cfx explain" case); everything else
/// (unknown facet, invalid option, loader/schema faults) is a usage error
/// (exit `2`).
pub(crate) fn classify(diagnostics: &DiagnosticsReport) -> PipelineError {
    let code = first_code(diagnostics);
    let message = first_message(diagnostics);
    if matches!(
        code,
        E_SELECTION_CONFLICT | E_SELECTION_UNSATISFIABLE | E_RESOLVE_CONTEXT_UNSATISFIED
    ) {
        PipelineError::unsat(message)
    } else {
        PipelineError::usage(format!("{code}: {message}"))
    }
}

/// The shared `open -> init -> apply*` prefix that both `cfx resolve` and
/// `cfx options` compose (ADR-0042 §1/§2). After preparation, `state` is the
/// canonical `SelectionState` with every `--select`/selection-file choice
/// applied; `resolve` feeds it to `resolve_from_selection`, `options` queries
/// `get_selection_options` against it. `context_tags` and `choices` are carried
/// so `options` can mark which facets are pinned/selected vs open.
pub struct PreparedSelection {
    pub handle: ModelHandle,
    pub scope: String,
    pub context_tags: BTreeMap<String, String>,
    pub choices: BTreeMap<String, String>,
    pub state: SelectionState,
}

/// The `open -> init` prologue shared by every verb, BEFORE any choice is
/// applied. Carries the opened handle, the resolved scope/context tags, the
/// merged (file + `--select`) choice map, and the freshly-initialised base
/// `SelectionState` (no choices applied yet). `cfx resolve`/`options` fold every
/// choice onto it; `cfx explain` applies them one at a time to locate the
/// conflicting choice (configflux-2awb.3).
pub struct OpenedModel {
    pub handle: ModelHandle,
    pub scope: String,
    pub context_tags: BTreeMap<String, String>,
    pub choices: BTreeMap<String, String>,
    pub base_state: SelectionState,
}

/// Compose the `open -> init` prologue and merge the selection-file choices with
/// the `--select` flags (file first, flags override — ADR-0042 §2). Reuses the
/// SAME `compiler::loader_api` calls the interpreter's `cli_adapter.rs` routes
/// to; reimplements no selection semantics. Does NOT apply any choice — the
/// caller decides how (fold-all for resolve/options, one-at-a-time for explain).
pub fn open_and_init(
    model: &Path,
    selection_file: Option<&Path>,
    selects: &[SelectPair],
) -> Result<OpenedModel, PipelineError> {
    // --- selection-file (scope + context_tags + base choices) --------------
    let (scope, context_tags, base_choices) = match selection_file {
        Some(path) => {
            let state = read_selection_file(path)?;
            (state.scope, state.context_tags, state.choices)
        }
        None => (DEFAULT_SCOPE.to_string(), BTreeMap::new(), BTreeMap::new()),
    };

    // Final choice set: file choices first, then --select flags override
    // (ADR-0042 §2). A merged map means no facet is applied twice, so an
    // override never self-conflicts; iteration is BTreeMap-sorted for
    // determinism.
    let mut choices: BTreeMap<String, String> = base_choices;
    for pair in selects {
        choices.insert(pair.facet.clone(), pair.option.clone());
    }

    // --- open --------------------------------------------------------------
    let opened = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: model.to_string_lossy().into_owned(),
    });
    if opened.status != OperationStatus::Ok {
        return Err(PipelineError::usage(format!(
            "open failed — {}: {}",
            first_code(&opened.diagnostics),
            first_message(&opened.diagnostics)
        )));
    }
    let handle: ModelHandle = opened
        .model_handle
        .ok_or_else(|| PipelineError::usage("open returned no model handle"))?;

    // --- init selection state ---------------------------------------------
    let init = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.clone(),
        context_tags: context_tags.clone(),
    });
    if init.status != OperationStatus::Ok {
        return Err(classify(&init.diagnostics));
    }
    let base_state: SelectionState = init
        .selection_state
        .ok_or_else(|| PipelineError::usage("init-selection-state returned no state"))?;

    Ok(OpenedModel {
        handle,
        scope,
        context_tags,
        choices,
        base_state,
    })
}

/// Compose the `open -> init -> apply*` prefix shared by `cfx resolve` and
/// `cfx options`. `selects` are the parsed `--select` pairs; they layer on top
/// of any `choices` in the selection file (file first, flags override —
/// ADR-0042 §2 / plan §2). Reuses the SAME `compiler::loader_api` calls the
/// interpreter's `cli_adapter.rs` routes to; reimplements no selection
/// semantics.
pub fn prepare(
    model: &Path,
    selection_file: Option<&Path>,
    selects: &[SelectPair],
) -> Result<PreparedSelection, PipelineError> {
    let OpenedModel {
        handle,
        scope,
        context_tags,
        choices,
        base_state,
    } = open_and_init(model, selection_file, selects)?;

    // --- apply each choice -------------------------------------------------
    let mut state = base_state;
    for (facet, option) in &choices {
        let applied = apply_selection(ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: scope.clone(),
            selection_state: state.clone(),
            selection_delta: SelectionDelta {
                facet: facet.clone(),
                option: option.clone(),
            },
        });
        if applied.status != OperationStatus::Ok {
            return Err(classify(&applied.diagnostics));
        }
        state = applied
            .selection_state
            .ok_or_else(|| PipelineError::usage("apply-selection returned no state"))?;
    }

    Ok(PreparedSelection {
        handle,
        scope,
        context_tags,
        choices,
        state,
    })
}

/// Run the whole `open -> init -> apply* -> resolve -> export -> write`
/// pipeline for `cfx resolve`.
pub fn run(
    model: &Path,
    selection_file: Option<&Path>,
    selects: &[SelectPair],
    out_dir: &Path,
) -> Result<ResolveOutcome, PipelineError> {
    let PreparedSelection {
        handle,
        scope,
        state,
        ..
    } = prepare(model, selection_file, selects)?;

    // --- resolve -----------------------------------------------------------
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.clone(),
        selection_state: state.clone(),
    });
    if resolved.status != OperationStatus::Ok {
        return Err(classify(&resolved.diagnostics));
    }
    let resolve_hash = resolved
        .resolve_hash
        .clone()
        .ok_or_else(|| PipelineError::usage("resolve returned no resolve_hash"))?;

    // --- export ------------------------------------------------------------
    let exported = export_resolved(ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolved.clone(),
        profile: EXPORT_PROFILE_CPP_EARLY_BINDING_V1.to_string(),
    });
    if exported.status != OperationStatus::Ok {
        return Err(PipelineError::usage(format!(
            "export failed — {}: {}",
            first_code(&exported.diagnostics),
            first_message(&exported.diagnostics)
        )));
    }
    let artifacts = exported
        .generated_artifacts
        .ok_or_else(|| PipelineError::usage("export returned no generated artifacts"))?;

    // --- write the exported snapshot to --out ------------------------------
    let mut written = Vec::with_capacity(artifacts.files.len());
    for file in &artifacts.files {
        let target = safe_join(out_dir, &file.path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                PipelineError::usage(format!(
                    "unable to create output directory '{}' ({err})",
                    parent.display()
                ))
            })?;
        }
        std::fs::write(&target, file.contents.as_bytes()).map_err(|err| {
            PipelineError::usage(format!(
                "unable to write '{}' ({err})",
                target.display()
            ))
        })?;
        written.push(file.path.clone());
    }
    // Sorted relative paths — deterministic `wrote:` lineage (ADR-0042 §3).
    written.sort();

    Ok(ResolveOutcome {
        model_hash: resolved.model_hash.clone(),
        selection_state_hash: state.selection_state_hash.clone(),
        resolve_hash,
        resolve_result: resolved,
        written,
    })
}

/// Join an artifact's declared relative path under `out_dir`, rejecting any
/// absolute path or `..` traversal so a compiled model can never write outside
/// the requested output directory.
fn safe_join(out_dir: &Path, rel: &str) -> Result<PathBuf, PipelineError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir))
    {
        return Err(PipelineError::usage(format!(
            "refusing to write artifact with unsafe path '{rel}'"
        )));
    }
    Ok(out_dir.join(rel_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_select_pair_accepts_facet_option() {
        let pair = parse_select_pair("region=eu").expect("valid pair");
        assert_eq!(pair.facet, "region");
        assert_eq!(pair.option, "eu");
    }

    #[test]
    fn parse_select_pair_rejects_missing_equals() {
        let err = parse_select_pair("regioneu").expect_err("no '=' must fail");
        assert_eq!(err.exit_code, crate::EXIT_USAGE);
        assert!(err.message.contains("regioneu"), "message names bad arg");
    }

    #[test]
    fn parse_select_pair_rejects_empty_sides() {
        assert_eq!(
            parse_select_pair("=eu").unwrap_err().exit_code,
            crate::EXIT_USAGE
        );
        assert_eq!(
            parse_select_pair("region=").unwrap_err().exit_code,
            crate::EXIT_USAGE
        );
    }

    #[test]
    fn classify_selection_conflict_is_unsat_exit_3() {
        let report = DiagnosticsReport {
            schema_version: PRODUCT_SCHEMA_VERSION,
            diagnostics: vec![compiler::product_api::Diagnostic {
                code: E_SELECTION_CONFLICT.to_string(),
                severity: compiler::product_api::DiagnosticSeverity::Error,
                message: "conflict".to_string(),
                source_id: None,
                entity_path: None,
                hint: None,
            }],
            error_count: 1,
            warning_count: 0,
        };
        let err = classify(&report);
        assert_eq!(err.exit_code, crate::EXIT_UNSAT);
        assert!(err.unsatisfiable);
    }

    #[test]
    fn classify_unknown_facet_is_usage_exit_2() {
        let report = DiagnosticsReport {
            schema_version: PRODUCT_SCHEMA_VERSION,
            diagnostics: vec![compiler::product_api::Diagnostic {
                code: "E_SELECTION_UNKNOWN_FACET".to_string(),
                severity: compiler::product_api::DiagnosticSeverity::Error,
                message: "unknown".to_string(),
                source_id: None,
                entity_path: None,
                hint: None,
            }],
            error_count: 1,
            warning_count: 0,
        };
        let err = classify(&report);
        assert_eq!(err.exit_code, crate::EXIT_USAGE);
        assert!(!err.unsatisfiable);
    }

    #[test]
    fn safe_join_rejects_traversal_and_absolute() {
        let out = Path::new("/tmp/out");
        assert!(safe_join(out, "../escape").is_err());
        assert!(safe_join(out, "/etc/passwd").is_err());
        assert_eq!(
            safe_join(out, "generated/config.hpp").unwrap(),
            Path::new("/tmp/out/generated/config.hpp")
        );
    }
}
