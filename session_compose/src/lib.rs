// SPDX-License-Identifier: BUSL-1.1
//
// `session_compose` — the shared solver-composition crate. Wires
// `solver::Session` into the `options` / `select` (apply) / `resolve` /
// `explain` composition the product CLIs share (configflux-kkhc), extracted
// verbatim from the interpreter's former `solver_session` module, per ADR-0017
// (+ its compiler-delegated-`resolve` amendment), ADR-0031, and ADR-0030. It is
// the single seam permitted to depend on BOTH `solver` and `compiler` (ADR-0003
// §2 amendment, configflux-szwp; neither library imports the other), so
// `interpreter` and `cfx` obtain solver-authoritative decision content without a
// direct `//solver` edge — the position `solver_session.rs` held pre-extraction.
//
// Per-command behaviour:
//   * `options` → the **solver** decides the still-valid set
//     (`Session::valid_options`); the result envelope is built with the same
//     fields legacy emits, so only `valid_options` is solver-sourced and it is
//     byte-identical (both engines read the same `.ccm` symbols, ADR-0005 §3).
//   * `select` → the **solver** decides accept/reject and the `E_SELECTION_*`
//     code (`Session::apply`, §5 table); the next `SelectionState` is rebuilt
//     via the same `canonical_selection_state` recipe, so its hash matches.
//   * `resolve` → **compiler-delegated** (amendment): the solver gates
//     satisfiability, then the whole payload (nested `resolved_output` tree +
//     dependencies + artifacts + legacy-recipe `resolve_hash`) comes from
//     `compiler::loader_api::resolve_from_selection`. Byte-identical BY
//     CONSTRUCTION — both paths bottom out in `resolve_scoped`. The solver's
//     own flat resolve shape (configflux-i0ne) is never consulted.
//
// Lifecycle (ADR-0017 §3): ephemeral sessions, re-derived per CLI invocation
// via `load_ccm(ccm_ref)` → `new` → replay `choices`; `CuddBackend` is the
// wired backend (§1).
//
// CCM availability is a HARD PRECONDITION (ADR-0030, amends ADR-0017): a usable
// `.ccm` (non-empty ref, loadable artifact, populated symbol table) is required
// for every solver-owned selection decision. When no usable model is reachable,
// the wrapper FAILS CLOSED with a stable diagnostic instead of degrading to the
// legacy compiler path:
//   * `options`/`select` → `E_SELECTION_SOLVER_MODEL_UNAVAILABLE`
//   * `resolve`          → `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE`
// A solver fault on a solver-owned query likewise fails closed (ADR-0030 D4),
// and a solver REJECT that legacy would accept is a correctness incident
// surfaced as `E_SELECTION_ENGINE_DIVERGENCE` (ADR-0030 D3), never silently
// reconciled. The legitimate compiler delegations that remain — schema/empty
// validation, unconstrained (unknown) facets, idempotent/context-pinned applies,
// and the entire `resolve` envelope composition — are the permanent
// division-of-labor of ADR-0030 D5, not availability fallback. The only path
// that still routes a *modeled* decision to the compiler is the typed-rejection
// render delegation on `select` (the solver decided REJECT; the compiler renders
// the canonical diagnostic bytes), which is byte-fidelity, not fallback.

use compiler::loader_api::{
    apply_selection, canonical_selection_state, explain_rejection, get_selection_options,
    resolve_from_selection, ApplySelectionRequest, ApplySelectionResult, ConflictingConstraint,
    ConstraintFacet, ConstraintKind, ExplainRejectionRequest, ExplainRejectionResult,
    GetSelectionOptionsRequest, GetSelectionOptionsResult, RejectionReason,
    ResolveFromSelectionRequest, ResolveResult, SelectionState, UnsatCore,
    E_LOADER_INDEX_INVALID, E_LOADER_UNSUPPORTED_SCHEMA_VERSION, E_RESOLVE_SOLVER_MODEL_UNAVAILABLE,
    E_SELECTION_ENGINE_DIVERGENCE, E_SELECTION_SOLVER_MODEL_UNAVAILABLE, E_SELECTION_STATE_INVALID,
    E_SELECTION_UNSATISFIABLE,
};
use compiler::product_api::{
    Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
};
use solver::{
    CoreConstraintKind, CuddBackend, LabeledAtom, LabeledConstraint, LabeledCore,
    RejectionExplanation, Session,
};
use std::collections::BTreeMap;
use std::path::Path;

