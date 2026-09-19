// SPDX-License-Identifier: BUSL-1.1
//
// Runtime-side `explain-rejection` solver wrapper (configflux-3b5y, ADR-0031
// D1–D4). Answers "why would setting this parameter to this value be rejected?"
// by driving `solver::Session::explain_rejection` over the open session's `.ccm`
// and converting the solver-owned labeled MUS into the compiler-side
// `UnsatCore` envelope.
//
// # Where the conversion lives and why (ADR-0003 §2, configflux-ykae)
//
// The MUS extraction lives in the `solver` crate (it owns the BDD/SAT machinery
// and returns a SOLVER-OWNED `LabeledCore` carrying labeled `{facet}.{value}`
// strings only — never a raw BDD/batsat index). The compiler must never import
// `solver`, so the conversion `LabeledCore` → `compiler::loader_api::UnsatCore`
// cannot live in `solver/` or `compiler/`. It lives in `session_compose`, the
// composition seam that may consult both (ADR-0003 §2 amendment), and this
// module CALLS it — `session_compose::labeled_core_to_unsat_core`.
//
// It used to live here, duplicated. That duplicate drifted: it glossed a model
// clause `blocked by model rule over cpu.highperf, cooling.air` where the shared
// one says `blocked by constraint highperf_requires_liquid: cpu != 'highperf' ||
// cooling != 'air'`, and having no `.ccm` roster in hand it could not name a
// declared `constraints:` entry at all (ADR-0054 §5.4). `cfx` and the interpreter
// reach explain through `session_compose::explain`; this command composes its own
// {parameter, value} envelope, but the core INSIDE that envelope is now the very
// same core, produced by the very same function.
//
// # The {parameter, value} <-> {facet, option} mapping (settled Q4)
//
// The runtime speaks **{parameter, value}**: a write path
// `component.<component_id>.param.<param_key>` plus a scalar `value`. The solver
// speaks **{facet, option}**: a boolean `{facet}.{value}` symbol (ADR-0005 §3).
// The mapping this wrapper applies — identical to the one `solver_validation.rs`
// already uses for `set-parameter` — is:
//
//     {parameter}  ──►  {facet}   :  the facet the parameter DECLARES it is the
//                                    handle for (ADR-0064 D1), read from the
//                                    resolved parameter via `bound_facet`
//     {value}      ──►  {option}  :  the request's string `value`
//
// It used to be the path's last segment. A parameter that merely shares a
// facet's name is not that facet's handle (ADR-0064), so the explanation now
// names the facet the model declared rather than one the path happened to spell.
//
// A non-string `value` names no `{facet}.{value}` symbol, a path that is not
// `component.<id>.param.<key>` is a free-form scalar the solver does not govern,
// and a parameter that declares no binding names no modeled facet at all. All
// three are permanent division-of-labor cases (ADR-0030 D5): there is no modeled
// option to explain, so the wrapper returns the unknown-facet rejection with no
// core. The compiler-side result echoes the {parameter, value} identity
// (`path`/`value`) so the response is self-describing in the caller's vocabulary
// while the core inside it is the shared {facet, option} `UnsatCore`.
//
// # Outcome map (ADR-0031 D2/D3/D4)
//
//   * Genuine constraint conflict (`Ok { would_reject: true, core: Some }`)
//       → SUCCESS (exit 0), `status: Ok`, `code = E_SELECTION_CONFLICT`,
//         `rejection.unsat_core = Some(<converted core>)`. A rejection
//         explanation is a successful query (ADR-0031 D2): the command answered
//         the question it was asked.
//   * Genuinely valid option (`Ok { would_reject: false, core: None }`)
//       → SUCCESS (exit 0), `status: Ok`, no core. There is nothing to explain;
//         the candidate would be accepted (ADR-0030 D5).
//   * Unknown facet (`Err(UnknownFacet)`), non-string value, non-facet path, or
//     a facet absent from the symbol table
//       → COMMAND ERROR (exit 2), `code = E_SELECTION_UNKNOWN_FACET`, no core
//         (division-of-labor; ADR-0031 D3/ADR-0030 D5).
//   * Unknown option (`Err(UnknownOption)`)
//       → COMMAND ERROR (exit 2), `code = E_SELECTION_INVALID_OPTION`, no core.
//   * Any other solver `Err` — MUS-extraction fault, unmappable variable, or a
//     Backend/Invariant/Ccm fault, AND a `.ccm` load/construct failure on a
//     facet-shaped write
//       → FAIL CLOSED (exit 2), `code = E_SELECTION_ENGINE_DIVERGENCE`, no core
//         (ADR-0031 D4). After a valid `runtime-open` the model is known usable
//         (ADR-0030 D2), so a fault here is an internal divergence, never a
//         "degraded explanation". Never a partial core, never a raw index.

