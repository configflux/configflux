// SPDX-License-Identifier: BUSL-1.1
//
// Runtime-side `explain-rejection` solver wrapper (configflux-3b5y, ADR-0031
// D1–D4). Answers "why would setting this parameter to this value be rejected?"
// by driving `solver::Session::explain_rejection` over the open session's `.ccm`
// and converting the solver-owned labeled MUS into the compiler-side
// `UnsatCore` envelope.
//
// # Where this lives and why (ADR-0003 §2)
//
// The MUS extraction lives in the `solver` crate (it owns the BDD/SAT machinery
// and returns a SOLVER-OWNED `LabeledCore` carrying labeled `{facet}.{value}`
// strings only — never a raw BDD/batsat index). The compiler must never import
// `solver`, so the conversion `LabeledCore` → `compiler::loader_api::UnsatCore`
// CANNOT live in `solver/` or `compiler/`. It lives HERE, in the `runtime`
// crate, which already depends on both — exactly the boundary
// `solver_validation.rs` (the `set-parameter` pre-check) and the interpreter's
// `solver_session.rs` already occupy. This is the single point where the two
// vocabularies meet.
//
// # The {parameter, value} <-> {facet, option} mapping (settled Q4)
//
// The runtime speaks **{parameter, value}**: a write path
// `component.<component_id>.param.<param_key>` plus a scalar `value`. The solver
// speaks **{facet, option}**: a boolean `{facet}.{value}` symbol (ADR-0005 §3).
// The mapping this wrapper applies — identical to the one `solver_validation.rs`
// already uses for `set-parameter` — is:
//
//     {parameter}  ──►  {facet}   :  param_key  (the last path segment of
//                                    `component.<id>.param.<param_key>`)
//     {value}      ──►  {option}  :  the request's string `value`
//
// A non-string `value` names no `{facet}.{value}` symbol, and a path that is not
// `component.<id>.param.<key>` is a free-form scalar the solver does not govern.
// Both are permanent division-of-labor cases (ADR-0030 D5): there is no modeled
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
    ConflictingConstraint, ConstraintFacet, ConstraintKind, RejectionReason, UnsatCore,
    E_SELECTION_CONFLICT, E_SELECTION_ENGINE_DIVERGENCE, E_SELECTION_INVALID_OPTION,
    E_SELECTION_UNKNOWN_FACET,
};
use compiler::product_api::{
    Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
};
use compiler::runtime_api::{
    RuntimeExplainRejectionRequest, RuntimeExplainRejectionResult, RuntimeSnapshot,
};
use compiler::schema::Value;
use solver::{
    CoreConstraintKind, CuddBackend, LabeledAtom, LabeledConstraint, LabeledCore,
    RejectionExplanation, Session,
};
use std::path::Path;

/// The fixed advisory note attached to every emitted core (ADR-0031 D3): MUS
/// extraction returns *a* minimal explanation, not *the* canonical one.
const UNSAT_CORE_NOTE: &str = "one minimal explanation; other minimal cores may exist";

