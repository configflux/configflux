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
// via `load_ccm(ccm_ref)` → `new` → `apply_environment` (context tags, then
// choices — ADR-0057 §D6); `CuddBackend` is the wired backend (§1). Every
// session this crate builds goes through that ONE replay (configflux-bmjt), so
// no two surfaces can reason over different deployments.
//
// CCM availability is a HARD PRECONDITION (ADR-0030, amends ADR-0017): a usable
// `.ccm` (non-empty ref, loadable artifact, populated symbol table) is required
// for every solver-owned selection decision. When no usable model is reachable,
// the wrapper FAILS CLOSED with a stable diagnostic instead of degrading to the
// legacy compiler path:
//   * `options`/`select` → `E_SELECTION_SOLVER_MODEL_UNAVAILABLE`
//   * `resolve`          → `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE`
// A solver fault on a solver-owned query likewise fails closed (ADR-0030 D4).
// The legitimate compiler delegations that remain — schema/empty validation,
// unconstrained (unknown) facets, idempotent/context-pinned applies, the
// integrity AND the model-admissibility of the caller-supplied `SelectionState`,
// and the entire `resolve` envelope composition — are the permanent
// division-of-labor of ADR-0030 D5, not availability fallback. The only path
// that still routes a *modeled* decision to the compiler is the typed-rejection
// render delegation on `select` (the solver decided REJECT; the compiler renders
// the canonical diagnostic bytes), which is byte-fidelity, not fallback.
//
// ADR-0030 Amendment 2 (configflux-eclx) settles who refuses a caller-supplied
// STATE, in three rules this crate implements two of:
//   * The COMPILER owns admissibility — a facet the model has, a value in its
//     domain, a closed facet's tags within its declared values. Every surface
//     here asks `selection_state_is_admissible` BEFORE it builds a session,
//     because a session silently skips what it cannot apply and the
//     inadmissible entry would be gone by the time the solver answered.
//   * The SOLVER owns satisfiability, and a `Rejected` replay is a rejection of
//     the STATE. No surface answers over the weakened session: each delegates,
//     keeps the compiler's bytes where the compiler saw the contradiction too,
//     and renders `E_SELECTION_CONFLICT` itself where it did not.
// `E_SELECTION_ENGINE_DIVERGENCE` is left with exactly three meanings: a solver
// fault on a solver-owned query (D4), a conflict reported without a core
// (ADR-0031 D3), and a `.ccm` that does not model a value the sources declare
// (Rule 2's skew). It is no longer what `select` says when the two engines
// merely disagree — after ADR-0057 the compiler's check is three-valued and
// `Unknown` is never a violation (ADR-0054 §2), so over an unbound derive-only
// binding it legitimately accepts what the total BDD refuses.

use compiler::loader_api::{
    apply_selection, attribute_core_clauses, canonical_selection_state, closed_facet_domains,
    explain_rejection, get_selection_options, resolve_from_selection,
    selection_state_is_admissible, selection_state_is_valid, ApplySelectionRequest,
    ApplySelectionResult, ClosedFacetDomains, ConstraintFacet, ConstraintKind, CoreClause,
    DeclaredConstraint, ExplainRejectionRequest, ExplainRejectionResult,
    GetSelectionOptionsRequest, GetSelectionOptionsResult, RejectionReason,
    ResolveFromSelectionRequest, ResolveResult, SelectionState, UnsatCore, E_LOADER_INDEX_INVALID,
    E_LOADER_UNSUPPORTED_SCHEMA_VERSION, E_RESOLVE_FACET_UNBOUND,
    E_RESOLVE_SOLVER_MODEL_UNAVAILABLE, E_SELECTION_CONFLICT, E_SELECTION_ENGINE_DIVERGENCE,
    E_SELECTION_SOLVER_MODEL_UNAVAILABLE, E_SELECTION_STATE_INVALID, E_SELECTION_UNSATISFIABLE,
};
use compiler::product_api::{
    Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
};
use solver::{
    Ccm, ConstraintRef, CoreConstraintKind, CuddBackend, LabeledAtom, LabeledConstraint,
    LabeledCore, RejectionExplanation, Session,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One diagnostic wrapped in the one-error `DiagnosticsReport` every
/// failed-envelope constructor in this crate carries.
fn one_error_report(diagnostic: Diagnostic) -> DiagnosticsReport {
    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![diagnostic],
        error_count: 1,
        warning_count: 0,
    }
}

/// The ARTIFACT-fault diagnostic: something about the model package is wrong
/// (missing, unloadable, or out of step with its sources), so the hint sends the
/// reader to the package rather than to their selection.
fn fault_diagnostic(code: &'static str, message: String, entity_path: &str) -> Diagnostic {
    Diagnostic {
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
    }
}

/// A single diagnostic in the given selection-family code, wrapped in the
/// one-error `DiagnosticsReport` the failed-envelope constructors carry.
fn fault_report(code: &'static str, message: String, entity_path: &str) -> DiagnosticsReport {
    one_error_report(fault_diagnostic(code, message, entity_path))
}

/// The diagnostic for a selection the SOLVER has proved unsatisfiable, in the
/// one place all four surfaces read it from (ADR-0030 Amendment 2 Rule 3,
/// configflux-eclx).
///
/// This is the wording `assert_unsatisfiable` introduced for `resolve`
/// (configflux-rzyd) and the reason it is shared: the same contradiction now
/// reaches `select`, `options` and `explain`, and a per-surface rewording would
/// be four descriptions of one fact — precisely the surface disagreement
/// ADR-0054 exists to remove.
///
/// It names the assignment the SOLVER refused rather than whatever downstream
/// symptom a surface happened to notice. Naming the symptom is how the old
/// `resolve` message came to assert a false cause, telling the reader to give a
/// facet a default that could not have helped.
fn selection_conflict_diagnostic(facet: &str, option: &str) -> Diagnostic {
    Diagnostic {
        code: E_SELECTION_CONFLICT.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!(
            "Selection is unsatisfiable: no assignment of the remaining facets satisfies \
             the model once '{facet}' = '{option}' is applied"
        ),
        source_id: None,
        entity_path: None,
        hint: Some(
            "Run explain with the same selection to see the minimal set of choices that conflict"
                .to_string(),
        ),
    }
}

/// The message for a `.ccm` that does not model a value the model sources
/// DECLARE (ADR-0030 Amendment 2 Rule 2's skew). Names the missing value: the
/// reader needs to know WHICH declaration the artifact is behind, not merely
/// that something is.
fn skew_message(facet: &str, option: &str) -> String {
    format!(
        "the solver model does not model declared value '{option}' of closed facet \
         '{facet}'; recompile the model"
    )
}

/// The outcome of resolving a `.ccm` reference into a query-ready session.
/// ADR-0030 makes a usable model a hard precondition: there is no longer a
/// "fall back to legacy" outcome here — every non-`Usable` case fails closed.
enum SolverModel {
    /// A usable solver model (loadable `.ccm`, populated symbol table) with the
    /// deployment environment — context tags, then choices — replayed onto it,
    /// TOGETHER with the verdict that replay reached.
    ///
    /// The verdict travels with the session because the caller needs both: the
    /// session to ask its own question, and the verdict to know whether the
    /// environment it is asking about is one the model can hold at all. Before
    /// ADR-0030 Amendment 2 it was discarded here, so every surface answered
    /// over a session that had quietly dropped whatever the model refused.
    Usable {
        session: Session<CuddBackend>,
        replay: EnvReplay,
    },
    /// The `.ccm` does not model a value the model sources DECLARE — the
    /// artifact is out of step with them (ADR-0030 Amendment 2 Rule 2).
    /// Rendered as each surface's D4 code, naming the value that is missing.
    Skew { facet: String, option: String },
    /// No usable model is reachable: empty reference, unloadable artifact, or a
    /// symbol-less stub CCM. ADR-0030 D1 → `*_SOLVER_MODEL_UNAVAILABLE`.
    Unavailable,
}