use compiler::loader_api::{
    ConstraintKind, RejectionReason, UnsatCore, E_SELECTION_CONFLICT,
    E_SELECTION_ENGINE_DIVERGENCE, E_SELECTION_INVALID_OPTION, E_SELECTION_UNKNOWN_FACET,
};
use compiler::product_api::{
    Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
};
use compiler::runtime_api::{
    RuntimeExplainRejectionRequest, RuntimeExplainRejectionResult, RuntimeSnapshot,
};
use compiler::schema::Value;
// The ONE path-to-declared-facet lookup and the ONE symbol-table facet
// predicate. Both were duplicated verbatim here until configflux-jraj lifted
// them, so the `explain-rejection` and write surfaces cannot drift on what a
// facet write is.
use crate::solver_validation::{bound_facet, facet_present};
use solver::{CuddBackend, RejectionExplanation, Session};
use std::path::Path;

/// Explain why setting `path` to `value` would be rejected against the snapshot.
///
/// The single CLI handler for the runtime `explain-rejection` command. Builds an
/// ephemeral `Session<CuddBackend>` from the snapshot's `ccm_ref`, replays the
/// committed `choices`, and asks the solver to explain the candidate
/// `(declared facet, value)` selection. See the module header for the full outcome
/// map and the {parameter, value} <-> {facet, option} mapping.
pub(crate) fn explain_rejection_via_solver(
    request: RuntimeExplainRejectionRequest,
) -> RuntimeExplainRejectionResult {
    let snapshot = &request.runtime_snapshot;

    // {value} -> {option}: only a string value can name a `{facet}.{value}`
    // symbol. A non-string value is a free-form scalar with no modeled option
    // (ADR-0030 D5) — there is no rejection to explain.
    let value_str = match &request.value {
        Value::String(s) => s.as_str(),
        _ => {
            return unknown_facet(
                &request,
                "the requested value is not a string, so it names no modeled \
                 facet option to explain",
            );
        }
    };

    // {parameter} -> {facet}: the candidate facet is the one the parameter
    // DECLARES it is the handle for (ADR-0064 D5.4). A path that is not
    // `component.<id>.param.<key>`, one no resolved parameter answers to, and
    // one whose parameter declares no binding all name no modeled facet — each
    // is a free-form scalar the solver does not govern (ADR-0030 D5).
    let Some(declared_facet) = bound_facet(snapshot, &request.path) else {
        return unknown_facet(
            &request,
            "the parameter declares no facet binding, so it names no modeled \
             facet to explain",
        );
    };
    let candidate_facet = declared_facet.as_str();

    // Re-derive the session from the snapshot (ADR-0017 §3). After a valid
    // runtime-open the `.ccm` is known usable (ADR-0030 D2); a load/construct
    // failure here on a facet-shaped write is an internal fault that FAILS
    // CLOSED (ADR-0031 D4), not a "no model" degrade.
    let ccm = match Session::<CuddBackend>::load_ccm(Path::new(snapshot.ccm_ref.trim())) {
        Ok(ccm) => ccm,
        Err(_) => return engine_divergence(&request, candidate_facet, value_str),
    };
    let mut session = match Session::<CuddBackend>::new(ccm) {
        Ok(session) => session,
        Err(_) => return engine_divergence(&request, candidate_facet, value_str),
    };

    // If the DECLARED facet is absent from the symbol table, the write is a
    // free-form scalar — an unconstrained facet the solver does not own
    // (ADR-0030 D5). No modeled option to explain → unknown facet, no core.
    if !facet_present(&session, candidate_facet) {
        return unknown_facet(
            &request,
            "the parameter is not a solver-modeled facet, so there is no \
             constraint conflict to explain",
        );
    }

    // Replay the session's TOTAL known assignment (ADR-0017 §3 as corrected by
    // the 2026-08-03 amendment), not `snapshot.choices` alone. This loop used to
    // be a byte-identical copy of the write path's, and carried the identical
    // blind spot: a sibling written during the session lands in an overlay, and
    // an overlay was never replayed — so explaining a candidate against an
    // in-session write reported "no conflict to explain" for a pair the write
    // path itself would reject. Both surfaces now project the snapshot the same
    // way, so they agree on what the session knows.
    //
    // The facet under explanation is skipped (the candidate supersedes any prior
    // pin on it). Re-deriving an already-accepted state must never spuriously
    // fault, so a replay error is ignored here — the explain call below is the
    // authority on the candidate decision.
    // ADR-0064 D5.2: two parameters declaring one facet cannot both be current,
    // so a divergence is a fault in the snapshot, not a question about the
    // candidate — fail CLOSED rather than explain against a state the model
    // cannot produce.
    let assignment = match crate::solver_validation::session_assignment(&session, snapshot) {
        Ok(assignment) => assignment,
        Err(divergence) => {
            return divergent_binding(&request, &divergence);
        }
    };
    for (facet, option) in assignment {
        if facet == candidate_facet {
            continue;
        }
        let _ = session.apply(&facet, &option);
    }

    // The declared-constraint roster the artifact itself carries (ADR-0054
    // §5.4). Read from the loaded `.ccm`, not from the model sources, so the ids
    // named in a core are the ids of the artifact that produced it. This is what
    // the retired local converter never had, and why it could not name a
    // `constraints:` entry (configflux-ykae).
    let roster = session.ccm().constraint_roster();

    // Ask the solver to explain the candidate. The solver decides; this wrapper
    // only composes the envelope (ADR-0017 amendment: solver DECIDES, the
    // compose layer COMPOSES).
    match session.explain_rejection(candidate_facet, value_str) {
        // Genuine constraint conflict with a labeled MUS → success, carry the
        // converted core (ADR-0031 D2/D3). The conversion is the shared one
        // `cfx`/interpreter get, so both surfaces report one core, one wording.
        Ok(RejectionExplanation {
            would_reject: true,
            core: Some(core),
        }) => explain_conflict(
            &request,
            candidate_facet,
            value_str,
            // The model's CLOSED facet declarations, carried on the open
            // contract (ADR-0060 D3/D4). The runtime still holds no
            // `ModelHandle` and still cannot open model sources — the resolver
            // computed this table and the open payload forwarded it, which is
            // what lets a core mentioning a closed facet ONLY negatively be
            // completed by entailment and name the constraint it breaks
            // (configflux-pt6v, configflux-tkwt). Empty when the opener supplied
            // none: byte-for-byte the asserted-only attribution this surface had
            // before (D7).
            session_compose::labeled_core_to_unsat_core(
                core,
                &roster,
                &snapshot.closed_facet_domains,
            ),
        ),
        // `would_reject` without a core would be a solver contract violation
        // (a genuine reject must carry its MUS, ADR-0031 D3). Treat the missing
        // core as a fail-closed fault rather than emitting a coreless conflict.
        Ok(RejectionExplanation {
            would_reject: true,
            core: None,
        }) => engine_divergence(&request, candidate_facet, value_str),
        // Genuinely valid option: nothing to explain (ADR-0030 D5) → success,
        // no core.
        Ok(RejectionExplanation {
            would_reject: false,
            ..
        }) => explain_not_rejected(&request, candidate_facet, value_str),
        // Division-of-labor unknown cases (ADR-0030 D5): the same typed errors
        // `apply` raises. Exit 2, no core (acceptance criterion #2).
        Err(solver::Error::UnknownFacet(_)) => unknown_facet(
            &request,
            "the facet is not present in the compiled model",
        ),
        Err(solver::Error::UnknownOption { .. }) => {
            invalid_option(&request, candidate_facet, value_str)
        }
        // MUS fault, unmappable variable, or Backend/Invariant/Ccm fault →
        // FAIL CLOSED (ADR-0031 D4). Never a partial core.
        Err(_) => engine_divergence(&request, candidate_facet, value_str),
    }
}

