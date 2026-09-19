// SPDX-License-Identifier: BUSL-1.1
//
// The one-shot `cfx resolve` pipeline (ADR-0042 §1/§2).
//
// This module composes the SAME entry points the interpreter's
// `src/cli_adapter.rs` dispatches to — `compiler::loader_api`'s `open_model`,
// `initialize_selection_state`, `apply_selection` and `export_resolved`, plus
// `session_compose::resolve` — into a single in-process
// open->select->resolve->export sequence, then writes the exported snapshot to
// a directory. It reimplements none of the resolution/selection/export
// semantics (ADR-0042 §2).
//
// `cfx` holds no decision logic: inference and resolve go through
// `session_compose` (ADR-0057 §D6, configflux-secb.3). `resolve` used to call
// the compiler's resolve entry point directly, bypassing that seam, on the
// reasoning that the solver only GATED satisfiability and so added nothing to
// the bytes. That stopped being true when resolve gained inferred binding: the
// implication a selection entails is now part of the composed output, and a
// surface that skipped the seam would default a facet the constraints had
// already decided — the disagreement between `cfx options` and `cfx resolve`
// that ADR-0057 §D6 exists to remove.
//
// Byte-identity with the interpreter envelope path (ADR-0042 §5) is therefore
// stronger than before, not weaker: both now reach the SAME
// `session_compose::resolve`, so they cannot diverge on selection semantics at
// all. `cfx` still has no direct `//solver` edge — it reaches the solver only
// through the seam (ADR-0003 §2 amendment) — and still calls the cudd-free
// `compiler` library directly for the loader-only steps that carry no decision
// (`open_model`, `initialize_selection_state`, `apply_selection`,
// `export_resolved`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use compiler::loader_api::{
    apply_selection, export_resolved, initialize_selection_state, open_model,
    ApplySelectionRequest, ExportResolvedRequest, InitializeSelectionStateRequest, ModelHandle,
    OpenModelRequest, ResolveFromSelectionRequest, ResolveResult, SelectionDelta, SelectionState,
    EXPORT_PROFILE_CPP_EARLY_BINDING_V1, E_RESOLVE_CONTEXT_UNSATISFIED, E_SELECTION_CONFLICT,
    E_SELECTION_INVALID_OPTION, E_SELECTION_UNSATISFIABLE,
};
use compiler::product_api::{DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION};

use crate::manifest::Cell;
use crate::render;

/// Scope used when nothing pins one: `all` is the compiler resolver's
/// whole-model selector (`compiler::resolver::parse_scope_selectors`).
pub(crate) const DEFAULT_SCOPE: &str = "all";

/// Where one resolution's `(scope, context_tags, base_choices)` triple comes
/// from (ADR-0059 D2). The three spellings differ ONLY in where those three
/// values are read; everything downstream — apply, resolve, export, write — is
/// the same code, which is what makes "the manifest path is the selection-file
/// path" true by construction rather than by two branches agreeing.
pub enum CellSource<'a> {
    /// Neither a manifest nor a selection file: the whole-model default scope,
    /// with choices coming only from `--select` flags.
    None,
    SelectionFile(&'a Path),
    Manifest(&'a Cell),
}

impl<'a> CellSource<'a> {
    /// Promote the optional `--selection-file` into a source.
    pub fn from_selection_file(path: Option<&'a Path>) -> Self {
        match path {
            Some(path) => CellSource::SelectionFile(path),
            None => CellSource::None,
        }
    }

    /// The `(scope, context_tags, base_choices)` this source supplies.
    fn inputs(
        &self,
    ) -> Result<(String, BTreeMap<String, String>, BTreeMap<String, String>), PipelineError> {
        match self {
            CellSource::None => Ok((DEFAULT_SCOPE.to_string(), BTreeMap::new(), BTreeMap::new())),
            CellSource::SelectionFile(path) => {
                let state = crate::selection_input::read_selection_file(path)?;
                Ok((state.scope, state.context_tags, state.choices))
            }
            CellSource::Manifest(cell) => Ok((
                cell.scope.clone(),
                cell.context_tags.clone(),
                cell.choices.clone(),
            )),
        }
    }

    /// The `<selection>` label for a resolve that pins no explicit choice.
    ///
    /// A manifest cell HAS a name for its target, and the reference resolver
    /// has always used it (`examples/resolve_environment.sh` `selection_label`),
    /// so a choice-free environment lands on its own name rather than on a
    /// `default` that every choice-free environment would share. This is the
    /// single behavioural difference between the manifest and selection-file
    /// paths (ADR-0059 D2).
    fn label_fallback(&self) -> &str {
        match self {
            CellSource::Manifest(cell) => &cell.environment,
            _ => DEFAULT_SELECTION_LABEL,
        }
    }
}

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
    /// The originating diagnostic's `(code, message)`, kept SEPARATE from the
    /// folded `message` line above. `cfx diff`'s `rejections[]` reports the two
    /// as distinct JSON fields (ADR-0059 M6), and re-splitting the folded line
    /// on its first `": "` would break on any message containing one. `None`
    /// for usage/IO failures no compiler diagnostic produced.
    pub diagnostic: Option<(String, String)>,
    /// The facet a rejected choice named, set ONLY where the rejection is
    /// `E_SELECTION_INVALID_OPTION` — the facet IS declared and the value is
    /// not in its domain — so the caller can point the user at the listing of
    /// what IS valid (configflux-ineg). `None` on every other path.
    ///
    /// Its own field for the same reason `unsatisfiable` is one: which guidance
    /// line a refusal earns is a property of the refusal, settled where the
    /// facet is still in hand, not re-derived by the printer. The only other
    /// source is the id embedded in `message`, and a hint parsed back out of
    /// diagnostic wording breaks the next time that wording is edited.
    pub invalid_option_facet: Option<String>,
}

