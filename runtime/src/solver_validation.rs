// SPDX-License-Identifier: BUSL-1.1
//
// Runtime-side `solver::Session` constraint validation for `set_parameter`
// (configflux-g3f.3, per ADR-0017 §3 and §5) and the `runtime-open` `.ccm`
// precondition (configflux-dj7f, per ADR-0030 D2).
//
// # Where this lives and why
//
// The constraint-evaluation core is the `solver` crate. The compiler must
// never import `solver` (ADR-0003 §2 hard rule), so this option-validity
// check cannot live in `compiler::runtime_api`. It lives in the `runtime`
// crate, which already depends on `compiler` and now also on `solver`, and
// runs as a pre-check that the runtime CLI's `set-parameter` /
// `set-parameters-atomically` handlers apply *before* forwarding to
// `compiler::runtime_api::set_parameter` (ADR-0017 §4: this is an *added*
// validation step, not a replacement of the existing write mechanics).
//
// # CCM precondition at open time (ADR-0030 D2)
//
// `runtime-open` enforces that a usable `.ccm` is reachable for the snapshot
// (`ccm_usable_for_open`). This is NEW behavior: pre-ADR-0030, `runtime-open`
// never touched the `.ccm` — the session was built lazily inside each
// `set_parameter` from `snapshot.ccm_ref`. Hoisting the integrity check to
// open time means `validate_set_parameter` no longer needs an availability
// skip: after a valid open, an empty/unloadable `ccm_ref` cannot recur, so the
// only remaining skips are the division-of-labor cases (non-string value,
// non-`component.*.param.*` path, unconstrained facet). A solver fault on the
// modeled trial-apply now FAILS CLOSED (D4) instead of silently permitting the
// write.
//
// # Session lifecycle (ADR-0017 §3)
//
// Ephemeral, re-derived from the `RuntimeSnapshot` on every call — there is
// no resident session and no new serialization blob in the snapshot. On each
// `set_parameter`:
//
//   1. `Session::<CuddBackend>::load_ccm(snapshot.ccm_ref)` → `new(ccm)`.
//   2. Re-derive selection state by replaying `snapshot.choices` (the
//      facet→option selections the compiler lowered into BDD variables)
//      onto the fresh session via `Session::apply`.
//   3. Map the proposed write's `(component_id, param_key)` path to a
//      candidate facet via the CCM symbol table. If the path is *not* a
//      facet (a free-form scalar — the common case for runtime-writable
//      params), there is nothing for the solver to reason about and the
//      check is skipped, retaining today's behaviour exactly.
//   4. If the path *is* a facet, trial-apply the proposed value. If the
//      session becomes unsatisfiable, surface the selection-family code
//      (ADR-0017 §5) and leave the snapshot unchanged.
//
// `CuddBackend` is the concrete backend per ADR-0017 §1 (the backend the
// release gate measures, ADR-0015).

use compiler::loader_api::{
    E_SELECTION_CONFLICT, E_SELECTION_ENGINE_DIVERGENCE, E_SELECTION_INVALID_OPTION,
    E_SELECTION_UNKNOWN_FACET, E_SELECTION_UNSATISFIABLE,
};
use compiler::product_api::{Diagnostic, DiagnosticSeverity, OperationStatus};
use compiler::runtime_api::{runtime_open, RuntimeOpenRequest, RuntimeOpenResult, RuntimeSnapshot};
use compiler::schema::Value;
use solver::{CuddBackend, Session};
use std::path::Path;

/// `runtime-open` with the ADR-0030 D2 `.ccm` precondition layered on. This is
/// the single enforcement point shared by **both** runtime-open entrypoints:
/// the CLI handler (`cli_adapter::Commands::RuntimeOpen`) and the C ABI export
/// (`runtime_c_abi::configflux_runtime_session_open`). Keeping it here — beside
/// `ccm_usable_for_open` — guarantees the precondition applies uniformly no
/// matter which surface drives the open (configflux-u32v; ADR-0030 Amendment 1).
///
/// `runtime_open` itself lives in the compiler crate, which may not import
/// `solver` (ADR-0003 §2), so the usability check is layered in the runtime
/// crate — exactly as the solver option-validity pre-check wraps `set_parameter`.
///
/// New behavior since ADR-0030 (does not exist pre-ADR-0030): after the
/// compiler's open validation succeeds, the snapshot's `ccm_ref` must resolve to
/// a usable solver model (loadable artifact with a populated symbol table). When
/// it does not — empty reference, unloadable artifact, or symbol-less stub — the
/// open FAILS CLOSED with `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` and no
/// snapshot is returned. A failed compiler open (already an error) is passed
/// through untouched; the precondition is only checked on an otherwise-
/// successful open.
pub(crate) fn runtime_open_with_solver_validation(
    request: RuntimeOpenRequest,
) -> RuntimeOpenResult {
    let result = runtime_open(request);
    // Only enforce the precondition on an otherwise-successful open; a failed
    // open already carries the compiler's diagnostic and no snapshot.
    if result.status != OperationStatus::Ok {
        return result;
    }
    let Some(snapshot) = result.runtime_snapshot.as_ref() else {
        return result;
    };
    if ccm_usable_for_open(&snapshot.ccm_ref) {
        return result;
    }
    runtime_open_solver_model_unavailable(&result)
}