/// Build the SUCCESS envelope for a genuine constraint-conflict explanation
/// (`status: Ok`, exit 0): the rejection carries `E_SELECTION_CONFLICT` and the
/// converted labeled core (ADR-0031 D2/D3).
fn explain_conflict(
    request: &RuntimeExplainRejectionRequest,
    facet: &str,
    option: &str,
    core: UnsatCore,
) -> RuntimeExplainRejectionResult {
    let rejection = RejectionReason {
        code: E_SELECTION_CONFLICT.to_string(),
        message: format!(
            "Setting '{}' to '{option}' conflicts with the current configuration",
            request.path
        ),
        blocking_choices: blocking_choices_from_core(&core),
        hint: Some(format!(
            "Choose a value for facet '{facet}' consistent with the current selection"
        )),
        unsat_core: Some(core),
    };
    ok_result(request, rejection)
}

/// Build the SUCCESS envelope for a genuinely-valid candidate (`status: Ok`,
/// exit 0, no core): there is no rejection to explain (ADR-0030 D5). The
/// `rejection` payload reports "not rejected" with no code and no core, so a
/// caller can distinguish "valid" from "conflict" by the presence of the core.
fn explain_not_rejected(
    request: &RuntimeExplainRejectionRequest,
    facet: &str,
    option: &str,
) -> RuntimeExplainRejectionResult {
    let rejection = RejectionReason {
        code: String::new(),
        message: format!(
            "Setting '{}' to '{option}' is consistent with the current \
             configuration; facet '{facet}' has no conflict to explain",
            request.path
        ),
        blocking_choices: std::collections::BTreeMap::new(),
        hint: None,
        unsat_core: None,
    };
    ok_result(request, rejection)
}