/// The outcome of replaying a deployment environment onto a session.
enum EnvReplay {
    /// Every modeled assignment the model can hold was accepted. Assignments on
    /// facets the `.ccm` does not model, wholly or partly, were skipped.
    Accepted,
    /// The solver proved the environment UNSATISFIABLE: it holds a `Conflict`
    /// over values the model does model.
    ///
    /// This variant means exactly that one thing (ADR-0030 Amendment 2 Rule 2).
    /// It used to mean three, folding a `Conflict` together with the two
    /// "the model does not know this" errors, and each surface then resolved
    /// the ambiguity its own way — `select` and `options` answered over the
    /// weakened session, `resolve` deferred to the compiler, inference gave up
    /// on the whole model. Splitting the verdict is what lets all four agree.
    ///
    /// Carries the FIRST refused assignment (configflux-rzyd), which is the
    /// only name a surface has for the cause when it must render the rejection
    /// itself. First rather than last because a later refusal is usually a
    /// consequence of the earlier one, and it is the convention `cfx explain`
    /// already follows — the first rejected choice is the one it explains.
    Rejected { facet: String, option: String },
    /// The `.ccm` cannot apply a value the model sources DECLARE for a CLOSED
    /// facet (ADR-0030 Amendment 2 Rule 2). Rule 1 has already admitted the
    /// assignment, so the two facts together say the artifact is behind its
    /// sources rather than the request being wrong: artifact skew, ADR-0030 D4.
    Skew { facet: String, option: String },
    /// The solver faulted (`Backend`/`Invariant`/`Ccm`) on a modeled
    /// assignment. ADR-0030 D4: a caller that can fail closed, must.
    Fault,
}

/// Replay one deployment ENVIRONMENT onto `session`: **context tags first,
/// then explicit choices** (ADR-0057 §D6).
///
/// **This is the ONE definition of "apply the environment to a session", and
/// every session this crate builds comes through it (configflux-bmjt).** The
/// three builders each carried their own copy and only the inference fixpoint
/// applied `context_tags`, so the surfaces reasoned over different deployments:
/// `options` offered values a tag had already excluded, `explain` called such a
/// value currently valid, and `select` accepted a choice the compiler's own
/// `apply_selection` refuses by name. That is precisely the surface
/// disagreement ADR-0054 exists to eliminate. A context tag is part of the
/// deployment environment, so a session that omits it answers about a
/// deployment nobody asked for.
///
/// **Tags before choices, and never the reverse.** ADR-0057 §D6's ladder is
/// about which SOURCE supplies an unbound facet, and a solver session can only
/// express it by binding the lower-ranked source first: a later contradicting
/// assignment is then REFUSED rather than silently overlaid, which is what the
/// compiler already does for the same pair (`merge_assignments` /
/// `apply_selection`'s immutable-context-tag rejection, configflux-kbue). Both
/// maps are `BTreeMap`s, so the sequence of solver calls is byte-stable.
///
/// Facets the CCM does not model are SKIPPED rather than failed: an
/// unconstrained facet is permanently the compiler's to adjudicate (ADR-0030
/// D5), and the assignment roster comes from the user's state rather than from
/// the symbol table.
///
/// **A typed refusal is CLASSIFIED, not flattened** (ADR-0030 Amendment 2
/// Rule 2). The three typed errors mean three different things and the caller
/// needs to tell them apart:
///
///   * `Conflict` — the model DOES model these values and forbids them
///     together. The environment is unsatisfiable. Recorded as `Rejected` and
///     the replay continues, so a caller re-deriving an already-accepted state
///     still gets every assignment the model accepts while a caller that must
///     fail closed still sees the verdict.
///   * `UnknownOption` on a facet OUTSIDE the closed roster — the `.ccm` models
///     the facet only partially, which is ordinary: an undeclared facet's
///     symbols come from whatever its conditions happen to name. Skipped
///     exactly as a wholly absent facet is. S1's smoke resolves with a
///     `region=us` tag against a model whose only mention of `region` is the
///     condition `region == 'eu'`; treating that as a rejection is what used to
///     disable inference for the entire model (configflux-cy3k).
///   * `UnknownOption` on a CLOSED facet, or `UnknownFacet` at all — the value
///     passed Rule 1, so the model DECLARES it and the artifact does not carry
///     it. That is skew, and it stops the replay. (`UnknownFacet` is
///     unreachable past `facet_present`; reaching it means something is wrong,
///     which is the same conclusion.)
///
/// `Fault` stops the replay and DOMINATES a rejection seen earlier in the same
/// replay: the
/// session is then missing every assignment after the fault, so it is strictly
/// less constrained than the deployment asked about, and no caller may treat it
/// as an answer. Every caller must therefore fail closed on `Fault` —
/// `model_from_replay` is where the query surfaces do it.
///
/// That dominance is a deliberate, narrow behaviour change (configflux-bmjt).
/// The pre-change satisfiability gate returned `Unsatisfiable` at the FIRST
/// typed rejection and so could never observe a later fault; it now reports
/// `Fault`, and `resolve` renders `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE` instead
/// of letting the compiler render its rejection. Both are refusals and the
/// fail-closed one is correct, but the code differs, so it is recorded here
/// rather than left for a reader to discover. It requires a Backend/Invariant/
/// Ccm error, which no well-formed artifact produces.
fn apply_environment(
    session: &mut Session<CuddBackend>,
    state: &SelectionState,
    closed: &ClosedFacetDomains,
) -> EnvReplay {
    let mut outcome = EnvReplay::Accepted;
    for (facet, option) in state.context_tags.iter().chain(state.choices.iter()) {
        if !facet_present(session, facet) {
            continue;
        }
        match session.apply(facet, option) {
            Ok(()) => {}
            Err(solver::Error::Conflict { .. }) => {
                // Keep the FIRST refusal, so the recorded name is stable and is
                // the cause rather than one of its consequences. Both maps are
                // `BTreeMap`s, so "first" is a deterministic position.
                if matches!(outcome, EnvReplay::Accepted) {
                    outcome = EnvReplay::Rejected {
                        facet: facet.clone(),
                        option: option.clone(),
                    };
                }
            }
            Err(solver::Error::UnknownOption { .. }) => {
                if !is_closed_facet(closed, facet) {
                    // The model knows this facet only partially. Skip it, as an
                    // absent facet is skipped — an unconstrained assignment is
                    // permanently the compiler's to adjudicate (ADR-0030 D5).
                    continue;
                }
                return EnvReplay::Skew {
                    facet: facet.clone(),
                    option: option.clone(),
                };
            }
            Err(solver::Error::UnknownFacet(_)) => {
                return EnvReplay::Skew {
                    facet: facet.clone(),
                    option: option.clone(),
                }
            }
            Err(_) => return EnvReplay::Fault,
        }
    }
    outcome
}

/// Whether `facet` is a declared CLOSED facet of the model — the roster
/// `closed_facet_domains` builds, bindings included (ADR-0057 §D3).
///
/// A linear walk rather than a lookup: the roster holds one entry per declared
/// closed facet and the replay consults it only for an assignment the solver
/// has already refused, so the cost is invisible beside the BDD call that
/// produced the refusal, and it keeps the type's read-only surface unwidened.
fn is_closed_facet(closed: &ClosedFacetDomains, facet: &str) -> bool {
    closed.iter().any(|(declared, _)| declared == facet)
}

/// Build a fresh `Session<CuddBackend>` from a `.ccm` reference and replay the
/// deployment environment — context tags, then choices — onto it (ADR-0017 §3,
/// ADR-0057 §D6).
///
/// ADR-0030 D1: a usable `.ccm` is a HARD PRECONDITION. An empty reference, an
/// unloadable artifact, or a symbol-less stub CCM yields `Unavailable` and the
/// caller fails closed with `*_SOLVER_MODEL_UNAVAILABLE` — there is no silent
/// legacy fallback. A replay that FAULTS yields `Unavailable` for the same
/// reason (ADR-0030 D4); `model_from_replay` records why that arm exists and why
/// the other two do not.
fn session_from_handle(
    ccm_ref: &str,
    state: &SelectionState,
    closed: &ClosedFacetDomains,
) -> SolverModel {
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
    let replay = apply_environment(&mut session, state, closed);
    model_from_replay(session, replay)
}