impl PipelineError {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: crate::EXIT_USAGE,
            message: message.into(),
            unsatisfiable: false,
            diagnostic: None,
            invalid_option_facet: None,
        }
    }

    fn unsat(code: &str, detail: &str) -> Self {
        Self {
            exit_code: crate::EXIT_UNSAT,
            message: format!("{code}: {detail}"),
            unsatisfiable: true,
            diagnostic: Some((code.to_string(), detail.to_string())),
            invalid_option_facet: None,
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
///
/// One code moved class when `resolve` went through `session_compose`
/// (configflux-secb.3): `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE`, which the seam
/// raises when no usable `.ccm` is reachable, is a usage error (exit `2`) and
/// not the unsatisfiable case — nothing about the SELECTION is wrong, the model
/// is unreadable. In practice this is unreachable: `compile_model` emits the
/// `.ccm` sibling unconditionally on every successful compile, so a model
/// `cfx` can open always has one, which is why `cfx options` has been able to
/// require it since ADR-0030. Recorded here rather than in a CLI contract doc
/// because `cfx`'s exit codes are specified by ADR-0042 §3 and by this
/// function, and there is no separate cfx contract document to note it in.
///
/// BOTH branches carry `code: message`. The unsatisfiable branch used to drop
/// them and let `emit_error` print the "run cfx explain" guidance alone, which
/// left the user with a verdict and no reason. ADR-0054 §6 makes that
/// untenable: a resolve rejected by a declared policy carries the constraint's
/// id and its condition text in exactly this payload (configflux-4sjk), and a
/// surface that discards it cannot tell the user WHICH policy they broke.
pub(crate) fn classify(diagnostics: &DiagnosticsReport) -> PipelineError {
    let code = first_code(diagnostics);
    let message = first_message(diagnostics);
    if matches!(
        code,
        E_SELECTION_CONFLICT | E_SELECTION_UNSATISFIABLE | E_RESOLVE_CONTEXT_UNSATISFIED
    ) {
        PipelineError::unsat(code, &message)
    } else {
        PipelineError::usage(format!("{code}: {message}"))
    }
}

/// The shared `open -> init -> apply*` prefix that both `cfx resolve` and
/// `cfx options` compose (ADR-0042 §1/§2). After preparation, `state` is the
/// canonical `SelectionState` with every `--select`/selection-file choice
/// applied; `resolve` feeds it to `session_compose::resolve`, `options` queries
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
    source: &CellSource,
    selects: &[SelectPair],
) -> Result<OpenedModel, PipelineError> {
    let handle = open(model)?;
    let (scope, context_tags, choices) = cell_inputs(source, selects)?;
    let base_state = init_state(&handle, &scope, &context_tags)?;
    Ok(OpenedModel {
        handle,
        scope,
        context_tags,
        choices,
        base_state,
    })
}

/// Open a compiled model and return its handle, which carries the `model_hash`.
///
/// Split out of `open_and_init` (configflux-dkmm.5) because `cfx diff` opens
/// each side ONCE and resolves every cell against that one handle. Two things
/// follow from that and neither is an optimization: the report's
/// `base_model_hash`/`head_model_hash` exist even when every cell is rejected,
/// and an unreadable package is a usage error raised before any cell is
/// attempted rather than as a side effect of whichever cell happened to run
/// first.
pub fn open(model: &Path) -> Result<ModelHandle, PipelineError> {
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
    opened
        .model_handle
        .ok_or_else(|| PipelineError::usage("open returned no model handle"))
}

/// The `(scope, context_tags, choices)` one cell resolves with.
///
/// Final choice set: source choices first, then `--select` flags override
/// (ADR-0042 §2). A merged map means no facet is applied twice, so an override
/// never self-conflicts; iteration is `BTreeMap`-sorted for determinism.
#[allow(clippy::type_complexity)]
fn cell_inputs(
    source: &CellSource,
    selects: &[SelectPair],
) -> Result<(String, BTreeMap<String, String>, BTreeMap<String, String>), PipelineError> {
    let (scope, context_tags, base_choices) = source.inputs()?;
    let mut choices: BTreeMap<String, String> = base_choices;
    for pair in selects {
        choices.insert(pair.facet.clone(), pair.option.clone());
    }
    Ok((scope, context_tags, choices))
}

/// `initialize_selection_state` for one `(scope, context_tags)` — the base
/// state, with no choice applied yet.
fn init_state(
    handle: &ModelHandle,
    scope: &str,
    context_tags: &BTreeMap<String, String>,
) -> Result<SelectionState, PipelineError> {
    let init = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        context_tags: context_tags.clone(),
    });
    if init.status != OperationStatus::Ok {
        return Err(classify(&init.diagnostics));
    }
    init.selection_state
        .ok_or_else(|| PipelineError::usage("init-selection-state returned no state"))
}