/// Build the COMMAND-ERROR envelope for an unknown facet / non-facet path /
/// non-string value (`status: Error`, exit 2, no core): a division-of-labor
/// case the solver does not own (ADR-0030 D5, ADR-0031 D3). Acceptance
/// criterion #2.
fn unknown_facet(
    request: &RuntimeExplainRejectionRequest,
    message: &str,
) -> RuntimeExplainRejectionResult {
    let rejection = RejectionReason {
        code: E_SELECTION_UNKNOWN_FACET.to_string(),
        message: message.to_string(),
        blocking_choices: std::collections::BTreeMap::new(),
        hint: Some(
            "Pass a 'component.<id>.param.<key>' path whose parameter is a \
             solver-modeled facet"
                .to_string(),
        ),
        unsat_core: None,
    };
    error_result(request, rejection)
}

/// Build the COMMAND-ERROR envelope for an unknown option on a known facet
/// (`status: Error`, exit 2, no core): division-of-labor (ADR-0030 D5).
fn invalid_option(
    request: &RuntimeExplainRejectionRequest,
    facet: &str,
    option: &str,
) -> RuntimeExplainRejectionResult {
    let rejection = RejectionReason {
        code: E_SELECTION_INVALID_OPTION.to_string(),
        message: format!("'{option}' is not a known option for facet '{facet}'"),
        blocking_choices: std::collections::BTreeMap::new(),
        hint: Some(format!(
            "Choose one of the modeled options for facet '{facet}'"
        )),
        unsat_core: None,
    };
    error_result(request, rejection)
}