/// Turn a finished environment replay into the model verdict the query surfaces
/// consume. Four arms, and the difference between them is the whole rule.
///
/// `Accepted` and `Rejected` both yield the session, and the verdict rides
/// along with it. The session is still needed — a `Rejected` environment has an
/// answer, it is just a refusal — and the caller cannot reconstruct the verdict
/// from the session, because the assignments the model refused simply are not
/// in it.
///
/// **The division of labor this arm carries** (ADR-0030 Amendment 2, correcting
/// what stood here before): the COMPILER owns admissibility — whether the
/// state's assignments are ones this model can hold at all — and every caller
/// screens for it before building a session, through
/// `selection_state_is_admissible`. The SOLVER owns satisfiability, and a
/// `Rejected` verdict is its refusal of the STATE. It is not something the
/// compiler can be relied on to re-derive: after ADR-0057 the compiler's check
/// is three-valued and `Unknown` is never a violation (ADR-0054 §2), so over an
/// unbound derive-only binding it legitimately accepts what the total BDD
/// refuses. Each caller therefore delegates, keeps the compiler's bytes where
/// the compiler saw the contradiction too, and renders `E_SELECTION_CONFLICT`
/// itself where it did not.
///
/// `Skew` withholds the session: the artifact cannot hold a value the model
/// sources declare, so it does not describe this model and nothing asked of it
/// would be an answer about the deployment in hand.
///
/// `Fault` withholds it, and must. `apply_environment` STOPS at a
/// Backend/Invariant/Ccm error, so the assignments after it were never applied
/// and the session is strictly LESS constrained than the deployment the caller
/// asked about. Answering `options`, `select` or `explain` over that session
/// would be failing OPEN — offering values the deployment excludes — which is
/// the one outcome ADR-0030 D4 forbids. All three callers already render
/// `*_SOLVER_MODEL_UNAVAILABLE` for this verdict, so failing closed costs
/// nothing but a diagnostic. Do not "simplify" this back to a discarded
/// verdict: the pre-configflux-bmjt loop swallowed faults but kept replaying,
/// so discarding the verdict now would be strictly worse than what it replaced.
fn model_from_replay(session: Session<CuddBackend>, replay: EnvReplay) -> SolverModel {
    match replay {
        EnvReplay::Fault => SolverModel::Unavailable,
        EnvReplay::Skew { facet, option } => SolverModel::Skew { facet, option },
        replay @ (EnvReplay::Accepted | EnvReplay::Rejected { .. }) => {
            SolverModel::Usable { session, replay }
        }
    }
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
    selection_options_error(
        request,
        fault_diagnostic(code, message.to_string(), "model_handle.ccm_ref"),
    )
}