/// Explain why setting `path` to `value` would be rejected against the snapshot.
///
/// The single CLI handler for the runtime `explain-rejection` command. Builds an
/// ephemeral `Session<CuddBackend>` from the snapshot's `ccm_ref`, replays the
/// committed `choices`, and asks the solver to explain the candidate
/// `(param_key, value)` selection. See the module header for the full outcome
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

    // {parameter} -> {facet}: the candidate facet is the `param_key` of a
    // `component.<id>.param.<param_key>` path. Any other shape is a free-form
    // scalar the solver does not govern (ADR-0030 D5).
    let Some(param_key) = parse_param_key(&request.path) else {
        return unknown_facet(
            &request,
            "the path is not a 'component.<id>.param.<key>' parameter, so it \
             names no modeled facet to explain",
        );
    };

    // Re-derive the session from the snapshot (ADR-0017 §3). After a valid
    // runtime-open the `.ccm` is known usable (ADR-0030 D2); a load/construct
    // failure here on a facet-shaped write is an internal fault that FAILS
    // CLOSED (ADR-0031 D4), not a "no model" degrade.
    let ccm = match Session::<CuddBackend>::load_ccm(Path::new(snapshot.ccm_ref.trim())) {
        Ok(ccm) => ccm,
        Err(_) => return engine_divergence(&request, param_key, value_str),
    };
    let mut session = match Session::<CuddBackend>::new(ccm) {
        Ok(session) => session,
        Err(_) => return engine_divergence(&request, param_key, value_str),
    };

    // If `param_key` is not a facet in the symbol table, the write is a
    // free-form scalar — an unconstrained facet the solver does not own
    // (ADR-0030 D5). No modeled option to explain → unknown facet, no core.
    if !facet_present(&session, param_key) {
        return unknown_facet(
            &request,
            "the parameter is not a solver-modeled facet, so there is no \
             constraint conflict to explain",
        );
    }

    // Replay the committed selection (ADR-0017 §3) so the explanation accounts
    // for prior choices. Skip the very facet under explanation (the candidate
    // supersedes any prior pin on it) and any choice the BDD does not model.
    // Re-deriving an already-accepted state must never spuriously fault, so a
    // replay error is ignored here — the explain call below is the authority on
    // the candidate decision.
    for (facet, option) in &snapshot.choices {
        if facet == param_key {
            continue;
        }
        let _ = session.apply(facet, option);
    }

    // Ask the solver to explain the candidate. The solver decides; this wrapper
    // only composes the envelope (ADR-0017 amendment: solver DECIDES, the
    // compose layer COMPOSES).
    match session.explain_rejection(param_key, value_str) {
        // Genuine constraint conflict with a labeled MUS → success, carry the
        // converted core (ADR-0031 D2/D3).
        Ok(RejectionExplanation {
            would_reject: true,
            core: Some(core),
        }) => explain_conflict(&request, param_key, value_str, convert_core(core)),
        // `would_reject` without a core would be a solver contract violation
        // (a genuine reject must carry its MUS, ADR-0031 D3). Treat the missing
        // core as a fail-closed fault rather than emitting a coreless conflict.
        Ok(RejectionExplanation {
            would_reject: true,
            core: None,
        }) => engine_divergence(&request, param_key, value_str),
        // Genuinely valid option: nothing to explain (ADR-0030 D5) → success,
        // no core.
        Ok(RejectionExplanation {
            would_reject: false,
            ..
        }) => explain_not_rejected(&request, param_key, value_str),
        // Division-of-labor unknown cases (ADR-0030 D5): the same typed errors
        // `apply` raises. Exit 2, no core (acceptance criterion #2).
        Err(solver::Error::UnknownFacet(_)) => unknown_facet(
            &request,
            "the facet is not present in the compiled model",
        ),
        Err(solver::Error::UnknownOption { .. }) => invalid_option(&request, param_key, value_str),
        // MUS fault, unmappable variable, or Backend/Invariant/Ccm fault →
        // FAIL CLOSED (ADR-0031 D4). Never a partial core.
        Err(_) => engine_divergence(&request, param_key, value_str),
    }
}

/// Convert the solver-owned `LabeledCore` (labeled `{facet}.{value}` strings)
/// into the compiler-side `UnsatCore` envelope (ADR-0031 D3). This is the
/// ADR-0003 §2 boundary crossing — the only place a solver decision type is
/// translated into a compiler contract type. Labeled names only ever flow
/// through; no BDD/batsat index can appear because `LabeledCore` carries none.
fn convert_core(core: LabeledCore) -> UnsatCore {
    UnsatCore {
        rejected: convert_atom(&core.rejected),
        conflicting_constraints: core
            .conflicting_constraints
            .iter()
            .map(convert_constraint)
            .collect(),
        minimal: core.minimal,
        note: UNSAT_CORE_NOTE.to_string(),
    }
}

/// Map one solver `LabeledAtom` onto a compiler `ConstraintFacet` — a pure
/// `{facet, value}` -> `{facet, option}` rename (the solver's `value` is the
/// compiler's `option`).
fn convert_atom(atom: &LabeledAtom) -> ConstraintFacet {
    ConstraintFacet {
        facet: atom.facet.clone(),
        option: atom.value.clone(),
    }
}

/// Map one solver `LabeledConstraint` onto a compiler `ConflictingConstraint`,
/// translating the `CoreConstraintKind` onto the compiler `ConstraintKind` and
/// synthesizing the advisory `summary` gloss from the labeled atoms (ADR-0031
/// D3 — `summary` is advisory text, not a parsed field).
fn convert_constraint(constraint: &LabeledConstraint) -> ConflictingConstraint {
    let kind = match constraint.kind {
        CoreConstraintKind::Selection => ConstraintKind::Selection,
        CoreConstraintKind::ModelRule => ConstraintKind::ModelRule,
    };
    let facets: Vec<ConstraintFacet> = constraint.atoms.iter().map(convert_atom).collect();
    ConflictingConstraint {
        kind,
        summary: constraint_summary(kind, &facets),
        facets,
    }
}