/// Build the FAIL-CLOSED envelope for an internal solver fault (`status: Error`,
/// exit 2, no core): MUS-extraction fault, unmappable variable, or a
/// Backend/Invariant/Ccm fault on a write the model is known to govern
/// (ADR-0031 D4). Never a partial core, never a raw index.
fn engine_divergence(
    request: &RuntimeExplainRejectionRequest,
    facet: &str,
    option: &str,
) -> RuntimeExplainRejectionResult {
    let rejection = RejectionReason {
        code: E_SELECTION_ENGINE_DIVERGENCE.to_string(),
        message: format!(
            "The solver faulted while explaining the rejection of '{option}' for \
             facet '{facet}'"
        ),
        blocking_choices: std::collections::BTreeMap::new(),
        hint: Some(
            "Recompile the model so a usable .ccm is emitted, then re-open and \
             retry the explanation"
                .to_string(),
        ),
        unsat_core: None,
    };
    error_result(request, rejection)
}

/// The ADR-0064 D5.2 FAIL-CLOSED envelope: two parameters declaring one facet
/// were observed holding different values. D2.4 refuses that at compile time, so
/// a snapshot that passed the open-time `resolve_hash` check cannot carry it;
/// observing it anyway means the snapshot and the model disagree, which is an
/// engine divergence (ADR-0031 D4) and not an explanation. The retired
/// skip-on-disagreement rule silently dropped the facet here instead.
fn divergent_binding(
    request: &RuntimeExplainRejectionRequest,
    divergence: &crate::solver_validation::DivergentBinding,
) -> RuntimeExplainRejectionResult {
    let rejection = RejectionReason {
        code: E_SELECTION_ENGINE_DIVERGENCE.to_string(),
        message: divergence.message(),
        blocking_choices: std::collections::BTreeMap::new(),
        hint: Some(
            "Re-resolve and re-open against the compiled model: one parameter \
             declares each facet, so two bound values cannot both be current"
                .to_string(),
        ),
        unsat_core: None,
    };
    error_result(request, rejection)
}

/// The `blocking_choices` map (facet -> option) derived from the prior-selection
/// entries of a core. Populated from `kind == Selection` constraints so the
/// existing `RejectionReason.blocking_choices` field stays meaningful for the
/// conflict path, mirroring how the compiler's classifier fills it.
fn blocking_choices_from_core(core: &UnsatCore) -> std::collections::BTreeMap<String, String> {
    let mut choices = std::collections::BTreeMap::new();
    for constraint in &core.conflicting_constraints {
        if constraint.kind == ConstraintKind::Selection {
            for facet in &constraint.facets {
                choices.insert(facet.facet.clone(), facet.option.clone());
            }
        }
    }
    choices
}

/// Build an `Ok` result envelope (exit 0) carrying `rejection`. Used for the two
/// success outcomes: a genuine rejection explanation and a "not rejected"
/// answer. No diagnostics are emitted — a successful query is not an error
/// condition.
fn ok_result(
    request: &RuntimeExplainRejectionRequest,
    rejection: RejectionReason,
) -> RuntimeExplainRejectionResult {
    let snapshot: &RuntimeSnapshot = &request.runtime_snapshot;
    RuntimeExplainRejectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash: snapshot.model_hash.clone(),
        scope: snapshot.scope.clone(),
        path: request.path.clone(),
        value: request.value.clone(),
        rejection,
        error_count: 0,
        warning_count: 0,
        diagnostics_ref: None,
        diagnostics: DiagnosticsReport {
            schema_version: PRODUCT_SCHEMA_VERSION,
            diagnostics: Vec::new(),
            error_count: 0,
            warning_count: 0,
        },
    }
}