/// The same `options` error envelope around an arbitrary diagnostic, for the
/// refusals that are NOT artifact faults — a state the solver proves
/// unsatisfiable carries the conflict diagnostic and its hint, not "recompile
/// the model" (ADR-0030 Amendment 2 Rule 3).
fn selection_options_error(
    request: GetSelectionOptionsRequest,
    diagnostic: Diagnostic,
) -> GetSelectionOptionsResult {
    let diagnostics = one_error_report(diagnostic);
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

/// Override the compiler's `valid_options` with the solver's decision — unless
/// the compiler REFUSED the request, in which case its envelope comes back
/// VERBATIM.
///
/// A refusal carries an empty option list deliberately: the compiler owns those
/// bytes (ADR-0030 D5) and an error envelope must carry no payload (Amendment 2
/// Rule 1). Overriding that list published one result saying both "this request
/// failed" and "here are the valid options", so a caller reading the list
/// without the status acted on options for a question the product had just
/// declined to answer (configflux-tsuf). A request nobody answered has no valid
/// set to report.
///
/// On the success path the override is unchanged, and it is a merge rather than
/// a field-by-field rebuild because that is what carries the ADR-0047 §6
/// `default` / `declared_open` annotations — pure functions of the facet
/// declaration, which only the loader reads — so `cfx options` sees a declared
/// facet's default arm and schema kind on the solver-served path too. Both are
/// skip-if-none, so an undeclared facet's envelope stays byte-identical to the
/// pre-ADR-0047 bytes.
fn merge_valid_options(
    legacy: GetSelectionOptionsResult,
    valid_options: Vec<String>,
) -> GetSelectionOptionsResult {
    if legacy.status == OperationStatus::Error {
        return legacy;
    }
    GetSelectionOptionsResult {
        valid_options,
        ..legacy
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
/// (schema mismatch, empty facet, an incoherent `SelectionState` —
/// configflux-q50t) and unconstrained facets (absent from the
/// symbol table — the `E_SELECTION_UNKNOWN_FACET` diagnostic). For the
/// solver-served path the result envelope is built with the same fields the
/// legacy `selection_options_ok` emits, so the only solver-derived field is
/// `valid_options`. Where the compiler REFUSES instead, its error envelope is
/// returned verbatim and nothing is spliced into it: a failed request has no
/// valid set to report (configflux-tsuf).
pub fn options(request: GetSelectionOptionsRequest) -> GetSelectionOptionsResult {
    // Non-constraint validation (schema version, empty facet, and the integrity
    // of the caller-supplied `SelectionState` — configflux-q50t) is permanently
    // compiler-owned (ADR-0030 D5), delegated so the diagnostics are
    // byte-identical; the solver only decides the satisfiability of a modeled
    // facet query.
    //
    // The state screen matters here for a different reason than on `select`.
    // This path never overrode the compiler's STATUS — the envelope is legacy's
    // merged with the solver's `valid_options`, so an invalid state already
    // came back as the compiler's error. What the merge DID override was the
    // EMPTY option list that refusal carries, publishing options enumerated
    // over an environment the engine had just called incoherent, on top of the
    // diagnostic saying so. An error envelope must carry no payload, so
    // `merge_valid_options` returns any error envelope verbatim and splices
    // nothing into it (configflux-tsuf): this screen still delegates the
    // refusal, and the refusal now reaches the caller whole.
    // ADR-0030 Amendment 2 Rule 1 joins the same block, and must be asked
    // BEFORE the session exists: the replay silently skips an assignment the
    // model cannot hold, so by the time the solver has answered, the
    // inadmissible entry is gone from the deployment being reasoned about.
    if request.schema_version != PRODUCT_SCHEMA_VERSION
        || request.facet.trim().is_empty()
        || !selection_state_is_valid(
            &request.model_handle,
            &request.scope,
            &request.selection_state,
        )
        || !selection_state_is_admissible(&request.model_handle, &request.selection_state)
    {
        return get_selection_options(request);
    }

    // The declared CLOSED facets, which the replay needs to tell a partially
    // modeled facet (skip) from an artifact that has fallen behind its sources
    // (skew). Read once, before the session: `session_from_handle` classifies
    // as it replays. Fails SOFT — an unreadable roster leaves every refusal
    // classified as the non-skew case, exactly today's behavior — and it cannot
    // happen here anyway, since the admissibility screen above already loaded
    // this model.
    let closed = closed_facet_domains(&request.model_handle).unwrap_or_default();

    let session = match session_from_handle(
        &request.model_handle.ccm_ref,
        &request.selection_state,
        &closed,
    ) {
        SolverModel::Usable { session, replay } => {
            // ADR-0030 Amendment 2 Rule 3: a `Rejected` verdict is the solver
            // refusing the STATE, so there is nothing to enumerate. Delegate,
            // because the compiler often sees the contradiction too and owns
            // those bytes; render the conflict here when it does not.
            if let EnvReplay::Rejected { facet, option } = replay {
                let legacy = get_selection_options(request.clone());
                return if legacy.status == OperationStatus::Error {
                    legacy
                } else {
                    selection_options_error(
                        request,
                        selection_conflict_diagnostic(&facet, &option),
                    )
                };
            }
            session
        }
        // ADR-0030 Amendment 2 Rule 2: the artifact does not model a value the
        // sources declare. D4's code, with a message naming the value.
        SolverModel::Skew { facet, option } => {
            return selection_options_unavailable(
                request,
                E_SELECTION_ENGINE_DIVERGENCE,
                &skew_message(&facet, &option),
            );
        }
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

    // Pruned reasons are a legacy-only convenience. Override `valid_options` with
    // the solver's decision as usual, but the legacy `pruned_options` list is the
    // compiler's `domain − compiler_valid_set`, and the compiler UNDER-reports an
    // override-gated facet (configflux-z1hj: on S1 `cooling_model` under
    // `cooling_brand=hydra` the compiler narrows to `["x200"]` while the solver
    // holds the authoritative `["a9","x200"]`). Emitting it verbatim would report
    // an option as BOTH valid and pruned — a self-contradictory `--format json`
    // envelope (configflux-zdf1). Recompute `pruned_options` against the solver's
    // valid set (drop options the solver holds valid): the solver is the authority
    // this path already defers to (ADR-0017 §4 / ADR-0030), staying inside the one
    // seam allowed to consult both engines (ADR-0003 §2) — no solver logic rebuilt
    // in the compiler. Genuinely-pruned reasons/order are untouched (byte-stable).
    if request.include_pruned_reasons {
        let mut legacy = get_selection_options(request);
        if let Some(reasons) = legacy.pruned_options.take() {
            let solver_valid: BTreeSet<&str> = valid_options.iter().map(String::as_str).collect();
            legacy.pruned_options = Some(
                reasons
                    .into_iter()
                    .filter(|pruned| !solver_valid.contains(pruned.option.as_str()))
                    .collect(),
            );
        }
        return merge_valid_options(legacy, valid_options);
    }

    // Take the legacy envelope and override ONLY the valid_options decision with
    // the solver's — byte-identical for every existing field by construction,
    // and untouched altogether when the compiler refused. `merge_valid_options`
    // documents both halves.
    merge_valid_options(get_selection_options(request), valid_options)
}

/// `select` command (ADR-0017 §4/§5): apply a `(facet, option)` selection,
/// with the accept/reject decision and rejection code sourced from
/// `solver::Session::apply` mapped through the §5 table.
///
/// ADR-0030: a usable `.ccm` is a hard precondition. When no usable solver
/// model is reachable the command FAILS CLOSED with
/// `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` (D1). The permanent compiler-owned
/// delegations (D5) remain: schema mismatch, empty facet/option, an incoherent
/// `SelectionState` (configflux-q50t), an already-applied or context-pinned
/// facet (the legacy path renders these without the BDD), and a facet absent
/// from the symbol table (an unconstrained facet). On a solver REJECT the
/// wrapper still asks legacy to
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
    //
    // The integrity of the `SelectionState` ITSELF joins them (configflux-q50t).
    // The whole state is caller-supplied at the SDK seam — `interpreter select`
    // deserializes one from stdin — and the compiler binds six properties to it
    // before it will adjudicate anything: the state's own `schema_version`
    // (a DIFFERENT field from the request's, checked above), a non-empty scope,
    // the model and the scope it is sealed against, its canonical hash, and
    // tags that do not contradict choices. None of the six is a constraint
    // decision, so screening them HERE is D5's division of labor and not the
    // accept-path re-check D3 forbids: the question is whether the request is
    // coherent enough to adjudicate, asked before any adjudication happens.
    //
    // It is screened BEFORE the session is built because the replay cannot
    // catch it. A state whose tags and choices disagree about one facet applies
    // the tag, typed-rejects the choice — which correctly still yields a
    // session — and then accepts a delta on any OTHER facet, so `select`
    // answered Ok with a freshly canonicalized hash for a state the engine
    // refuses. Delegating leaves the canonical `E_SELECTION_STATE_INVALID`
    // bytes the compiler's, exactly as every other arm of this block does.
    if request.schema_version != PRODUCT_SCHEMA_VERSION
        || facet.trim().is_empty()
        || option.trim().is_empty()
        || request.selection_state.context_tags.contains_key(&facet)
        || request.selection_state.choices.contains_key(&facet)
        || !selection_state_is_valid(
            &request.model_handle,
            &request.scope,
            &request.selection_state,
        )
        // ADR-0030 Amendment 2 Rule 1. Integrity is not admissibility: the six
        // bindings above say the state is coherent and sealed against this
        // model, and say nothing about whether the model HAS the facets it
        // names or admits the values it gives them. A hand-authored state with
        // an out-of-domain choice passes all six, and the replay cannot catch
        // it either — the solver simply cannot apply the assignment, so it is
        // skipped and the delta is then adjudicated over a deployment missing
        // the very entry that was wrong.
        || !selection_state_is_admissible(&request.model_handle, &request.selection_state)
    {
        return apply_selection(request);
    }

    // The declared CLOSED facets, which the replay needs to tell a partially
    // modeled facet (skip) from an artifact behind its sources (skew).
    let closed = closed_facet_domains(&request.model_handle).unwrap_or_default();

    let mut session = match session_from_handle(
        &request.model_handle.ccm_ref,
        &request.selection_state,
        &closed,
    ) {
        SolverModel::Usable { session, replay } => {
            // ADR-0030 Amendment 2 Rule 3, and it runs BEFORE the delta is
            // looked at: the solver has refused the STATE, so no delta over it
            // can be accepted, whatever the delta is. Delegate first — the
            // compiler frequently sees the same contradiction and owns those
            // bytes — and render the conflict here only where it does not.
            if let EnvReplay::Rejected { facet, option } = replay {
                let legacy = apply_selection(request);
                return match legacy.status {
                    OperationStatus::Error => legacy,
                    OperationStatus::Ok => apply_error(
                        legacy.model_hash,
                        legacy.scope,
                        selection_conflict_diagnostic(&facet, &option),
                    ),
                };
            }
            session
        }
        // ADR-0030 Amendment 2 Rule 2: artifact skew, D4's code.
        SolverModel::Skew { facet, option } => {
            return apply_unavailable(
                request,
                E_SELECTION_ENGINE_DIVERGENCE,
                &skew_message(&facet, &option),
            );
        }
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
        // renders the canonical rejection diagnostic bytes — and where legacy
        // would have ACCEPTED what the solver refused, the wrapper renders the
        // refusal itself, as `E_SELECTION_CONFLICT` (ADR-0030 D3 as narrowed by
        // Amendment 2).
        //
        // **Why this arm is no longer engine divergence.** D3 assumed two
        // complete engines over one formula, so a disagreement had to be a
        // correctness incident. After ADR-0057 the compiler's check is
        // three-valued and `Unknown` is never a violation (ADR-0054 §2): over an
        // unbound derive-only binding it accepts what the total BDD refuses, by
        // design and on a perfectly good model. Reporting that as an
        // internal-fault code told the user to recompile a model that was doing
        // exactly what it says — the false positive configflux-rzyd removed from
        // `resolve` and configflux-narb met once already on this surface. The
        // compile-time parity tripwire D3's arm was also serving belongs to the
        // link oracle and the lowering test (configflux-dtsq), not to a
        // production envelope.
        Err(solver::Error::Conflict { .. })
        | Err(solver::Error::UnknownOption { .. })
        | Err(solver::Error::UnknownFacet(_)) => {
            let legacy = apply_selection(request);
            match legacy.status {
                // Engines agree on REJECT → legacy's canonical bytes (identical
                // to pre-ADR-0030 behavior on every existing rejection).
                OperationStatus::Error => legacy,
                // The solver is the decision authority; the compiler could not
                // see this contradiction. Report it as the conflict it is,
                // naming the DELTA the solver refused.
                OperationStatus::Ok => apply_error(
                    legacy.model_hash,
                    legacy.scope,
                    selection_conflict_diagnostic(&facet, &option),
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
pub fn resolve(mut request: ResolveFromSelectionRequest) -> ResolveResult {
    // ADR-0030 Amendment 2 Rule 1 (configflux-oime): the state is screened
    // BEFORE the artifact is looked for, so an incoherent or inadmissible state
    // under an unreachable `.ccm` reports the STATE. `resolve` used to reach
    // `load_solver_ccm` first and blame the missing artifact, leaving a caller
    // who fixed what the diagnostic named still holding an invalid state — and
    // `apply` and `options` had already chosen the other precedence, so the
    // three surfaces disagreed about one request.
    if request.schema_version != PRODUCT_SCHEMA_VERSION
        || !selection_state_is_valid(
            &request.model_handle,
            &request.scope,
            &request.selection_state,
        )
        || !selection_state_is_admissible(&request.model_handle, &request.selection_state)
    {
        return resolve_from_selection(request);
    }

    // ADR-0057 §D6 budgets ONE constraint-model load per resolve, so the `.ccm`
    // is read HERE — once — and the parsed model is handed to the gate and then
    // to inference. Neither step reads the artifact itself. `None` is the same
    // "no usable solver model" verdict the gate used to reach at its own load
    // site, so a missing or unreadable `.ccm` still fails closed (ADR-0030 D1).
    let Some(ccm) = load_solver_ccm(&request.model_handle.ccm_ref) else {
        return resolve_unavailable(
            request,
            E_RESOLVE_SOLVER_MODEL_UNAVAILABLE,
            "no usable solver model (.ccm) is reachable to gate resolve satisfiability",
        );
    };

    // The declared CLOSED facets, read ONCE for the whole resolve and shared by
    // the gate (which needs it to classify a refusal) and the inference fixpoint
    // (whose roster it is). Inference used to load its own copy, which made two
    // reads of one thing per resolve for no benefit.
    let closed = closed_facet_domains(&request.model_handle).unwrap_or_default();

    match solver_sat_gate(&ccm, &request.selection_state, &closed) {
        // Solver model present and the selection is satisfiable: the compiler
        // is the source of the response bytes (amendment "compiler composes the
        // entire rich ResolveResult").
        SatVerdict::Satisfiable => {
            // ADR-0057 §D6: the ONE place inference happens, so the interpreter
            // and `cfx` cannot disagree about what the constraints entail. It
            // only ever ADDS to the request; the compiler still composes every
            // byte of the response.
            request.implied_choices = infer_forced_bindings(ccm, &closed, &request);
            resolve_from_selection(request)
        }
        // Unsatisfiable: the compiler still composes the bytes, and on all but
        // one rejection it reaches the same verdict for itself. `assert_
        // unsatisfiable` is the exception (configflux-rzyd).
        //
        // Inference is still run rather than skipped: it fails soft on a
        // rejected replay and returns nothing, so both arms reach
        // `resolve_from_selection` through one code path with one precedence
        // ladder. Short-circuiting here would make the unsat envelope's
        // `implied_choices` depend on which arm produced it.
        SatVerdict::Unsatisfiable { facet, option } => {
            request.implied_choices = infer_forced_bindings(ccm, &closed, &request);
            assert_unsatisfiable(resolve_from_selection(request), &facet, &option)
        }
        // ADR-0030 Amendment 2 Rule 2: the artifact does not model a value the
        // sources declare. `resolve`'s D4 code, with a message naming the value.
        SatVerdict::Skew { facet, option } => resolve_unavailable(
            request,
            E_RESOLVE_SOLVER_MODEL_UNAVAILABLE,
            &skew_message(&facet, &option),
        ),
        // ADR-0030 D1/D4: no usable model, or the gate faulted → fail closed.
        // The solver model is part of every resolve decision's lineage; a
        // missing or faulting gate is no longer silently tolerated.
        SatVerdict::Unavailable | SatVerdict::Fault => resolve_unavailable(
            request,
            E_RESOLVE_SOLVER_MODEL_UNAVAILABLE,
            "no usable solver model (.ccm) is reachable to gate resolve satisfiability",
        ),
    }
}

/// Keep a rejection the SOLVER has already proved unsatisfiable inside the
/// unsatisfiable family, whatever the compiler managed to see (configflux-rzyd),
/// and refuse outright where the compiler saw nothing wrong at all
/// (configflux-im7s).
///
/// # The compiler's `Ok` over a selection the solver has refused
///
/// The first arm exists because a proved-unsatisfiable selection could reach
/// here with a full, successful snapshot attached. The mechanism is the one the
/// next section describes, minus its final step: the contradiction runs through
/// a derive-only binding, the binding is left unbound, the tying constraint
/// evaluates `Unknown` and is therefore not a violation (ADR-0054 §2) — and
/// where NOTHING requires that binding, `resolve_scoped` has no unmet
/// requirement to fail on either. It composes a configuration for a deployment
/// that cannot exist, with a `resolve_hash` over it, while `options` and
/// `explain` both already call the same selection unsatisfiable.
///
/// The early return that used to stand here (`status != Error → return
/// composed`) is what let that through: it was written to pass a compiler
/// SUCCESS along untouched, on the premise that a rejection always arrives as
/// an error. The premise is false, and ADR-0030 Amendment 2 Rule 3 replaces it —
/// the solver's verdict decides, and the wrapper renders it.
///
/// # The one rejection the compiler can get wrong
///
/// `E_RESOLVE_FACET_UNBOUND` carries a CLAIM, not just a name: ADR-0047 §6 and
/// the code's own registry entry say the model **is satisfiable once the facet
/// is bound**, and that claim is the whole reason `cfx` classifies it as a
/// usage error (exit `2`) rather than the valid-input-but-unsatisfiable family
/// (exit `3`, ADR-0042 §3). Reaching this function means the solver has just
/// proved the claim false, so the code must not stand.
///
/// It is reachable through a binding whose value comes from a `derive` table —
/// the shape ADR-0057 §D3/§D4 exists to make ordinary. The contradiction leaves
/// the binding with no value at all: the replay above refused a choice, so
/// `infer_forced_bindings` bails at its own rejected-replay guard and implies
/// nothing, and a derived binding has no default either. The compiler then
/// evaluates its constraints over an assignment that is missing it, where the
/// tying constraint can only come out `Unknown` — never a violation (ADR-0054
/// §2) — so the contradiction is invisible to it and `resolve_scoped` fails on
/// the unmet component requirement instead. The reader is told to bind the
/// facet or give it a default, neither of which can help, while `options` and
/// `explain` both already report the selection unsatisfiable.
///
/// # Why this is the narrowest possible rewrite
///
/// ONE code is rewritten, not "anything outside the unsat family". A model that
/// will not load and output that will not serialize are real faults whose codes
/// must survive a rejection verdict untouched; masking either as a selection
/// conflict would send the user to fix their selection over a broken package.
/// `E_RESOLVE_FACET_UNBOUND` is singled out because it is the only code whose
/// contract the solver's verdict directly contradicts.
///
/// Everything the compiler renders in the family already — the constraint
/// violation `evaluate_constraints` reports when the facets ARE bound, the
/// `E_RESOLVE_CONTEXT_UNSATISFIED` fold — passes through byte-identically, so
/// the composed-by-the-compiler rule (the ADR-0017 amendment) still describes
/// every rejection but this one.
///
/// # Why `E_SELECTION_CONFLICT`
///
/// Because the SAME contradiction, authored with the binding selected
/// explicitly instead of derived, already produces `E_SELECTION_CONFLICT` from
/// the compiler's own constraint evaluation. Reporting one modelling mistake
/// under two codes depending on which authoring construct expressed it is the
/// surface disagreement this whole path exists to remove; `E_SELECTION_
/// UNSATISFIABLE` would read as well in isolation and worse beside its twin.
fn assert_unsatisfiable(mut composed: ResolveResult, facet: &str, option: &str) -> ResolveResult {
    // The compiler saw nothing wrong and composed a payload. The solver has
    // proved there is no such deployment, so the payload must not be delivered
    // (configflux-im7s).
    if composed.status == OperationStatus::Ok {
        return resolve_conflict(composed, facet, option);
    }
    // The FIRST diagnostic, because that is the one `cfx`'s `classify` reads to
    // pick an exit code — rewriting any other would leave the exit code wrong.
    let Some(first) = composed.diagnostics.diagnostics.first_mut() else {
        return composed;
    };
    if first.code != E_RESOLVE_FACET_UNBOUND {
        return composed;
    }

    // Only the three fields that carry the claim, so the compiler's own
    // `source_id` and `entity_path` survive the rewrite: they attribute the
    // diagnostic to the model, and the rewrite changes what is being said, not
    // where it was found.
    let rendered = selection_conflict_diagnostic(facet, option);
    first.code = rendered.code;
    first.message = rendered.message;
    first.hint = rendered.hint;
    composed
}

/// The `resolve` error envelope for a selection the SOLVER proved
/// unsatisfiable and the compiler composed a payload for anyway
/// (ADR-0030 Amendment 2 Rule 3, configflux-im7s).
///
/// Built from the composed result rather than from the request, so the identity
/// fields a client attributes the failure by — the model, the scope, the state
/// hash, and the selection itself — are the ones the compiler just reported. The
/// payload and everything derived from it are dropped, with no `resolve_hash`
/// and no `resolved_output_hash`: nothing was delivered, so there is no delivery
/// to identify (ADR-0059 D3, ADR-0060 D8.2). `implied_choices` goes with them —
/// what the solver entailed on the way to proving the selection impossible is
/// not a fact about any deployment, which is the same rule `resolve_unavailable`
/// follows.
fn resolve_conflict(composed: ResolveResult, facet: &str, option: &str) -> ResolveResult {
    let diagnostics = one_error_report(selection_conflict_diagnostic(facet, option));
    ResolveResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: composed.model_hash,
        scope: composed.scope,
        selection_state_hash: composed.selection_state_hash,
        resolve_hash: None,
        resolved_output_hash: None,
        resolved_output: None,
        context_tags: composed.context_tags,
        choices: composed.choices,
        defaulted_choices: BTreeMap::new(),
        implied_choices: BTreeMap::new(),
        closed_facet_domains: Default::default(),
        resolved_component_dependencies: BTreeMap::new(),
        resolved_artifacts: BTreeMap::new(),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// Every declared CLOSED facet the constraints already decide, given the user's
/// context tags and choices (ADR-0057 §D6).
///
/// Apply tags then choices to a fresh session, then iterate to a fixpoint: a
/// still-unbound closed facet whose domain has collapsed to ONE valid value is
/// bound to it and recorded. Facets are visited in ascending order
/// (`ClosedFacetDomains` is `BTreeMap`-backed) so the sequence of solver calls
/// is byte-stable; the RESULT is order-independent anyway, because forced-
/// literal propagation is confluent. Each productive round strictly shrinks the
/// unbound set, so this terminates in at most one round per facet.
///
/// **Why a zero-option facet aborts everything rather than being skipped.**
/// ADR-0054 §5.2 gives every declared CLOSED facet an `exactly_one_of` conjunct
/// — at-least-one AND pairwise at-most-one (`compiler_core::
/// synthesize_facet_cardinality`, folded into the root by the emitter). Cite
/// §5.2 and not ADR-0047 §4: that earlier synthesis was REMOVED by ADR-0047
/// Amendment 1 and REINSTATED by §5.2/§5.3, so anyone re-deriving this premise
/// from ADR-0047 reaches the opposite conclusion and would "fix" the loop below.
///
/// At-least-one is what makes a satisfiable formula leave every closed facet at
/// least one option, so zero options anywhere proves the formula is ALREADY
/// unsatisfiable and every other facet would return zero too — nothing has been
/// implied yet, and nothing can be. Aborting and skipping therefore agree here.
/// Abort is nonetheless STRICTLY SAFER, and this must not be "simplified" away,
/// because two holes exist where zero options does NOT imply unsat: an OPEN
/// facet gets at-most-one with NO at-least-one, and `Session::valid_options`
/// INTERSECTS across partitions, so a facet spanning two of them can yield an
/// empty intersection with neither partition unsat. In both, aborting degrades
/// to exactly today's behavior while skipping would invent a novel implied set.
/// The roster is closed-only by construction — `closed_facet_domains` omits open
/// facets and the type makes carrying one impossible — which is what keeps the
/// first hole out of reach here.
///
/// Fails SOFT everywhere: an unreadable IR model, a `Ccm` no session will
/// accept, a rejected replay or a solver fault all yield an empty map, which is
/// precisely today's behavior. Inference may only ever ADD information, never
/// cost a resolve.
///
/// Takes the `Ccm` the caller already read BY VALUE: ADR-0057 §D6 budgets one
/// constraint-model load per resolve, and `Session::new` consumes its model, so
/// the last user of it is the one that gets to move it. `domains` — the declared
/// CLOSED facets, bindings included, since ADR-0057 §D3 makes a binding one more
/// closed facet (configflux-secb.4) — is likewise the caller's, read once for
/// the whole resolve and shared with the satisfiability gate.
fn infer_forced_bindings(
    ccm: Ccm,
    domains: &ClosedFacetDomains,
    request: &ResolveFromSelectionRequest,
) -> BTreeMap<String, String> {
    let none = BTreeMap::new();

    if domains.is_empty() {
        return none;
    }

    let Ok(mut session) = Session::<CuddBackend>::new(ccm) else {
        return none;
    };
    if session.ccm().symbols().is_none() {
        return none;
    }

    // The environment, through the ONE shared replay every session in this
    // crate is built with (configflux-bmjt). This function used to carry its
    // own copy of the sequence — the only one of the three that applied context
    // tags — which is exactly how the surfaces came to reason over different
    // deployments. A fresh SESSION is still needed (the gate's cannot be
    // reused); the parsed model already is shared.
    //
    // A REJECTED replay means the environment is already unsatisfiable, so
    // there is nothing left to entail and nothing here to record. `resolve`
    // holds that verdict and sees to it that the rejection is reported as one
    // (`assert_unsatisfiable`, configflux-rzyd). `Skew` and `Fault` bail for the
    // same fail-soft reason they always did.
    //
    // A partially modelled facet no longer lands here (ADR-0030 Amendment 2
    // Rule 2). It used to: one `region=us` tag against a model whose only
    // mention of `region` is `region == 'eu'` returned a flat rejection and
    // abandoned every entailment the constraints held for the WHOLE model —
    // configflux-cy3k's symptom, and a resolve over an ordinary deployment
    // silently losing the binding the model itself decides. The replay now
    // skips such an assignment exactly as it skips an absent facet, so this
    // guard sees only the case its own argument is about.
    let state = &request.selection_state;
    if !matches!(
        apply_environment(&mut session, state, domains),
        EnvReplay::Accepted
    ) {
        return none;
    }

    // Only facets the CCM actually models can be asked. `valid_options` returns
    // `Err(UnknownFacet)` for a facet with no `{facet}.` symbol, and the roster
    // comes from the IR declarations rather than from the symbol table — so ONE
    // declaration-to-CCM skew would otherwise trip the abort above and silently
    // disable inference for the WHOLE model. This is the same guard
    // `apply_environment` applies to the assignments it replays.
    let roster: Vec<(&str, &BTreeSet<String>)> = domains
        .iter()
        .filter(|(facet, _)| facet_present(&session, facet))
        .collect();

    let mut implied: BTreeMap<String, String> = BTreeMap::new();
    loop {
        let mut bound_this_round = false;
        // A roster facet is already bound exactly when the environment named
        // it: the roster is pre-filtered to facets the CCM models, and the
        // replay above returned `Accepted`, so every named modeled facet was
        // applied. Reading the state directly says that in the state's own
        // terms instead of re-deriving it into a set.
        for (facet, declared) in &roster {
            if state.context_tags.contains_key(*facet)
                || state.choices.contains_key(*facet)
                || implied.contains_key(*facet)
            {
                continue;
            }
            let Ok(options) = session.valid_options(facet) else {
                return none;
            };
            match options.options.as_slice() {
                // Already unsatisfiable — see the doc comment above.
                [] => return none,
                // Forced. The declared-domain check is a soundness floor: only a
                // value the model actually declares may be recorded, so a
                // symbol-table artifact can never enter the resolve-hash
                // pre-image as if it were an authored option.
                [only] if declared.contains(only) => {
                    if session.apply(facet, only).is_err() {
                        return none;
                    }
                    implied.insert((*facet).to_string(), only.clone());
                    bound_this_round = true;
                }
                // Still free, or forced to something undeclared: leave it alone
                // and let the declared default (if any) apply.
                _ => {}
            }
        }
        if !bound_this_round {
            return implied;
        }
    }
}

/// Build the fail-closed `resolve` error envelope (ADR-0030 D1/D4), mirroring
/// the legacy `resolve_failed` field layout (carries the request's context_tags
/// and choices so a client can attribute the failure to its selection).
fn resolve_unavailable(
    request: ResolveFromSelectionRequest,
    code: &'static str,
    message: &str,
) -> ResolveResult {
    let diagnostics = fault_report(code, message.to_string(), "model_handle.ccm_ref");
    ResolveResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: request.model_handle.model_hash,
        scope: request.scope,
        selection_state_hash: request.selection_state.selection_state_hash,
        resolve_hash: None,
        // ADR-0059 D3: no payload delivered, so no payload identity.
        resolved_output_hash: None,
        resolved_output: None,
        context_tags: request.selection_state.context_tags,
        choices: request.selection_state.choices,
        defaulted_choices: BTreeMap::new(),
        // ADR-0057 §D6: a resolve that never reached the solver inferred
        // nothing, so there is no implication to report.
        implied_choices: BTreeMap::new(),
        // ADR-0060 D8.2: no payload delivered, so no declarations to attribute
        // against — the same rule the compiler's own `resolve_failed` follows.
        closed_facet_domains: Default::default(),
        resolved_component_dependencies: BTreeMap::new(),
        resolved_artifacts: BTreeMap::new(),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// Read the `.ccm` a model handle points at — the ONE constraint-model load a
/// resolve is allowed (ADR-0057 §D6). The parsed model is then shared by the
/// satisfiability gate and the inference fixpoint, which each need a session of
/// their own but not a file read of their own.
///
/// `None` is exactly what `solver_sat_gate` used to fold into
/// `SatVerdict::Unavailable` at its own load site: an empty ref, or an artifact
/// that will not load (ADR-0030 D1). The other two "no usable model" conditions
/// — a `Session` that will not construct, and a symbol-less stub — need a
/// session and so stay with the gate, the only caller that can turn them into a
/// verdict.
fn load_solver_ccm(ccm_ref: &str) -> Option<Ccm> {
    let ccm_ref = ccm_ref.trim();
    if ccm_ref.is_empty() {
        return None;
    }
    Session::<CuddBackend>::load_ccm(Path::new(ccm_ref)).ok()
}

/// The solver's verdict on whether the current selection is satisfiable over
/// the `.ccm` boolean model.
enum SatVerdict {
    /// A solver model was present and the committed selection is satisfiable.
    Satisfiable,
    /// A solver model was present and the committed selection is
    /// unsatisfiable. Carries the first assignment the solver refused
    /// (configflux-rzyd), which is the only name `resolve` has for the cause
    /// when the compiler cannot see the contradiction for itself.
    Unsatisfiable { facet: String, option: String },
    /// The `.ccm` does not model a value the model sources DECLARE for a closed
    /// facet (ADR-0030 Amendment 2 Rule 2). Fails closed under the same code as
    /// `Unavailable`, because that is exactly what it is — the artifact does not
    /// describe this model — but with a message naming what is missing.
    Skew { facet: String, option: String },
    /// No usable solver model was reachable (empty ref, unloadable artifact,
    /// or symbol-less stub). ADR-0030 D1 → fail closed.
    Unavailable,
    /// The gate faulted on a solver-owned query (Backend/Invariant/Ccm error
    /// while replaying a modeled choice). ADR-0030 D4 → fail closed.
    Fault,
}

/// Gate the current selection's satisfiability with the solver by re-deriving
/// the deployment environment onto a fresh ephemeral session (ADR-0017 §3).
/// Expresses the amendment's "solver gates satisfiability" step explicitly.
/// ADR-0030 D1/D4: this gate is load-bearing — its verdict can fail the
/// command. `Unavailable` (no usable model) and `Fault` (a query fault while
/// replaying a modeled assignment) both make `resolve` fail closed.
fn solver_sat_gate(
    ccm: &Ccm,
    selection_state: &SelectionState,
    closed: &ClosedFacetDomains,
) -> SatVerdict {
    // The caller read the artifact (ONE load per resolve, ADR-0057 §D6) and
    // still needs it for inference, so the gate builds its session from a clone
    // of the parsed model rather than from a second read of the same file.
    // Construct failures remain "no usable model" (ADR-0030 D1), distinct from
    // a query fault on an otherwise-usable model (D4) below; the empty-ref and
    // unreadable-artifact halves of that verdict now live in the caller.
    let Ok(mut session) = Session::<CuddBackend>::new(ccm.clone()) else {
        return SatVerdict::Unavailable;
    };
    if session.ccm().symbols().is_none() {
        return SatVerdict::Unavailable;
    }

    // The environment, through the ONE shared replay (configflux-bmjt). The
    // gate used to see `choices` alone, which made it the only step in a
    // resolve reasoning about a different deployment than the inference
    // fixpoint two lines later. A typed rejection means the environment is
    // unsatisfiable; the compiler resolver renders that rejection wherever it
    // can see the contradiction, and `resolve` renders it where it cannot
    // (configflux-rzyd). A Backend/Invariant/Ccm fault on this solver-owned
    // query fails closed (ADR-0030 D4) — it is not masked as model absence.
    match apply_environment(&mut session, selection_state, closed) {
        EnvReplay::Accepted => SatVerdict::Satisfiable,
        EnvReplay::Rejected { facet, option } => SatVerdict::Unsatisfiable { facet, option },
        EnvReplay::Skew { facet, option } => SatVerdict::Skew { facet, option },
        EnvReplay::Fault => SatVerdict::Fault,
    }
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
/// `model_hash`/`scope` (ADR-0030 D4), used on the fault and skew paths where
/// the request has already been consumed.
fn apply_divergence(
    model_hash: String,
    scope: String,
    code: &'static str,
    message: &str,
) -> ApplySelectionResult {
    apply_error(
        model_hash,
        scope,
        fault_diagnostic(code, message.to_string(), "model_handle.ccm_ref"),
    )
}

/// The same `select` error envelope around an arbitrary diagnostic, for the
/// refusals that are NOT artifact faults — a selection the solver proves
/// unsatisfiable carries the conflict diagnostic and its hint, not "recompile
/// the model" (ADR-0030 Amendment 2 Rule 3).
fn apply_error(
    model_hash: String,
    scope: String,
    diagnostic: Diagnostic,
) -> ApplySelectionResult {
    let diagnostics = one_error_report(diagnostic);
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

    // ADR-0030 Amendment 2 Rule 1, in the shape this surface already uses for
    // every request-invalid shape: `legacy` has ALREADY run the screen and
    // rendered the canonical refusal (`status: error`, exit 2 — the "could not
    // run" class), so returning it verbatim keeps these bytes the compiler's,
    // exactly as the block above does. Asked here rather than folded into that
    // `matches!` because the screen's two codes are ordinary selection codes
    // that mean something else entirely when they come from the probe.
    if !selection_state_is_admissible(&request.model_handle, &request.selection_state) {
        return legacy;
    }

    // The model's CLOSED facet declarations, read BEFORE the session: the replay
    // needs them to classify a refusal (Rule 2), and attribution needs them to
    // complete a core clause that mentions a facet only negatively
    // (configflux-pt6v). One load serves both.
    //
    // Fail SOFT, uniquely on this path. Every other load failure here is a
    // command error, but `legacy` above already ran the canonical request
    // validation and returned early on an unloadable model, so reaching this
    // line means the model loads. If it somehow does not, degrading to
    // asserted-only attribution costs a constraint NAME on a diagnostic; failing
    // the command costs the whole explanation. The former is strictly better.
    let domains = closed_facet_domains(&request.model_handle).unwrap_or_default();

    let session = match session_from_handle(
        &request.model_handle.ccm_ref,
        &request.selection_state,
        &domains,
    ) {
        SolverModel::Usable { session, replay } => {
            // ADR-0030 Amendment 2 Rule 3: the solver refuses the STATE, so the
            // operation cannot run over it — ADR-0031 D2's "could not run"
            // class, exit 2. This is not the probe's rejection, and reporting
            // it as one would answer a question about a deployment that cannot
            // exist.
            if let EnvReplay::Rejected { facet, option } = replay {
                return explain_conflict(request, &facet, &option);
            }
            session
        }
        // ADR-0030 Amendment 2 Rule 2: artifact skew, D4's code.
        SolverModel::Skew { facet, option } => {
            return explain_command_error(
                request,
                E_SELECTION_ENGINE_DIVERGENCE,
                &skew_message(&facet, &option),
            );
        }
        // ADR-0030 D1 / ADR-0031 D4: no usable model → fail closed, exit 2.
        SolverModel::Unavailable => {
            return explain_command_error(
                request,
                E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
                "no usable solver model (.ccm) is reachable to explain the rejection",
            );
        }
    };

    // The declared-constraint roster the artifact itself carries (ADR-0054
    // §5.4). Read from the loaded `.ccm`, not from the model sources, so the
    // ids named in a core are the ids of the artifact that produced it.
    let roster = session.ccm().constraint_roster();

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
            labeled_core_to_unsat_core(core, &roster, &domains),
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
///
/// `roster` is the model's declared-constraint roster from the top-level
/// `ccm.manifest.json` (ADR-0054 §5.4). It is what gives the core clause
/// identity the BDD root cannot: `attribute_core_clauses` names the authored
/// constraint each model clause violates, and reports a core that reduces to
/// synthesized cardinality as the model being over-constrained rather than
/// borrowing a constraint id it did not earn.
///
/// **Public because it is the ONE conversion (configflux-ykae).** The runtime
/// binary's `explain-rejection` composes its own {parameter, value} envelope but
/// must carry the same core inside it as `cfx` and the interpreter do. It used
/// to convert the labeled MUS with a private copy of this function, which drifted
/// — the copy glossed a model clause `blocked by model rule over ...` where this
/// one says `blocked by constraint <id>: <condition>`, and it could not attribute
/// at all because it never read the roster. Both surfaces now call this; there is
/// no second implementation to drift.
///
/// `domains` carries the model's CLOSED facet declarations so a clause that
/// mentions a facet only negatively can still be completed by entailment
/// (configflux-pt6v). Pass an EMPTY map when they are not reachable: that is
/// exactly the pre-entailment, asserted-only behavior, and it is what the
/// runtime surface passes — a `RuntimeSnapshot` carries `model_hash` /
/// `ccm_ref` / `resolve_hash` but no `ModelHandle`, so the runtime cannot open
/// the model sources the declarations live in.
pub fn labeled_core_to_unsat_core(
    core: LabeledCore,
    roster: &[ConstraintRef],
    domains: &ClosedFacetDomains,
) -> UnsatCore {
    let clauses: Vec<CoreClause> = core
        .conflicting_constraints
        .iter()
        .map(constraint_to_core_clause)
        .collect();
    let declared: Vec<DeclaredConstraint> = roster
        .iter()
        .map(|entry| DeclaredConstraint {
            id: entry.id.clone(),
            condition: entry.condition.clone(),
            root_index: entry.root_index,
        })
        .collect();
    let rejected = atom_to_facet(&core.rejected);
    UnsatCore {
        conflicting_constraints: attribute_core_clauses(&clauses, &rejected, &declared, domains),
        rejected,
        minimal: core.minimal,
        // Fixed advisory string acknowledging non-uniqueness (ADR-0031 D3): MUS
        // extraction returns *a* minimal explanation, not *the* canonical one.
        note: "one minimal explanation; other minimal cores may exist".to_string(),
    }
}

/// Convert one solver `LabeledConstraint` into the solver-agnostic
/// `CoreClause` the compiler-side attribution consumes, carrying the signed
/// forbidden assignment (`LabeledConstraint::forbidden`) that makes the
/// ADR-0054 §5.4 mapping possible.
///
/// The `summary` here is the caller's advisory gloss (ADR-0031 D3: advisory
/// text, not a parsed field) and survives only for a `Selection`; a
/// `ModelRule`'s gloss is derived by the attribution from the declared
/// constraint it names. The JSON→text presentation renderer is a separate,
/// layered concern (configflux-9d28 / ADR-0031 D5).
fn constraint_to_core_clause(constraint: &LabeledConstraint) -> CoreClause {
    CoreClause {
        kind: kind_to_constraint_kind(constraint.kind),
        facets: constraint.atoms.iter().map(atom_to_facet).collect(),
        summary: summarize_constraint(constraint),
        forbidden: constraint
            .forbidden
            .iter()
            .map(|literal| (atom_to_facet(&literal.atom), literal.asserted))
            .collect(),
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
        // A model clause's gloss is replaced by `attribute_core_clauses` with
        // either the declared constraint's condition text or the
        // model-over-constrained sentence, so this is a fallback only. It no
        // longer embeds "blocked by model rule", which the renderer already
        // prefixes — that duplication was configflux-hdgn.
        CoreConstraintKind::ModelRule => format!("relating {}", atoms.join(", ")),
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
    explain_error(
        request,
        fault_diagnostic(code, message.to_string(), "model_handle.ccm_ref"),
    )
}

/// The `explain-rejection` command error for a state the SOLVER proves
/// unsatisfiable (ADR-0030 Amendment 2 Rule 3): `status: error`, exit `2`,
/// `E_SELECTION_CONFLICT`, no core, naming the assignment the solver refused.
///
/// Deliberately NOT the exit-`0` rejection path. A rejection is not an error
/// (ADR-0031 D2) — but that rule is about the OPTION being probed, and here the
/// probe never got asked: the deployment the question was about is one the model
/// cannot hold. Reporting it as a successful "here is why your option is
/// rejected" would answer a question nobody could act on.
fn explain_conflict(
    request: ExplainRejectionRequest,
    facet: &str,
    option: &str,
) -> ExplainRejectionResult {
    explain_error(request, selection_conflict_diagnostic(facet, option))
}

/// The shared `explain-rejection` command-error envelope: `status: error`,
/// exit `2`, no core, with the given diagnostic carried in BOTH the
/// `rejection` reason and the diagnostics report, so the two never disagree.
/// Mirrors the legacy `explain_rejection_failed` field layout.
fn explain_error(
    request: ExplainRejectionRequest,
    diagnostic: Diagnostic,
) -> ExplainRejectionResult {
    let facet = request.rejected_option.facet;
    let option = request.rejected_option.option;
    let rejection = RejectionReason {
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        blocking_choices: BTreeMap::new(),
        hint: diagnostic.hint.clone(),
        unsat_core: None,
    };
    let diagnostics = one_error_report(diagnostic);
    ExplainRejectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: request.model_handle.model_hash,
        scope: request.scope,
        facet,
        option,
        rejection,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

// configflux-secb.3 (ADR-0057 §D6): the inference fixpoint's own suite. It
// lives in this crate because this crate is where the decision is made — the
// interpreter and `cfx` both reach it through `resolve`, so a test that drove
// either binary would be testing the caller, not the rule.
#[cfg(test)]
mod tests;