/// Build the fail-closed `runtime-open` error envelope (ADR-0030 D2), mirroring
/// the compiler's `runtime_open_failed` field layout. Reuses the identity
/// fields (`model_hash`/`resolve_hash`/`scope`) the successful open computed so
/// the rejection still attributes to the right model.
fn runtime_open_solver_model_unavailable(ok: &RuntimeOpenResult) -> RuntimeOpenResult {
    let diagnostics = compiler::product_api::DiagnosticsReport {
        schema_version: compiler::product_api::PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![compiler::product_api::Diagnostic {
            code: compiler::runtime_api::E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE.to_string(),
            severity: compiler::product_api::DiagnosticSeverity::Error,
            message: "runtime-open requires a usable .ccm solver model; the snapshot's \
                      ccm_ref is empty, unloadable, or carries no symbol table"
                .to_string(),
            source_id: None,
            entity_path: Some("runtime_open_request.ccm_ref".to_string()),
            hint: Some(
                "Recompile the model so a usable .ccm sibling is emitted, then re-open \
                 against the refreshed snapshot"
                    .to_string(),
            ),
        }],
        error_count: 1,
        warning_count: 0,
    };
    RuntimeOpenResult {
        schema_version: compiler::product_api::PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: ok.model_hash.clone(),
        resolve_hash: ok.resolve_hash.clone(),
        scope: ok.scope.clone(),
        runtime_snapshot: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// Whether `ccm_ref` resolves to a usable solver model: a non-empty reference
/// that loads into a `Session` with a populated symbol table. ADR-0030 D2 — the
/// open-time precondition. An empty reference, an unloadable artifact, or a
/// symbol-less stub CCM all return `false` (open fails closed). Mirrors the
/// "usable model" rule the interpreter selection path enforces
/// (`interpreter::solver_session`).
pub(crate) fn ccm_usable_for_open(ccm_ref: &str) -> bool {
    let ccm_ref = ccm_ref.trim();
    if ccm_ref.is_empty() {
        return false;
    }
    let Ok(ccm) = Session::<CuddBackend>::load_ccm(Path::new(ccm_ref)) else {
        return false;
    };
    let Ok(session) = Session::<CuddBackend>::new(ccm) else {
        return false;
    };
    session.ccm().symbols().is_some()
}

/// A solver-detected constraint violation for a proposed `set_parameter`
/// write. Carries the selection-family code (ADR-0017 §5; or the internal
/// fault code per ADR-0030 D4) and a human-readable message; the CLI layer
/// turns this into a `Diagnostic`/`OperationStatus::Error` with the snapshot
/// left unchanged.
pub(crate) struct SolverRejection {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) facet: String,
}

impl SolverRejection {
    /// Render this rejection as the runtime `Diagnostic` the CLI surfaces.
    /// `entity_path` is the write path so clients can attribute the
    /// rejection to the offending parameter.
    pub(crate) fn to_diagnostic(&self, write_path: &str) -> Diagnostic {
        Diagnostic {
            code: self.code.to_string(),
            severity: DiagnosticSeverity::Error,
            message: self.message.clone(),
            source_id: None,
            entity_path: Some(write_path.to_string()),
            hint: Some(format!(
                "Choose a value for facet '{}' consistent with the current selection",
                self.facet
            )),
        }
    }
}

/// Validate a proposed `set_parameter` write against the solver.
///
/// Returns:
///   - `None` when the write is *not* a solver selection (a non-string value,
///     a path that does not map to a facet, or a facet absent from the CCM
///     symbol table — the permanent division-of-labor cases of ADR-0030 D5),
///     in which case the runtime proceeds with its existing type/limit/
///     lifecycle validation unchanged; or when the value is a valid selection.
///   - `Some(SolverRejection)` when the path maps to a facet and the proposed
///     value is rejected by the solver — either a typed constraint violation
///     (ADR-0017 §5) or an internal solver fault, which now FAILS CLOSED
///     (ADR-0030 D4) instead of being silently skipped.
///
/// ADR-0030 D2 retires the *availability* skips here: a usable `.ccm` is now a
/// hard precondition enforced at open time (`ccm_usable_for_open`), so this
/// function no longer skips the constraint check on an empty/unloadable
/// `ccm_ref`. The only skips that remain are the division-of-labor cases (D5),
/// which narrow the set of accepted selections without ever widening rejections
/// beyond the constraint model.
pub(crate) fn validate_set_parameter(
    snapshot: &RuntimeSnapshot,
    write_path: &str,
    value: &Value,
) -> Option<SolverRejection> {
    // The solver reasons over boolean facet selections, whose values are
    // strings (`{facet}.{value}` symbols, ADR-0005 §3). A non-string value
    // can never name a facet option, so it is a scalar write → skip (D5).
    let value_str = match value {
        Value::String(s) => s.as_str(),
        _ => return None,
    };

    // The runtime path is `component.<component_id>.param.<param_key>`.
    // The `param_key` is the candidate facet (ADR-0017 §3). A non-matching
    // path is a free-form scalar write the solver does not govern → skip (D5).
    let Some(param_key) = parse_param_key(write_path) else {
        return None;
    };

    // Re-derive the session from the snapshot (ADR-0017 §3). ADR-0030 D2/D4: a
    // load or construction failure is no longer masked as "no constraint
    // model" — `runtime-open` already certified the `.ccm` is usable, so a
    // failure here on a facet-shaped write is an internal fault that FAILS
    // CLOSED rather than silently permitting the write.
    let ccm = match Session::<CuddBackend>::load_ccm(Path::new(snapshot.ccm_ref.trim())) {
        Ok(ccm) => ccm,
        Err(_) => return Some(solver_fault_rejection(param_key)),
    };
    let mut session = match Session::<CuddBackend>::new(ccm) {
        Ok(session) => session,
        Err(_) => return Some(solver_fault_rejection(param_key)),
    };

    // If the path's `param_key` is not a facet in the symbol table, the
    // write is a free-form scalar — skip (retain today's behaviour). The
    // symbol table stores `{facet}.{value}` entries, so a facet is present
    // iff some symbol is prefixed by `{param_key}.`.
    if !facet_present(&session, param_key) {
        return None;
    }

    // Re-derive the committed selection by replaying the snapshot's
    // `choices` (facet→option). Selections that do not correspond to a
    // symbol in this CCM are ignored (consistent with the resolve adapter,
    // ADR-0017 §3): the snapshot may carry context/selection keys the BDD
    // does not model.
    for (facet, option) in &snapshot.choices {
        // Skip re-applying the very facet we are about to test; the
        // proposed value supersedes any prior selection on it.
        if facet == param_key {
            continue;
        }
        let _ = session.apply(facet, option);
    }

    // Trial-apply the proposed value and map the outcome onto the
    // selection-family codes (ADR-0017 §5). On success, also confirm the
    // facet still has at least one valid option (defence in depth against
    // a globally-unsatisfiable model).
    match session.apply(param_key, value_str) {
        Ok(()) => {
            match session.valid_options(param_key) {
                Ok(opts) if opts.count == 0 => Some(SolverRejection {
                    code: E_SELECTION_UNSATISFIABLE,
                    message: format!(
                        "Setting '{param_key}' to '{value_str}' leaves the model unsatisfiable"
                    ),
                    facet: param_key.to_string(),
                }),
                _ => None,
            }
        }
        Err(solver::Error::Conflict { facet, value }) => Some(SolverRejection {
            code: E_SELECTION_CONFLICT,
            message: format!(
                "Selection '{facet}={value}' conflicts with the current configuration"
            ),
            facet,
        }),
        Err(solver::Error::UnknownOption { facet, value }) => Some(SolverRejection {
            code: E_SELECTION_INVALID_OPTION,
            message: format!("'{value}' is not a known option for facet '{facet}'"),
            facet,
        }),
        Err(solver::Error::UnknownFacet(facet)) => Some(SolverRejection {
            code: E_SELECTION_UNKNOWN_FACET,
            message: format!("Facet '{facet}' is not present in the model"),
            facet,
        }),
        // Backend / invariant / ccm fault on a solver-owned modeled write.
        // ADR-0030 D4 (replaces ADR-0017 §5's "backend faults defer to the
        // compiler"): a fault that silently permits the write is
        // indistinguishable from rot, so it FAILS CLOSED.
        Err(_) => Some(solver_fault_rejection(param_key)),
    }
}

/// Build the fail-closed rejection for an internal solver fault on a modeled
/// `set_parameter` write (ADR-0030 D4). Carries the selection surface's
/// internal-fault code so a client can distinguish a fault from a typed
/// constraint violation.
fn solver_fault_rejection(param_key: &str) -> SolverRejection {
    SolverRejection {
        code: E_SELECTION_ENGINE_DIVERGENCE,
        message: format!(
            "The solver faulted while validating the write to facet '{param_key}'"
        ),
        facet: param_key.to_string(),
    }
}

/// Extract the `param_key` from a `component.<id>.param.<param_key>` path.
/// Returns `None` for any other shape (those are not runtime parameter
/// writes the solver governs).
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

/// Whether the loaded CCM's symbol table contains a facet named `facet`
/// (i.e. some `{facet}.{value}` symbol is present). Mirrors the prefix
/// convention `Session::valid_options` uses.
fn facet_present(session: &Session<CuddBackend>, facet: &str) -> bool {
    let Some(symbols) = session.ccm().symbols() else {
        return false;
    };
    let prefix = format!("{facet}.");
    symbols.variable_order().any(|sym| sym.starts_with(&prefix))
}