/// Build an `Error` result envelope (exit 2) carrying `rejection`, with a single
/// matching diagnostic. Used for the division-of-labor and fail-closed outcomes
/// (ADR-0031 D4).
fn error_result(
    request: &RuntimeExplainRejectionRequest,
    rejection: RejectionReason,
) -> RuntimeExplainRejectionResult {
    let snapshot: &RuntimeSnapshot = &request.runtime_snapshot;
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![Diagnostic {
            code: rejection.code.clone(),
            severity: DiagnosticSeverity::Error,
            message: rejection.message.clone(),
            source_id: None,
            entity_path: Some(request.path.clone()),
            hint: rejection.hint.clone(),
        }],
        error_count: 1,
        warning_count: 0,
    };
    RuntimeExplainRejectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: snapshot.model_hash.clone(),
        scope: snapshot.scope.clone(),
        path: request.path.clone(),
        value: request.value.clone(),
        rejection,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    //! Unit coverage for the core this command puts on the wire. The
    //! `LabeledCore` -> `UnsatCore` conversion itself is no longer owned here —
    //! it is `session_compose::labeled_core_to_unsat_core`, the one both this
    //! surface and `cfx` call (configflux-ykae) — so these tests pin the
    //! properties the RUNTIME depends on across that call: the
    //! {facet, value} -> {facet, option} mapping, the kind translation, the
    //! labeled-name invariant, the advisory note, the ADR-0054 §5.4 rule that an
    //! unattributed model clause is reported as over-constrained rather than
    //! given an id it did not earn, and the `blocking_choices` derivation. All
    //! pure-function, no solver/BDD dependency. That the two surfaces agree on a
    //! real `.ccm` is `run_051`; the end-to-end path is `run_046`/`run_047`.

    use super::*;
    use compiler::loader_api::{
        ClosedFacetDomains, ConflictingConstraint, ConstraintFacet,
        MODEL_OVER_CONSTRAINED_SUMMARY,
    };
    use session_compose::labeled_core_to_unsat_core;
    use solver::{CoreConstraintKind, LabeledAtom, LabeledConstraint, LabeledCore};

    fn atom(facet: &str, value: &str) -> LabeledAtom {
        LabeledAtom {
            facet: facet.to_string(),
            value: value.to_string(),
        }
    }

    /// One signed literal of the partial assignment a clause forbids
    /// (ADR-0054 §5.4). `asserted` is the sign the BDD falsifying path had.
    fn literal(facet: &str, value: &str, asserted: bool) -> solver::LabeledLiteral {
        solver::LabeledLiteral {
            atom: atom(facet, value),
            asserted,
        }
    }

    /// A `LabeledCore` carrying one prior-selection constraint and one
    /// multi-atom model rule converts to the ADR-0031 D3 `UnsatCore` shape:
    /// `{facet, value}` becomes `{facet, option}`, the kinds map across, the
    /// fixed note is attached, and `minimal` is carried through.
    ///
    /// The roster is empty here, which is exactly the ADR-0054 §5.4 boundary
    /// case: no declared constraint can account for the model clause, so it must
    /// be reported as the model being over-constrained and must NOT be handed an
    /// id. The old local converter reached this shape by never attributing at
    /// all; the shared one reaches it by finding nothing to attribute to.
    #[test]
    fn shared_conversion_maps_facets_kinds_and_note() {
        let core = LabeledCore {
            rejected: atom("database", "postgres"),
            conflicting_constraints: vec![
                LabeledConstraint {
                    kind: CoreConstraintKind::Selection,
                    atoms: vec![atom("storage", "local")],
                    forbidden: vec![literal("storage", "local", true)],
                },
                LabeledConstraint {
                    kind: CoreConstraintKind::ModelRule,
                    atoms: vec![atom("database", "postgres"), atom("storage", "remote")],
                    forbidden: vec![
                        literal("database", "postgres", true),
                        literal("storage", "remote", false),
                    ],
                },
            ],
            minimal: true,
        };

        let converted = labeled_core_to_unsat_core(core, &[], &ClosedFacetDomains::default());

        assert_eq!(converted.rejected.facet, "database");
        assert_eq!(converted.rejected.option, "postgres");
        assert!(converted.minimal);
        assert_eq!(
            converted.note,
            "one minimal explanation; other minimal cores may exist"
        );
        assert_eq!(converted.conflicting_constraints.len(), 2);

        let selection = &converted.conflicting_constraints[0];
        assert_eq!(selection.kind, ConstraintKind::Selection);
        assert_eq!(selection.facets, vec![ConstraintFacet {
            facet: "storage".to_string(),
            option: "local".to_string(),
        }]);
        assert!(selection.summary.contains("storage.local"));

        // ADR-0054 §5.4: nothing in the (empty) roster accounts for this clause,
        // so it reports the model as over-constrained and carries no id. A
        // synthesized cardinality conjunct reaches this same path — which is why
        // the rule is "attributed OR over-constrained", never "nearest match".
        let model_rule = &converted.conflicting_constraints[1];
        assert_eq!(model_rule.kind, ConstraintKind::ModelRule);
        assert_eq!(model_rule.facets.len(), 2);
        assert_eq!(model_rule.constraint_id, None);
        assert_eq!(model_rule.summary, MODEL_OVER_CONSTRAINED_SUMMARY);
    }

    /// Every emitted atom is a labeled `{facet}.{option}` name — never a bare
    /// integer. Guards the configflux-osp labeled-MUS invariant at the
    /// conversion boundary (the solver type already carries labels; the
    /// converter must not lose them).
    #[test]
    fn shared_conversion_emits_only_labeled_names() {
        let core = LabeledCore {
            rejected: atom("engine", "v8"),
            conflicting_constraints: vec![LabeledConstraint {
                kind: CoreConstraintKind::ModelRule,
                atoms: vec![atom("engine", "v6"), atom("engine", "v8")],
                forbidden: vec![
                    literal("engine", "v6", true),
                    literal("engine", "v8", true),
                ],
            }],
            minimal: true,
        };
        let converted = labeled_core_to_unsat_core(core, &[], &ClosedFacetDomains::default());
        let is_label = |s: &str| !s.is_empty() && !s.chars().all(|c| c.is_ascii_digit());
        assert!(is_label(&converted.rejected.facet) && is_label(&converted.rejected.option));
        for constraint in &converted.conflicting_constraints {
            for f in &constraint.facets {
                assert!(
                    is_label(&f.facet) && is_label(&f.option),
                    "atom must be labeled, not a raw index: {}.{}",
                    f.facet,
                    f.option
                );
            }
        }
    }

    /// `blocking_choices` is populated from the prior-Selection constraints of
    /// the core (facet -> option), and ignores ModelRule constraints.
    #[test]
    fn blocking_choices_uses_only_prior_selections() {
        let core = UnsatCore {
            rejected: ConstraintFacet {
                facet: "database".to_string(),
                option: "postgres".to_string(),
            },
            conflicting_constraints: vec![
                ConflictingConstraint {
                    kind: ConstraintKind::Selection,
                    facets: vec![ConstraintFacet {
                        facet: "storage".to_string(),
                        option: "local".to_string(),
                    }],
                    summary: String::new(),
                    constraint_id: None,
                },
                ConflictingConstraint {
                    kind: ConstraintKind::ModelRule,
                    facets: vec![ConstraintFacet {
                        facet: "network".to_string(),
                        option: "offline".to_string(),
                    }],
                    summary: String::new(),
                    constraint_id: None,
                },
            ],
            minimal: true,
            note: String::new(),
        };
        let choices = blocking_choices_from_core(&core);
        assert_eq!(choices.get("storage").map(String::as_str), Some("local"));
        assert!(
            !choices.contains_key("network"),
            "model-rule atoms must not appear in blocking_choices"
        );
    }
}