/// Fold every choice onto a base state with `apply_selection`.
fn apply_all(
    handle: &ModelHandle,
    scope: &str,
    mut state: SelectionState,
    choices: &BTreeMap<String, String>,
) -> Result<SelectionState, PipelineError> {
    for (facet, option) in choices {
        let applied = apply_selection(ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: scope.to_string(),
            selection_state: state.clone(),
            selection_delta: SelectionDelta {
                facet: facet.clone(),
                option: option.clone(),
            },
        });
        if applied.status != OperationStatus::Ok {
            let mut err = classify(&applied.diagnostics);
            // This loop is the ONE place the refused facet is still in hand:
            // `classify` is handed diagnostics and nothing else, and past it the
            // id survives only inside the message text (configflux-ineg).
            if first_code(&applied.diagnostics) == E_SELECTION_INVALID_OPTION {
                err.invalid_option_facet = Some(facet.clone());
            }
            return Err(err);
        }
        state = applied
            .selection_state
            .ok_or_else(|| PipelineError::usage("apply-selection returned no state"))?;
    }
    Ok(state)
}

/// Resolve ONE cell against an ALREADY-OPEN model and return the result —
/// writing nothing (ADR-0059 D4).
///
/// This is `run_cell` without its export-and-write tail. `cfx diff` compares
/// two resolutions in memory: it must create no file at all, so it must not
/// reach `export_resolved`, and it needs the `ResolveResult` itself rather than
/// the lineage summary a write produces. The `init -> apply* -> resolve`
/// sequence is the SAME composition `prepare`/`run_cell` use, through the same
/// three helpers, so the two paths cannot drift on selection semantics.
pub fn resolve_only(
    handle: &ModelHandle,
    source: &CellSource,
    selects: &[SelectPair],
) -> Result<ResolveResult, PipelineError> {
    let (scope, context_tags, choices) = cell_inputs(source, selects)?;
    let base_state = init_state(handle, &scope, &context_tags)?;
    let state = apply_all(handle, &scope, base_state, &choices)?;

    let resolved = session_compose::resolve(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope,
        selection_state: state,
        implied_choices: Default::default(),
    });
    if resolved.status != OperationStatus::Ok {
        return Err(classify(&resolved.diagnostics));
    }
    Ok(resolved)
}

/// Compose the `open -> init -> apply*` prefix shared by `cfx resolve` and
/// `cfx options`. `selects` are the parsed `--select` pairs; they layer on top
/// of any `choices` in the selection file (file first, flags override —
/// ADR-0042 §2 / plan §2). Reuses the SAME `compiler::loader_api` calls the
/// interpreter's `cli_adapter.rs` routes to; reimplements no selection
/// semantics.
pub fn prepare(
    model: &Path,
    source: &CellSource,
    selects: &[SelectPair],
) -> Result<PreparedSelection, PipelineError> {
    let OpenedModel {
        handle,
        scope,
        context_tags,
        choices,
        base_state,
    } = open_and_init(model, source, selects)?;

    // --- apply each choice -------------------------------------------------
    let state = apply_all(&handle, &scope, base_state, &choices)?;

    Ok(PreparedSelection {
        handle,
        scope,
        context_tags,
        choices,
        state,
    })
}