/// A single diagnostic in the given selection-family code, wrapped in the
/// one-error `DiagnosticsReport` the failed-envelope constructors carry.
fn fault_report(code: &'static str, message: String, entity_path: &str) -> DiagnosticsReport {
    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![Diagnostic {
            code: code.to_string(),
            severity: DiagnosticSeverity::Error,
            message,
            source_id: None,
            entity_path: Some(entity_path.to_string()),
            hint: Some(
                "Recompile the model so a usable .ccm is emitted, or re-run against \
                 a model whose .ccm is reachable"
                    .to_string(),
            ),
        }],
        error_count: 1,
        warning_count: 0,
    }
}

/// The outcome of resolving a `.ccm` reference into a query-ready session.
/// ADR-0030 makes a usable model a hard precondition: there is no longer a
/// "fall back to legacy" outcome here — every non-`Usable` case fails closed.
enum SolverModel {
    /// A usable solver model (loadable `.ccm`, populated symbol table) with the
    /// committed `choices` replayed onto it.
    Usable(Session<CuddBackend>),
    /// No usable model is reachable: empty reference, unloadable artifact, or a
    /// symbol-less stub CCM. ADR-0030 D1 → `*_SOLVER_MODEL_UNAVAILABLE`.
    Unavailable,
}

/// Build a fresh `Session<CuddBackend>` from a `.ccm` reference and replay the
/// committed `choices` onto it (ADR-0017 §3).
///
/// ADR-0030 D1: a usable `.ccm` is a HARD PRECONDITION. An empty reference, an
/// unloadable artifact, or a symbol-less stub CCM yields `Unavailable` and the
/// caller fails closed with `*_SOLVER_MODEL_UNAVAILABLE` — there is no silent
/// legacy fallback. Replayed `choices` the solver rejects, or that the BDD does
/// not model, are skipped: re-deriving a previously-accepted state must never
/// spuriously fail; the query being served is the authority on the *current*
/// decision.
fn session_from_handle(ccm_ref: &str, choices: &BTreeMap<String, String>) -> SolverModel {
    let ccm_ref = ccm_ref.trim();
    if ccm_ref.is_empty() {
        return SolverModel::Unavailable;
    }
    // An unloadable artifact is "no usable model" (ADR-0030 D1), not a query
    // fault — the artifact never became part of any decision's lineage.
    let Ok(ccm) = Session::<CuddBackend>::load_ccm(Path::new(ccm_ref)) else {
        return SolverModel::Unavailable;
    };
    let Ok(mut session) = Session::<CuddBackend>::new(ccm) else {
        return SolverModel::Unavailable;
    };
    // An empty/stub CCM (no symbol table) carries no constraints — it is a
    // symbol-less stub, which ADR-0030 D1 classifies as "no usable model".
    if session.ccm().symbols().is_none() {
        return SolverModel::Unavailable;
    }
    for (facet, option) in choices {
        let _ = session.apply(facet, option);
    }
    SolverModel::Usable(session)
}

/// Whether the loaded CCM's symbol table contains the facet `facet`
/// (some `{facet}.{value}` symbol is present). Mirrors the prefix
/// convention `Session::valid_options` uses to recognise a facet.
fn facet_present(session: &Session<CuddBackend>, facet: &str) -> bool {
    let Some(symbols) = session.ccm().symbols() else {
        return false;
    };
    let prefix = format!("{facet}.");
    symbols.variable_order().any(|sym| sym.starts_with(&prefix))
}

/// An empty, OK diagnostics report — byte-for-byte the container the legacy
/// `selection_options_ok` / `apply_selection_ok` constructors produce on the
/// success path (`schema_version`, no diagnostics, zero counts).
fn empty_diagnostics() -> DiagnosticsReport {
    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    }
}