/// A short, advisory one-line gloss for a conflicting constraint (ADR-0031 D3).
/// Derived purely from the labeled atoms; the human renderer (configflux-9d28)
/// is the richer presentation layer over the JSON — this is only the embedded
/// gloss so the machine envelope is self-describing without it.
fn constraint_summary(kind: ConstraintKind, facets: &[ConstraintFacet]) -> String {
    let atoms: Vec<String> = facets
        .iter()
        .map(|f| format!("{}.{}", f.facet, f.option))
        .collect();
    match kind {
        ConstraintKind::Selection => {
            format!("conflicts with your earlier choice {}", atoms.join(", "))
        }
        ConstraintKind::ModelRule => {
            format!("blocked by model rule over {}", atoms.join(", "))
        }
    }
}

/// Whether the loaded CCM's symbol table contains a facet named `facet` (some
/// `{facet}.{value}` symbol is present). Mirrors the prefix convention
/// `Session::valid_options` uses and the identical helper in
/// `solver_validation.rs`.
fn facet_present(session: &Session<CuddBackend>, facet: &str) -> bool {
    let Some(symbols) = session.ccm().symbols() else {
        return false;
    };
    let prefix = format!("{facet}.");
    symbols.variable_order().any(|sym| sym.starts_with(&prefix))
}

/// Extract the `param_key` from a `component.<id>.param.<param_key>` path.
/// Returns `None` for any other shape (those are not runtime parameter writes
/// the solver governs). Identical to the `solver_validation.rs` parser so the
/// `explain-rejection` and `set-parameter` surfaces agree on what a facet write
/// is.
fn parse_param_key(path: &str) -> Option<&str> {
    let mut parts = path.split('.');
    if parts.next()? != "component" {
        return None;
    }
    let _component_id = parts.next()?;
    if parts.next()? != "param" {
        return None;
    }
    let param_key = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(param_key)
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
    //! Unit coverage for the solver-owned `LabeledCore` -> compiler-side
    //! `UnsatCore` conversion (the ADR-0003 §2 boundary crossing this module
    //! owns). These are pure-function tests with NO solver/BDD dependency, so
    //! they pin the {facet, value} -> {facet, option} mapping, the kind
    //! translation, the labeled-name invariant, the advisory note, and the
    //! `blocking_choices` derivation independently of the live MUS extraction.
    //! The end-to-end path through a real `.ccm` is covered by the runtime
    //! integration tests `run_046`/`run_047`.

    use super::*;

    fn atom(facet: &str, value: &str) -> LabeledAtom {
        LabeledAtom {
            facet: facet.to_string(),
            value: value.to_string(),
        }
    }

    /// A `LabeledCore` carrying one prior-selection constraint and one
    /// multi-atom model rule converts to the ADR-0031 D3 `UnsatCore` shape:
    /// `{facet, value}` becomes `{facet, option}`, the kinds map across, the
    /// fixed note is attached, and `minimal` is carried through.
    #[test]
    fn convert_core_maps_facets_kinds_and_note() {
        let core = LabeledCore {
            rejected: atom("database", "postgres"),
            conflicting_constraints: vec![
                LabeledConstraint {
                    kind: CoreConstraintKind::Selection,
                    atoms: vec![atom("storage", "local")],
                },
                LabeledConstraint {
                    kind: CoreConstraintKind::ModelRule,
                    atoms: vec![atom("database", "postgres"), atom("storage", "remote")],
                },
            ],
            minimal: true,
        };

        let converted = convert_core(core);

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

        let model_rule = &converted.conflicting_constraints[1];
        assert_eq!(model_rule.kind, ConstraintKind::ModelRule);
        assert_eq!(model_rule.facets.len(), 2);
        assert!(model_rule.summary.contains("model rule"));
    }

    /// Every emitted atom is a labeled `{facet}.{option}` name — never a bare
    /// integer. Guards the configflux-osp labeled-MUS invariant at the
    /// conversion boundary (the solver type already carries labels; the
    /// converter must not lose them).
    #[test]
    fn convert_core_emits_only_labeled_names() {
        let core = LabeledCore {
            rejected: atom("engine", "v8"),
            conflicting_constraints: vec![LabeledConstraint {
                kind: CoreConstraintKind::ModelRule,
                atoms: vec![atom("engine", "v6"), atom("engine", "v8")],
            }],
            minimal: true,
        };
        let converted = convert_core(core);
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
                },
                ConflictingConstraint {
                    kind: ConstraintKind::ModelRule,
                    facets: vec![ConstraintFacet {
                        facet: "network".to_string(),
                        option: "offline".to_string(),
                    }],
                    summary: String::new(),
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