/// The `<selection>` label used when a resolve pins no explicit choice. `cfx`
/// has no environment name to fall back on (the reference resolver does, and
/// keeps using it), so the choice-free target is simply `default`.
const DEFAULT_SELECTION_LABEL: &str = "default";

/// Restrict a name component to the file-safe token set the snapshot naming
/// convention uses (`docs/service-integration-guide.md` §The deployment bundle,
/// and `examples/resolve_environment.sh`): every character outside
/// `[A-Za-z0-9._-]` becomes `_`.
fn sanitize(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The `<root>` component of a snapshot name and of a matrix cell directory:
/// the scope root. `all` for the whole-model scope, the component name for
/// `component:<name>`, otherwise the whole scope string sanitized. One
/// definition, two callers, matching `examples/resolve_environment.sh`'s
/// `scope_root` — the reference script is the layout oracle (ADR-0059 M1).
pub(crate) fn scope_root(scope: &str) -> String {
    if scope == DEFAULT_SCOPE {
        DEFAULT_SCOPE.to_string()
    } else if let Some(component) = scope.strip_prefix("component:") {
        sanitize(component)
    } else {
        sanitize(scope)
    }
}

/// The snapshot file name for a resolve: `resolve_result.<root>.<selection>.json`
/// (configflux-dkmm.1, ADR-0042 amendment).
///
/// `<root>` is the scope root — `all` for the whole-model scope, the component
/// name for `component:<name>`, otherwise the whole scope string sanitized.
/// `<selection>` is the EXPLICIT choices (never the auto-bound
/// `defaulted_choices`, which are provenance, not identity) in sorted-facet
/// order, values joined with `-`; a choice-free resolve uses `label_fallback`.
///
/// Pure and total: the result is always a single path component from the safe
/// token set, so it can never escape `--out`, and it never ends in `_` (the
/// artifact the reference script's `echo | tr` produced by sanitizing a
/// trailing newline).
fn snapshot_file_name(
    scope: &str,
    choices: &BTreeMap<String, String>,
    label_fallback: &str,
) -> String {
    let root = scope_root(scope);
    let selection = if choices.is_empty() {
        sanitize(label_fallback)
    } else {
        // BTreeMap iteration is sorted by facet, so the label is deterministic.
        sanitize(
            &choices
                .values()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("-"),
        )
    };
    format!("resolve_result.{root}.{selection}.json")
}

/// Run the whole `open -> init -> apply* -> resolve -> export -> write`
/// pipeline for ONE cell — which is what every `cfx resolve` is: a
/// selection-file cell, a flags-only cell, or one manifest cell of a matrix.
pub fn run_cell(
    model: &Path,
    source: &CellSource,
    selects: &[SelectPair],
    out_dir: &Path,
) -> Result<ResolveOutcome, PipelineError> {
    let PreparedSelection {
        handle,
        scope,
        choices,
        state,
        ..
    } = prepare(model, source, selects)?;

    // --- resolve -----------------------------------------------------------
    let resolved = session_compose::resolve(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.clone(),
        selection_state: state.clone(),
        implied_choices: Default::default(),
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

    // --- write the resolved snapshot itself --------------------------------
    // The `ResolveResult` envelope is what a Pattern 1 service reads at
    // startup; before configflux-dkmm.1 it existed only on `--format json`
    // stdout, so every integrator redirected it into a file by hand. Written
    // through the SAME `render::snapshot_bytes` `--format json` uses, so the
    // file is byte-identical to that stdout — and therefore to the interpreter
    // `resolve` response — by construction. It lands only after resolve AND
    // export have succeeded, so a rejected selection still writes nothing
    // (the exit-3 no-partial-output contract, ADR-0042 §3).
    let snapshot_name = snapshot_file_name(&scope, &choices, source.label_fallback());
    let snapshot_path = safe_join(out_dir, &snapshot_name)?;
    if let Some(parent) = snapshot_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            PipelineError::usage(format!(
                "unable to create output directory '{}' ({err})",
                parent.display()
            ))
        })?;
    }
    let snapshot = render::snapshot_bytes(&resolved).map_err(|err| {
        PipelineError::usage(format!("unable to serialize the resolved snapshot ({err})"))
    })?;
    std::fs::write(&snapshot_path, &snapshot).map_err(|err| {
        PipelineError::usage(format!(
            "unable to write '{}' ({err})",
            snapshot_path.display()
        ))
    })?;
    written.push(snapshot_name);

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