/// Build the fail-closed `options` error envelope (ADR-0030 D1/D4), mirroring
/// the legacy `selection_options_failed` field layout so a client sees the same
/// shape it would on any other `options` error — only the code differs.
fn selection_options_unavailable(
    request: GetSelectionOptionsRequest,
    code: &'static str,
    message: &str,
) -> GetSelectionOptionsResult {
    let diagnostics = fault_report(code, message.to_string(), "model_handle.ccm_ref");
    GetSelectionOptionsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: request.model_handle.model_hash,
        scope: request.scope,
        facet: request.facet,
        valid_options: Vec::new(),
        default: None,
        declared_open: None,
        pruned_options: None,
        selection_state_hash: request.selection_state.selection_state_hash,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// `options` command (ADR-0017 §4): list still-valid options for a facet,
/// with the validity decision sourced from `solver::Session::valid_options`.
///
/// ADR-0030: a usable `.ccm` is a hard precondition. When no usable solver
/// model is reachable the command FAILS CLOSED with
/// `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` (D1); a solver fault on the modeled
/// facet FAILS CLOSED with `E_SELECTION_ENGINE_DIVERGENCE` (D4). The compiler
/// still owns, by permanent division of labor (D5): non-constraint validation
/// (schema mismatch, empty facet) and unconstrained facets (absent from the
/// symbol table — the `E_SELECTION_UNKNOWN_FACET` diagnostic). For the
/// solver-served path the result envelope is built with the same fields the
/// legacy `selection_options_ok` emits, so the only solver-derived field is
/// `valid_options`.
pub fn options(request: GetSelectionOptionsRequest) -> GetSelectionOptionsResult {
    // Non-constraint validation (schema version, empty facet) is permanently
    // compiler-owned (ADR-0030 D5), delegated so the diagnostics are
    // byte-identical; the solver only decides the satisfiability of a modeled
    // facet query.
    if request.schema_version != PRODUCT_SCHEMA_VERSION || request.facet.trim().is_empty() {
        return get_selection_options(request);
    }

    let session = match session_from_handle(
        &request.model_handle.ccm_ref,
        &request.selection_state.choices,
    ) {
        SolverModel::Usable(session) => session,
        // ADR-0030 D1: no usable model → fail closed, do NOT fall back.
        SolverModel::Unavailable => {
            return selection_options_unavailable(
                request,
                E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
                "no usable solver model (.ccm) is reachable for the requested model",
            );
        }
    };

    if !facet_present(&session, &request.facet) {
        // The facet is not in the solver's symbol table — an unconstrained
        // facet the compiler permanently owns (ADR-0030 D5). Defer to the
        // legacy path, which owns the `E_SELECTION_UNKNOWN_FACET` diagnostic.
        return get_selection_options(request);
    }

    let valid_options = match session.valid_options(&request.facet) {
        Ok(opts) => opts.options,
        // ADR-0030 D4: a fault on a solver-owned query fails closed — it is not
        // converted into a silent legacy fallback.
        Err(_) => {
            return selection_options_unavailable(
                request,
                E_SELECTION_ENGINE_DIVERGENCE,
                "the solver faulted while enumerating valid options for a modeled facet",
            );
        }
    };

    // Pruned reasons are a legacy-only convenience; when requested, take the
    // legacy envelope and override only the valid_options decision with the
    // solver's (byte-identical by construction — belt and braces).
    if request.include_pruned_reasons {
        let legacy = get_selection_options(request);
        return GetSelectionOptionsResult {
            valid_options,
            ..legacy
        };
    }

    // Take the legacy envelope and override ONLY the valid_options decision with
    // the solver's (byte-identical for every existing field by construction —
    // the comment above documents why). Merging the legacy result — rather than
    // re-building the struct field-by-field — also carries the ADR-0047 §6
    // `default` annotation (a pure function of the facet declaration, which only
    // the loader reads), so `cfx options` sees a declared facet's default arm on
    // the solver-served path too. `default` is skip-if-none, so an undeclared
    // facet's envelope stays byte-identical to the pre-ADR-0047 bytes.
    let legacy = get_selection_options(request);
    GetSelectionOptionsResult {
        valid_options,
        ..legacy
    }
}

/// `select` command (ADR-0017 §4/§5): apply a `(facet, option)` selection,
/// with the accept/reject decision and rejection code sourced from
/// `solver::Session::apply` mapped through the §5 table.
///
/// ADR-0030: a usable `.ccm` is a hard precondition. When no usable solver
/// model is reachable the command FAILS CLOSED with
/// `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` (D1). The permanent compiler-owned
/// delegations (D5) remain: schema mismatch, empty facet/option, an
/// already-applied or context-pinned facet (the legacy path renders these
/// without the BDD), and a facet absent from the symbol table (an
/// unconstrained facet). On a solver REJECT the wrapper still asks legacy to
/// render the canonical rejection bytes — but if legacy *accepts* what the
/// solver rejected, that engine disagreement is surfaced as
/// `E_SELECTION_ENGINE_DIVERGENCE` (D3), never silently reconciled. A solver
/// fault on the modeled apply FAILS CLOSED with the same internal-fault code
/// (D4). On solver acceptance the next `SelectionState` is rebuilt with the
/// same `canonical_selection_state` recipe the legacy path uses.
pub fn apply(request: ApplySelectionRequest) -> ApplySelectionResult {
    let facet = request.selection_delta.facet.clone();
    let option = request.selection_delta.option.clone();

    // Delegate the request shapes the legacy path adjudicates without the BDD
    // (schema, empty facet/option, idempotent re-apply, context-tag and
    // already-chosen conflicts) — permanently compiler-owned (ADR-0030 D5) so
    // their diagnostics stay byte-identical.
    if request.schema_version != PRODUCT_SCHEMA_VERSION
        || facet.trim().is_empty()
        || option.trim().is_empty()
        || request.selection_state.context_tags.contains_key(&facet)
        || request.selection_state.choices.contains_key(&facet)
    {
        return apply_selection(request);
    }

    let mut session = match session_from_handle(
        &request.model_handle.ccm_ref,
        &request.selection_state.choices,
    ) {
        SolverModel::Usable(session) => session,
        // ADR-0030 D1: no usable model → fail closed, do NOT fall back.
        SolverModel::Unavailable => {
            return apply_unavailable(
                request,
                E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
                "no usable solver model (.ccm) is reachable for the requested model",
            );
        }
    };

    if !facet_present(&session, &facet) {
        // Unconstrained facet — permanently compiler-owned (ADR-0030 D5).
        return apply_selection(request);
    }

    // The solver adjudicates the apply. On ACCEPT the wrapper builds the OK
    // envelope itself (its next `SelectionState` is rebuilt via the same
    // `canonical_selection_state` recipe legacy uses, so the bytes are
    // identical). On any REJECT — `E_SELECTION_*` (§5) or an internal fault —
    // the wrapper defers to the legacy `apply_selection`, which owns the
    // canonical rejection diagnostic (message/hint/blocking_choices). This
    // keeps the rejection JSON byte-identical to legacy while the *decision*
    // (accept vs reject) is the solver's.
    match session.apply(&facet, &option) {
        Ok(()) => {
            let mut next_choices = request.selection_state.choices.clone();
            next_choices.insert(facet.clone(), option.clone());
            match canonical_selection_state(
                request.selection_state.model_hash.clone(),
                request.selection_state.scope.clone(),
                request.selection_state.context_tags.clone(),
                next_choices,
            ) {
                Ok(next_state) => {
                    apply_ok(request.model_handle.model_hash, request.scope, next_state)
                }
                // A canonicalization failure is not a constraint decision —
                // delegate so legacy owns the (identical) error envelope. This
                // is byte-fidelity, not availability fallback (ADR-0030 D5).
                Err(_) => apply_selection(request),
            }
        }
        // Typed REJECT (ADR-0017 §5): the solver decided to reject. Legacy
        // renders the canonical rejection diagnostic bytes — BUT if legacy
        // would have *accepted* this selection, the engines disagree, which
        // ADR-0030 D3 surfaces as `E_SELECTION_ENGINE_DIVERGENCE` (fail closed)
        // rather than letting the legacy OK win.
        Err(solver::Error::Conflict { .. })
        | Err(solver::Error::UnknownOption { .. })
        | Err(solver::Error::UnknownFacet(_)) => {
            let legacy = apply_selection(request);
            match legacy.status {
                // Engines agree on REJECT → legacy's canonical bytes (identical
                // to pre-ADR-0030 behavior on every existing rejection).
                OperationStatus::Error => legacy,
                // Engines disagree: solver REJECT vs legacy OK → correctness
                // incident, fail closed (ADR-0030 D3).
                OperationStatus::Ok => apply_divergence(
                    legacy.model_hash,
                    legacy.scope,
                    E_SELECTION_ENGINE_DIVERGENCE,
                    "solver rejected a selection the legacy engine accepted \
                     (engine divergence)",
                ),
            }
        }
        // Internal solver fault (Backend/Invariant/Ccm) on the modeled apply.
        // ADR-0030 D4: fail closed — a fault that silently switches engines is
        // indistinguishable from rot. Do NOT delegate to legacy.
        Err(_) => apply_unavailable(
            request,
            E_SELECTION_ENGINE_DIVERGENCE,
            "the solver faulted while adjudicating a modeled selection",
        ),
    }
}

/// `resolve` command (ADR-0017 amendment): gate satisfiability with the
/// solver, then compose the entire rich `ResolveResult` from the compiler
/// resolver. Byte-identical to the legacy path **by construction** — the
/// composed output bytes ALWAYS come from
/// `compiler::loader_api::resolve_from_selection` (`resolve_scoped` +
/// `compute_resolve_hash`), exactly as the legacy path does.
///
/// The solver's role here is the **satisfiability gate** the amendment
/// specifies: before delegating, it confirms the current selection is
/// satisfiable over the `.ccm` boolean model.
///
/// ADR-0030 D1 makes the solver model part of every selection decision's
/// lineage: the gate is no longer advisory-on-absence. When no usable solver
/// model is reachable, or the gate faults (D4), `resolve` FAILS CLOSED with
/// `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE` instead of delegating to the compiler.
/// On a present model: the compiler resolver independently rejects an
/// unsatisfiable selection (its `E_RESOLVE_CONTEXT_UNSATISFIED` path), so on a
/// detected unsat the wrapper returns the **compiler's** canonical rejection
/// (bytes identical to legacy), and on sat it returns the compiler's composed
/// payload. The solver's own flat `resolved_output`/`resolve_hash`
/// (configflux-i0ne) is never consulted (amendment "i0ne demotion").
pub fn resolve(request: ResolveFromSelectionRequest) -> ResolveResult {
    match solver_sat_gate(&request.model_handle.ccm_ref, &request.selection_state) {
        // Solver model present, satisfiable or unsatisfiable: the compiler is
        // the source of the response bytes (amendment "compiler composes the
        // entire rich ResolveResult"). On unsat the compiler resolver renders
        // the same canonical rejection the legacy path would.
        SatVerdict::Satisfiable | SatVerdict::Unsatisfiable => resolve_from_selection(request),
        // ADR-0030 D1/D4: no usable model, or the gate faulted → fail closed.
        // The solver model is part of every resolve decision's lineage; a
        // missing or faulting gate is no longer silently tolerated.
        SatVerdict::Unavailable | SatVerdict::Fault => {
            resolve_unavailable(request, E_RESOLVE_SOLVER_MODEL_UNAVAILABLE)
        }
    }
}

/// Build the fail-closed `resolve` error envelope (ADR-0030 D1/D4), mirroring
/// the legacy `resolve_failed` field layout (carries the request's context_tags
/// and choices so a client can attribute the failure to its selection).
fn resolve_unavailable(
    request: ResolveFromSelectionRequest,
    code: &'static str,
) -> ResolveResult {
    let diagnostics = fault_report(
        code,
        "no usable solver model (.ccm) is reachable to gate resolve satisfiability".to_string(),
        "model_handle.ccm_ref",
    );
    ResolveResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: request.model_handle.model_hash,
        scope: request.scope,
        selection_state_hash: request.selection_state.selection_state_hash,
        resolve_hash: None,
        resolved_output: None,
        context_tags: request.selection_state.context_tags,
        choices: request.selection_state.choices,
        defaulted_choices: BTreeMap::new(),
        resolved_component_dependencies: BTreeMap::new(),
        resolved_artifacts: BTreeMap::new(),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// The solver's verdict on whether the current selection is satisfiable over
/// the `.ccm` boolean model.
enum SatVerdict {
    /// A solver model was present and the committed selection is satisfiable.
    Satisfiable,
    /// A solver model was present and the committed selection is
    /// unsatisfiable (the compiler resolver will reject it too).
    Unsatisfiable,
    /// No usable solver model was reachable (empty ref, unloadable artifact,
    /// or symbol-less stub). ADR-0030 D1 → fail closed.
    Unavailable,
    /// The gate faulted on a solver-owned query (Backend/Invariant/Ccm error
    /// while replaying a modeled choice). ADR-0030 D4 → fail closed.
    Fault,
}

/// Gate the current selection's satisfiability with the solver by re-deriving
/// the committed `choices` onto a fresh ephemeral session (ADR-0017 §3).
/// Expresses the amendment's "solver gates satisfiability" step explicitly.
/// ADR-0030 D1/D4: this gate is load-bearing — its verdict can fail the
/// command. `Unavailable` (no usable model) and `Fault` (a query fault while
/// replaying a modeled choice) both make `resolve` fail closed.
fn solver_sat_gate(ccm_ref: &str, selection_state: &SelectionState) -> SatVerdict {
    let ccm_ref = ccm_ref.trim();
    if ccm_ref.is_empty() {
        return SatVerdict::Unavailable;
    }
    // Load/construct failures are "no usable model" (ADR-0030 D1), distinct
    // from a query fault on an otherwise-usable model (D4) below.
    let Ok(ccm) = Session::<CuddBackend>::load_ccm(Path::new(ccm_ref)) else {
        return SatVerdict::Unavailable;
    };
    let Ok(mut session) = Session::<CuddBackend>::new(ccm) else {
        return SatVerdict::Unavailable;
    };
    if session.ccm().symbols().is_none() {
        return SatVerdict::Unavailable;
    }

    // Replay each committed choice the BDD models. A typed rejection means the
    // selection became unsatisfiable (the compiler resolver will reject it
    // too). A Backend/Invariant/Ccm fault on this solver-owned query fails
    // closed (ADR-0030 D4) — it is no longer masked as model absence.
    for (facet, option) in &selection_state.choices {
        if !facet_present(&session, facet) {
            continue;
        }
        match session.apply(facet, option) {
            Ok(()) => {}
            Err(solver::Error::Conflict { .. })
            | Err(solver::Error::UnknownOption { .. })
            | Err(solver::Error::UnknownFacet(_)) => return SatVerdict::Unsatisfiable,
            Err(_) => return SatVerdict::Fault,
        }
    }
    SatVerdict::Satisfiable
}

/// Build the `apply_selection` OK result envelope (mirrors the legacy
/// `apply_selection_ok` constructor field-for-field).
fn apply_ok(model_hash: String, scope: String, next_state: SelectionState) -> ApplySelectionResult {
    ApplySelectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        selection_state: Some(next_state),
        error_count: 0,
        warning_count: 0,
        diagnostics_ref: None,
        diagnostics: empty_diagnostics(),
    }
}

/// Build the fail-closed `select` error envelope from the request (ADR-0030
/// D1/D4), mirroring the legacy `apply_selection_failed` field layout.
fn apply_unavailable(
    request: ApplySelectionRequest,
    code: &'static str,
    message: &str,
) -> ApplySelectionResult {
    apply_divergence(
        request.model_handle.model_hash,
        request.scope,
        code,
        message,
    )
}

/// Build the fail-closed `select` error envelope from an already-extracted
/// `model_hash`/`scope` (ADR-0030 D3/D4), used on the divergence and fault
/// paths where the request has already been consumed.
fn apply_divergence(
    model_hash: String,
    scope: String,
    code: &'static str,
    message: &str,
) -> ApplySelectionResult {
    let diagnostics = fault_report(code, message.to_string(), "model_handle.ccm_ref");
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

/// `explain-rejection` command (ADR-0017 §4, ADR-0031): explain why a
/// `(facet, option)` pair would be rejected, sourcing the labeled minimal
/// unsatisfiable subset (MUS) from `solver::Session::explain_rejection` and
/// mapping it into the compiler-side `UnsatCore` envelope (ADR-0031 D3). A
/// read-side, solver-decided query (D1).
///
/// Transport rule (ADR-0031 D2): **a rejection is not an error.** "Why is X
/// rejected? — because of A and B" is the *success* path (`status: ok`, exit
/// `0`); exit `2` is reserved for "the explain operation could not run at all"
/// (no usable `.ccm`, solver fault). This differs from the legacy compiler
/// `explain_rejection`, which reported every rejection as `status: error`.
///
/// Outcome map:
///   * **Genuine conflict** (`would_reject: true`, core) → `ok`, exit `0`,
///     `E_SELECTION_UNSATISFIABLE`, `unsat_core` populated from the solver MUS.
///     The decision content is the solver's; this wrapper only maps it onto the
///     compiler envelope (ADR-0003 §2 — the compiler never imports the solver).
///   * **Valid option** (`would_reject: false`) → not a rejection; `ok`, exit
///     `0`, no core (the compiler's canonical "currently valid" diagnostic).
///   * **Division-of-labor** unknown facet / invalid option (ADR-0030 D5) →
///     `ok`, exit `0`, no core (the compiler's canonical `E_SELECTION_*` bytes).
///   * **No usable `.ccm`** (ADR-0030 D1) → fail closed: `error`, exit `2`,
///     `E_SELECTION_SOLVER_MODEL_UNAVAILABLE`.
///   * **Solver / MUS fault or unmappable variable** (ADR-0030/0031 D4) → fail
///     closed: `error`, exit `2`, `E_SELECTION_ENGINE_DIVERGENCE`. Never a
///     partial core, never a raw index.
///   * **Request-invalid** (bad `schema_version`, tampered `selection_state`,
///     unloadable model) → delegated to the compiler, which renders the
///     canonical `E_LOADER_*` / `E_SELECTION_STATE_INVALID` command error
///     (`error`, exit `2`). The operation could not run; this is not a
///     rejection (ADR-0031 D2).
pub fn explain(request: ExplainRejectionRequest) -> ExplainRejectionResult {
    let facet = request.rejected_option.facet.clone();
    let option = request.rejected_option.option.clone();

    // Compute the compiler's verdict once. It owns the canonical request
    // validation (schema_version, `selection_state` integrity, model load) and
    // the canonical rendering of every division-of-labor rejection (ADR-0030
    // D5); the solver owns only the *modeled* decision and labeled MUS (ADR-0031
    // D1/D3). The composition crate is the one place that may consult both
    // (ADR-0003 §2 amendment). The `clone` keeps the request for the solver path.
    let legacy = explain_rejection(request.clone());

    // Request-invalid shapes (bad `schema_version`, tampered `selection_state`,
    // unloadable model) are "the operation could not run" — not a rejection
    // (ADR-0031 D2). The compiler already rendered the canonical command error
    // (`status: error`, exit 2); return it verbatim. Delegating the verdict
    // (vs. re-deriving the hash here) keeps these bytes byte-identical to legacy.
    if matches!(
        legacy.rejection.code.as_str(),
        E_LOADER_UNSUPPORTED_SCHEMA_VERSION | E_SELECTION_STATE_INVALID | E_LOADER_INDEX_INVALID
    ) {
        return legacy;
    }

    let session = match session_from_handle(
        &request.model_handle.ccm_ref,
        &request.selection_state.choices,
    ) {
        SolverModel::Usable(session) => session,
        // ADR-0030 D1 / ADR-0031 D4: no usable model → fail closed, exit 2.
        SolverModel::Unavailable => {
            return explain_command_error(
                request,
                E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
                "no usable solver model (.ccm) is reachable to explain the rejection",
            );
        }
    };

    match session.explain_rejection(&facet, &option) {
        // Genuine conflict: the solver produced a labeled MUS. Map it onto the
        // compiler `UnsatCore` and report the successful query (ADR-0031 D2/D3).
        Ok(RejectionExplanation {
            would_reject: true,
            core: Some(core),
        }) => explain_rejected_ok(
            request.model_handle.model_hash,
            request.scope,
            facet,
            option,
            labeled_core_to_unsat_core(core),
        ),
        // Defensive: a conflict with no core is an internal inconsistency (the
        // core is present iff the code is a genuine conflict, ADR-0031 D3). Fail
        // closed (D4) rather than emit a coreless conflict.
        Ok(RejectionExplanation {
            would_reject: true,
            core: None,
        }) => explain_command_error(
            request,
            E_SELECTION_ENGINE_DIVERGENCE,
            "the solver reported a conflict rejection without a labeled core",
        ),
        // Valid option: not a rejection. Keep the compiler's canonical
        // "currently valid" diagnostic but report the successful-query status.
        Ok(RejectionExplanation {
            would_reject: false,
            ..
        }) => explain_as_successful_query(legacy),
        // Division-of-labor (ADR-0030 D5): unknown facet / invalid option are
        // the typed errors `apply` surfaces, with no conflict to minimize. Keep
        // the compiler's canonical `E_SELECTION_*` bytes but report the
        // successful-query status (a rejection is not an error, ADR-0031 D2).
        Err(solver::Error::UnknownOption { .. }) | Err(solver::Error::UnknownFacet(_)) => {
            explain_as_successful_query(legacy)
        }
        // Internal solver / MUS fault or unmappable variable (ADR-0030/0031 D4).
        // A silently empty or guessed core is indistinguishable from rot; fail
        // closed. Never a partial core, never a raw index.
        Err(_) => explain_command_error(
            request,
            E_SELECTION_ENGINE_DIVERGENCE,
            "the solver faulted while extracting the rejection's minimal core",
        ),
    }
}

/// Map the solver-owned labeled core (`LabeledCore`, labeled `{facet}.{value}`
/// strings only) onto the compiler-side `UnsatCore` envelope (ADR-0031 D3). The
/// compiler never imports the solver (ADR-0003 §2), so the conversion lives
/// here. Every atom is already a symbol-table label (an unmappable variable is a
/// solver-side fail-closed `Err`, never reaching here), so no raw BDD/batsat
/// index can appear in the output — the D3 schema invariant.
fn labeled_core_to_unsat_core(core: LabeledCore) -> UnsatCore {
    UnsatCore {
        rejected: atom_to_facet(&core.rejected),
        conflicting_constraints: core
            .conflicting_constraints
            .iter()
            .map(constraint_to_conflicting)
            .collect(),
        minimal: core.minimal,
        // Fixed advisory string acknowledging non-uniqueness (ADR-0031 D3): MUS
        // extraction returns *a* minimal explanation, not *the* canonical one.
        note: "one minimal explanation; other minimal cores may exist".to_string(),
    }
}

/// Convert one solver `LabeledConstraint` into the compiler-side
/// `ConflictingConstraint`, synthesizing the advisory `summary` gloss (ADR-0031
/// D3: advisory text, not a parsed field). The JSON→text presentation renderer
/// is a separate, layered concern (configflux-9d28 / ADR-0031 D5).
fn constraint_to_conflicting(constraint: &LabeledConstraint) -> ConflictingConstraint {
    ConflictingConstraint {
        kind: kind_to_constraint_kind(constraint.kind),
        facets: constraint.atoms.iter().map(atom_to_facet).collect(),
        summary: summarize_constraint(constraint),
    }
}

/// A deterministic one-line advisory gloss for a conflicting constraint, built
/// from its labeled atoms (stable for a given shape so the envelope is
/// reproducible). Style follows ADR-0031 D3/D5; the authoritative presentation
/// layer is configflux-9d28.
fn summarize_constraint(constraint: &LabeledConstraint) -> String {
    let atoms: Vec<String> = constraint
        .atoms
        .iter()
        .map(|atom| format!("{}.{}", atom.facet, atom.value))
        .collect();
    match constraint.kind {
        CoreConstraintKind::Selection => {
            format!("blocked by your earlier choice: {}", atoms.join(", "))
        }
        CoreConstraintKind::ModelRule => {
            format!("blocked by model rule relating: {}", atoms.join(", "))
        }
    }
}

/// Map the solver's `CoreConstraintKind` onto the compiler-side
/// `ConstraintKind` (ADR-0031 D3 `kind` field).
fn kind_to_constraint_kind(kind: CoreConstraintKind) -> ConstraintKind {
    match kind {
        CoreConstraintKind::Selection => ConstraintKind::Selection,
        CoreConstraintKind::ModelRule => ConstraintKind::ModelRule,
    }
}

/// Convert a solver `LabeledAtom` into the compiler-side `ConstraintFacet`
/// (the labeled `{facet, option}` pair, ADR-0031 D3). Labeled names only.
fn atom_to_facet(atom: &LabeledAtom) -> ConstraintFacet {
    ConstraintFacet {
        facet: atom.facet.clone(),
        option: atom.value.clone(),
    }
}

/// Re-stamp a compiler-rendered explain result as the successful query it is
/// (ADR-0031 D2): a rejection is the success path (`status: ok`, exit 0), not a
/// command error. Keeps the compiler's canonical `rejection`
/// (code/message/hint/no core) but reports the successful-query status, zeroed
/// counts, and an OK-empty `diagnostics` (the rejection lives in `rejection`; a
/// successful query carries no error diagnostics).
fn explain_as_successful_query(result: ExplainRejectionResult) -> ExplainRejectionResult {
    ExplainRejectionResult {
        status: OperationStatus::Ok,
        error_count: 0,
        warning_count: 0,
        diagnostics_ref: None,
        diagnostics: empty_diagnostics(),
        ..result
    }
}

/// Build the successful `explain-rejection` envelope for a genuine conflict
/// carrying a labeled unsat core (ADR-0031 D2/D3): `status: ok`, exit `0`,
/// `E_SELECTION_UNSATISFIABLE`, `unsat_core` present. `blocking_choices` mirrors
/// the prior `selection` atoms named by the core so clients reading that field
/// still see the conflicting choices.
fn explain_rejected_ok(
    model_hash: String,
    scope: String,
    facet: String,
    option: String,
    unsat_core: UnsatCore,
) -> ExplainRejectionResult {
    let blocking_choices = unsat_core
        .conflicting_constraints
        .iter()
        .filter(|c| c.kind == ConstraintKind::Selection)
        .flat_map(|c| c.facets.iter())
        .map(|f| (f.facet.clone(), f.option.clone()))
        .collect();
    let rejection = RejectionReason {
        code: E_SELECTION_UNSATISFIABLE.to_string(),
        message: format!("Selection '{facet}'='{option}' is unsatisfiable under current constraints"),
        blocking_choices,
        hint: Some(
            "Use get_selection_options to choose a compatible option first".to_string(),
        ),
        unsat_core: Some(unsat_core),
    };
    ExplainRejectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        scope,
        facet,
        option,
        rejection,
        error_count: 0,
        warning_count: 0,
        diagnostics_ref: None,
        diagnostics: empty_diagnostics(),
    }
}

/// Build the fail-closed `explain-rejection` command-error envelope (ADR-0030
/// D1/D4, ADR-0031 D4): `status: error`, exit `2`, the given code, no core. Used
/// when the explain operation could not run — no usable `.ccm`, or a solver/MUS
/// fault. Mirrors the legacy `explain_rejection_failed` field layout.
fn explain_command_error(
    request: ExplainRejectionRequest,
    code: &'static str,
    message: &str,
) -> ExplainRejectionResult {
    let facet = request.rejected_option.facet;
    let option = request.rejected_option.option;
    let diagnostics = fault_report(code, message.to_string(), "model_handle.ccm_ref");
    ExplainRejectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: request.model_handle.model_hash,
        scope: request.scope,
        facet,
        option,
        rejection: RejectionReason {
            code: code.to_string(),
            message: message.to_string(),
            blocking_choices: BTreeMap::new(),
            hint: Some(
                "Recompile the model so a usable .ccm is emitted, or re-run against \
                 a model whose .ccm is reachable"
                    .to_string(),
            ),
            unsat_core: None,
        },
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}