/// One cell's place in a matrix run: where it wrote (relative to `--out`) and
/// what happened. `result` is `Err` only for the exit-`3` unsatisfiable class —
/// a usage/IO failure aborts the whole run and never becomes a cell status.
pub struct CellOutcome {
    pub environment: String,
    pub scope: String,
    /// `<environment>/<root>` — the directory this cell wrote under, relative
    /// to `--out`. The renderer prefixes the `wrote:` lineage with it so every
    /// printed path can be copied straight out of stdout.
    pub prefix: String,
    pub result: Result<ResolveOutcome, PipelineError>,
}

/// Resolve every cell of a matrix into `<out>/<environment>/<root>/` — the
/// layout `examples/resolve_environment.sh --matrix` writes (ADR-0059 D2/M1).
///
/// EVERY cell is attempted. A cell rejected as unsatisfiable is recorded and
/// the loop continues, because "which of my targets broke" is the question the
/// matrix exists to answer and stopping at the first one refuses to answer it;
/// a rejected cell writes nothing, since `run_cell` creates no directory until
/// resolve AND export have both succeeded. A usage/IO-class failure is a
/// different animal — the model is unreadable or the disk is full, so every
/// remaining cell would fail the same way — and aborts immediately with the
/// error, per ADR-0059 D2. The split is `classify()`'s and is not restated here.
///
/// Cell directories are safe by construction: `environment` was validated
/// against `[A-Za-z0-9._-]+` at manifest load and `scope_root` sanitizes to the
/// same set, so neither can escape `out_dir`.
pub fn run_cells(
    model: &Path,
    cells: &[Cell],
    selects: &[SelectPair],
    out_dir: &Path,
) -> Result<Vec<CellOutcome>, PipelineError> {
    let mut outcomes = Vec::with_capacity(cells.len());
    for cell in cells {
        let root = scope_root(&cell.scope);
        let cell_out = out_dir.join(&cell.environment).join(&root);
        let result = match run_cell(model, &CellSource::Manifest(cell), selects, &cell_out) {
            Ok(outcome) => Ok(outcome),
            Err(err) if err.unsatisfiable => Err(err),
            Err(err) => return Err(err),
        };
        outcomes.push(CellOutcome {
            environment: cell.environment.clone(),
            scope: cell.scope.clone(),
            prefix: format!("{}/{root}", cell.environment),
            result,
        });
    }
    Ok(outcomes)
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
    fn snapshot_file_name_sanitizes() {
        assert_eq!(
            snapshot_file_name("component:thermal_control", &BTreeMap::new(), "default"),
            "resolve_result.thermal_control.default.json"
        );
        let mut choices = BTreeMap::new();
        choices.insert("a".to_string(), "1.0".to_string());
        choices.insert("b".to_string(), "x y".to_string());
        assert_eq!(
            snapshot_file_name("all", &choices, "default"),
            "resolve_result.all.1.0-x_y.json"
        );
        assert_eq!(
            snapshot_file_name("platform:all", &BTreeMap::new(), "default"),
            "resolve_result.platform_all.default.json"
        );
        // ADR-0059 D2: a manifest cell falls back to its environment name.
        assert_eq!(
            snapshot_file_name("component:webapp", &BTreeMap::new(), "robot-alpha"),
            "resolve_result.webapp.robot-alpha.json"
        );
    }

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
    fn classify_facet_unbound_is_usage_exit_2_not_unsat() {
        // ADR-0047 §6: E_RESOLVE_FACET_UNBOUND is a valid-input-but-
        // underspecified USAGE error (exit 2), deliberately NOT in the
        // unsatisfiable family — the model IS satisfiable once the facet is
        // bound. This also governs `cfx explain`, which routes through the same
        // resolve path + `classify`: a facet-unbound resolve surfaces the
        // precise message via the usage/exit-2 branch, never "unsatisfiable".
        let report = DiagnosticsReport {
            schema_version: PRODUCT_SCHEMA_VERSION,
            diagnostics: vec![compiler::product_api::Diagnostic {
                code: "E_RESOLVE_FACET_UNBOUND".to_string(),
                severity: compiler::product_api::DiagnosticSeverity::Error,
                message: "Declared facet 'region' is unbound and has no default, but an active \
                          condition requires it; declared domain: [eu, us]"
                    .to_string(),
                source_id: None,
                entity_path: None,
                hint: None,
            }],
            error_count: 1,
            warning_count: 0,
        };
        let err = classify(&report);
        assert_eq!(err.exit_code, crate::EXIT_USAGE);
        assert!(!err.unsatisfiable, "facet-unbound must not be the unsat family");
        assert!(err.message.contains("region"), "message names the facet: {}", err.message);
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
