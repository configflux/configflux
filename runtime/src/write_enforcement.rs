// SPDX-License-Identifier: BUSL-1.1
//
// Constraint enforcement for the runtime's three write commands
// (configflux-jraj, per the 2026-08-03 amendment to ADR-0017).
//
// # What this module owns, and what it does not
//
// `solver_validation` answers "what does this session know?" — it projects a
// `RuntimeSnapshot` onto a `facet -> option` assignment and owns the open-time
// `.ccm` precondition. This module answers "what does enforcement do with
// that?": it replays the assignment onto a fresh `solver::Session`, decides
// satisfiability, and composes the rejection envelope each command returns.
//
// # Why the wrappers live here rather than in `cli_adapter`
//
// Both transports must enforce identically. `runtime_c_abi` dispatched
// `SetParameter`, `SetParametersAtomically` and `CommitConfiguration` straight
// to the raw `compiler::runtime_api` entry points, so every C++/ROS2 SDK write
// ran with no solver check at all — not even the `choices`-based one the CLI
// already had. Pointing the ABI at the CLI's wrappers required them to be
// reachable from the staticlib crate root (`c_abi_lib.rs`), which deliberately
// excludes the clap-based CLI shell. Putting them here keeps the staticlib free
// of clap and gives both surfaces one enforcement point — the same unification
// configflux-u32v performed for open, extended to the write path.
//
// # The ordering that makes this correct (amendment D1)
//
// Each wrapper runs the compiler operation FIRST and returns its failures
// unchanged, so `E_RUNTIME_TYPE_MISMATCH`, `E_RUNTIME_LIMIT_VIOLATION`,
// `E_RUNTIME_LIFECYCLE_IMMUTABLE`, `E_RUNTIME_UNKNOWN_PATH` and
// `E_RUNTIME_ARTIFACT_UNKNOWN` all fire ahead of the constraint check and are
// reported as themselves. Only on success is the RESULTING snapshot tested; a
// violation discards that result entirely, so the caller's snapshot is
// unchanged and one discarded write is the whole cost of the rejection path.

use crate::solver_validation::{
    bound_facet, facet_present, session_assignment, DivergentBinding,
};
use compiler::loader_api::{
    ClosedFacetDomains, UnsatCore, CONSTRAINT_ENTITY_PATH_PREFIX, E_SELECTION_CONFLICT,
    E_SELECTION_ENGINE_DIVERGENCE, E_SELECTION_INVALID_OPTION, E_SELECTION_UNKNOWN_FACET,
    E_SELECTION_UNSATISFIABLE,
};
use compiler::product_api::{
    Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
};
use compiler::runtime_api::{
    commit_configuration, set_parameter, set_parameters_atomically, CommitConfigurationRequest,
    CommitConfigurationResult, RuntimeSnapshot, SetParameterRequest, SetParameterResult,
    SetParametersAtomicallyRequest, SetParametersAtomicallyResult,
};
use solver::{CuddBackend, Session};
use std::collections::BTreeSet;
use std::path::Path;

/// A solver-decided constraint rejection of a write (ADR-0017 amendment
/// D4/D5). Carries everything the three result envelopes need: the diagnostic,
/// the shared labeled core, and the set of facets that core names — which is
/// what `set_parameters_atomically` uses to decide which batch paths
/// participate in the violation.
pub(crate) struct ConstraintRejection {
    pub(crate) diagnostic: Diagnostic,
    pub(crate) core: Option<UnsatCore>,
    pub(crate) core_facets: BTreeSet<String>,
}

/// Enforce the model's declared constraints over the session's total known
/// assignment (ADR-0017 amendment D1–D5).
///
/// `written_paths` are the paths this operation writes — one for
/// `set_parameter`, the batch for `set_parameters_atomically`, and **none** for
/// `commit_configuration`, which changes no effective value and is checked as
/// the D6 defense-in-depth boundary. `snapshot` is the **resulting** snapshot:
/// the compiler operation has already run and succeeded, so its writes are
/// already in the overlays and the assignment below is the post-operation one.
///
/// Returns `None` when the operation is accepted or falls outside the solver's
/// remit (ADR-0030 D5), and `Some(..)` when the resulting assignment is
/// unsatisfiable — in which case the caller discards the compiler's result and
/// returns the rejection envelope, leaving the caller's snapshot untouched.
pub(crate) fn enforce_write_constraints(
    snapshot: &RuntimeSnapshot,
    written_paths: &[String],
) -> Option<ConstraintRejection> {
    // Candidate facets of this operation's own writes, path-sorted so the
    // replay order — and therefore which write a rejection is attributed to —
    // is deterministic (D3). A written path whose parameter declares no facet
    // binding is not a facet write at all (ADR-0064 D5.1): it drops out here
    // and keeps its type/limit/lifecycle path unchanged.
    let mut written: Vec<(&str, String)> = written_paths
        .iter()
        .filter_map(|path| bound_facet(snapshot, path).map(|facet| (path.as_str(), facet)))
        .collect();
    written.sort_by(|left, right| left.0.cmp(right.0));

    // Re-derive the session (ADR-0017 §3). ADR-0030 D2/D4: on a facet-shaped
    // write a load failure is an internal fault that FAILS CLOSED rather than
    // silently permitting the write. An operation with no facet-shaped path —
    // every `commit_configuration`, and a batch of pure scalars — has no
    // solver-owned write to fail closed ON, so an unusable model leaves it to
    // the compiler's own validation exactly as before.
    let fail_closed = written.first().map(|(_, facet)| facet.clone());
    let ccm = match Session::<CuddBackend>::load_ccm(Path::new(snapshot.ccm_ref.trim())) {
        Ok(ccm) => ccm,
        Err(_) => return fail_closed.map(|facet| solver_fault_rejection(&facet)),
    };
    let mut session = match Session::<CuddBackend>::new(ccm) {
        Ok(session) => session,
        Err(_) => return fail_closed.map(|facet| solver_fault_rejection(&facet)),
    };

    // Division of labor (ADR-0030 D5, restated by D3): an operation that writes
    // paths but binds no facet of this model is a free-form scalar write the
    // solver does not govern — skip entirely and keep the existing
    // type/limit/lifecycle path. Nothing here widens the set of writes the
    // solver governs.
    let operation_facets: BTreeSet<String> = written
        .iter()
        .filter(|(_, facet)| facet_present(&session, facet))
        .map(|(_, facet)| facet.clone())
        .collect();
    if !written_paths.is_empty() && operation_facets.is_empty() {
        return None;
    }

    // ADR-0064 D5.2: two parameters declaring one facet cannot both be current,
    // so a divergence here is a fault in the snapshot rather than a policy
    // question — fail CLOSED. The retired D2 rule dropped the facet instead and
    // let the write through unchecked.
    let assignment = match session_assignment(&session, snapshot) {
        Ok(assignment) => assignment,
        Err(divergence) => return Some(divergent_binding_rejection(&divergence)),
    };
    if assignment.is_empty() {
        return None;
    }

    // D3's fixed replay order: everything this operation did NOT write first,
    // in canonical order, then the operation's own facets. Applying the new
    // writes last is what makes a rejection attributable to the operation
    // rather than to pre-existing state.
    for (facet, option) in &assignment {
        if operation_facets.contains(facet) {
            continue;
        }
        match session.apply(facet, option) {
            Ok(()) => {}
            // The fact is not expressible in this model — a scalar parameter
            // that happens to share a facet's name, or a `choices` entry the
            // BDD does not model. Skip and continue (ADR-0030 D5).
            Err(solver::Error::UnknownFacet(_)) | Err(solver::Error::UnknownOption { .. }) => {}
            // The pre-existing state is already self-contradictory (D5). Abort
            // immediately: `apply` returns from its staging loop BEFORE
            // committing, so continuing would test the caller's write against a
            // session that never absorbed the conflicting fact — and could
            // accept it. The rejection is not attributable to the caller.
            Err(solver::Error::Conflict { facet, value }) => {
                return Some(pre_existing_conflict(
                    &session,
                    &facet,
                    &value,
                    written_paths,
                    &snapshot.closed_facet_domains,
                ));
            }
            Err(_) => return Some(solver_fault_rejection(facet)),
        }
    }

    for (path, facet) in &written {
        if !operation_facets.contains(facet) {
            continue;
        }
        // The assignment carries every facet this operation wrote: the write is
        // already in the dirty overlay (tier 1) and a bound parameter is always
        // `string` (ADR-0064 D2.2), which the compiler operation checked before
        // this ran. The guard covers only that unreachable residue; there is no
        // longer an "omitted because two paths disagree" case to skip.
        let Some(option) = assignment.get(facet.as_str()) else {
            continue;
        };
        match session.apply(facet, option) {
            Ok(()) => {}
            Err(solver::Error::Conflict { facet, value }) => {
                return Some(write_conflict(
                    &session,
                    &facet,
                    &value,
                    path,
                    &snapshot.closed_facet_domains,
                ));
            }
            Err(solver::Error::UnknownOption { facet, value }) => {
                return Some(typed_rejection(
                    E_SELECTION_INVALID_OPTION,
                    format!("'{value}' is not a known option for facet '{facet}'"),
                    &facet,
                    path,
                ));
            }
            Err(solver::Error::UnknownFacet(facet)) => {
                return Some(typed_rejection(
                    E_SELECTION_UNKNOWN_FACET,
                    format!("Facet '{facet}' is not present in the model"),
                    &facet,
                    path,
                ));
            }
            Err(_) => return Some(solver_fault_rejection(facet)),
        }
        // Defence in depth against a globally-unsatisfiable model: an accepted
        // apply must still leave the facet with at least one valid option.
        if let Ok(options) = session.valid_options(facet) {
            if options.count == 0 {
                return Some(typed_rejection(
                    E_SELECTION_UNSATISFIABLE,
                    format!("Setting '{facet}' to '{option}' leaves the model unsatisfiable"),
                    facet,
                    path,
                ));
            }
        }
    }

    None
}

/// The D4 rejection: the caller's write makes the resulting assignment
/// unsatisfiable. `entity_path` carries `constraints/<id>` when the core
/// attributes to a declared constraint — ADR-0054 §6's machine-consumer
/// contract, and the only way a client tells a policy violation from the other
/// `E_SELECTION_CONFLICT` causes.
fn write_conflict(
    session: &Session<CuddBackend>,
    facet: &str,
    option: &str,
    write_path: &str,
    domains: &ClosedFacetDomains,
) -> ConstraintRejection {
    let core = explain_core(session, facet, option, domains);
    let named = core.as_ref().and_then(named_constraint);
    let message = match &named {
        Some((id, summary)) => format!(
            "Writing '{write_path}' selects '{facet}={option}', which violates constraint \
             '{id}': {summary}"
        ),
        None => format!(
            "Writing '{write_path}' selects '{facet}={option}', which conflicts with the \
             current configuration"
        ),
    };
    ConstraintRejection {
        diagnostic: Diagnostic {
            code: E_SELECTION_CONFLICT.to_string(),
            severity: DiagnosticSeverity::Error,
            message,
            source_id: None,
            entity_path: Some(match &named {
                Some((id, _)) => format!("{CONSTRAINT_ENTITY_PATH_PREFIX}{id}"),
                None => write_path.to_string(),
            }),
            hint: Some(recovery_hint(facet)),
        },
        core_facets: core.as_ref().map(core_facets).unwrap_or_default(),
        core,
    }
}

/// The D5 rejection: the session's PRE-EXISTING state is already
/// self-contradictory, so the conflict belongs to a facet the caller never
/// touched. A snapshot can arrive this way — hand-assembled, or written through
/// an unchecked surface before this change. `unsat_core.rejected` names the
/// pre-existing entry, never the caller's write; that difference is exactly how
/// a consumer distinguishes this case from D4 without a new diagnostic code.
fn pre_existing_conflict(
    session: &Session<CuddBackend>,
    facet: &str,
    option: &str,
    written_paths: &[String],
    domains: &ClosedFacetDomains,
) -> ConstraintRejection {
    let core = explain_core(session, facet, option, domains);
    let named = core.as_ref().and_then(named_constraint);
    let subject = match written_paths.first() {
        Some(path) => format!("The write to '{path}'"),
        None => "The commit".to_string(),
    };
    let message = match &named {
        Some((id, summary)) => format!(
            "{subject} was not evaluated: the session already holds '{facet}={option}', which \
             violates constraint '{id}': {summary}"
        ),
        None => format!(
            "{subject} was not evaluated: the session already holds '{facet}={option}', which \
             conflicts with the model"
        ),
    };
    ConstraintRejection {
        diagnostic: Diagnostic {
            code: E_SELECTION_CONFLICT.to_string(),
            severity: DiagnosticSeverity::Error,
            message,
            source_id: None,
            entity_path: Some(match (&named, written_paths.first()) {
                (Some((id, _)), _) => format!("{CONSTRAINT_ENTITY_PATH_PREFIX}{id}"),
                (None, Some(path)) => path.clone(),
                (None, None) => format!("facet:{facet}"),
            }),
            hint: Some(recovery_hint(facet)),
        },
        core_facets: core.as_ref().map(core_facets).unwrap_or_default(),
        core,
    }
}

/// The ADR-0064 D5.2 fail-closed rejection: two parameters declaring one facet
/// were observed holding different values. D2.4 refuses that at compile time, so
/// a snapshot that passed the open-time `resolve_hash` check cannot carry it;
/// observing it anyway means the snapshot and the model disagree, which is an
/// engine divergence (ADR-0030 D4), not a policy violation. This is the exact
/// point where the retired skip rule dropped the facet and let the write
/// through.
fn divergent_binding_rejection(divergence: &DivergentBinding) -> ConstraintRejection {
    ConstraintRejection {
        diagnostic: Diagnostic {
            code: E_SELECTION_ENGINE_DIVERGENCE.to_string(),
            severity: DiagnosticSeverity::Error,
            message: divergence.message(),
            source_id: None,
            entity_path: Some(format!("facet:{}", divergence.facet)),
            hint: Some(
                "Re-resolve and re-open against the compiled model: one parameter \
                 declares each facet, so two bound values cannot both be current"
                    .to_string(),
            ),
        },
        core: None,
        core_facets: BTreeSet::new(),
    }
}

/// A typed selection-family rejection with no core: the division-of-labor cases
/// (`UnknownOption`, `UnknownFacet`) and the globally-unsatisfiable guard. These
/// keep the codes and wording they have today.
fn typed_rejection(
    code: &'static str,
    message: String,
    facet: &str,
    write_path: &str,
) -> ConstraintRejection {
    ConstraintRejection {
        diagnostic: Diagnostic {
            code: code.to_string(),
            severity: DiagnosticSeverity::Error,
            message,
            source_id: None,
            entity_path: Some(write_path.to_string()),
            hint: Some(format!(
                "Choose a value for facet '{facet}' consistent with the current selection"
            )),
        },
        core: None,
        core_facets: BTreeSet::new(),
    }
}

/// Ask the solver for the labeled MUS behind a conflict and convert it through
/// the SHARED converter (`session_compose::labeled_core_to_unsat_core`, fed the
/// roster the artifact itself carries). This is the same path `explain-rejection`
/// and `cfx` take, so the write and explain surfaces name one constraint with one
/// wording. A solver that cannot produce a core yields `None`: the rejection
/// still stands, it simply carries no structured explanation.
fn explain_core(
    session: &Session<CuddBackend>,
    facet: &str,
    option: &str,
    domains: &ClosedFacetDomains,
) -> Option<UnsatCore> {
    let roster = session.ccm().constraint_roster();
    match session.explain_rejection(facet, option) {
        Ok(solver::RejectionExplanation {
            would_reject: true,
            core: Some(core),
        // The closed-facet domains the snapshot carries — same source as
        // `explain_rejection`: the resolver computed them and the open payload
        // forwarded them (ADR-0060 D3/D4). Empty when the opener supplied none,
        // which is byte-for-byte the asserted-only attribution the write surface
        // had before (D7).
        }) => Some(session_compose::labeled_core_to_unsat_core(core, &roster, domains)),
        _ => None,
    }
}

/// The first declared constraint a core attributes to, as `(id, condition)`.
/// `None` when the core names no declared constraint — an over-constrained
/// model, where ADR-0054 §5.4 forbids handing a clause an id it did not earn.
fn named_constraint(core: &UnsatCore) -> Option<(String, String)> {
    core.conflicting_constraints.iter().find_map(|constraint| {
        constraint
            .constraint_id
            .as_ref()
            .map(|id| (id.clone(), constraint.summary.clone()))
    })
}

/// Every facet a core names, rejected candidate included. `rejected_paths` on
/// the atomic envelope is the batch paths whose facet appears here.
fn core_facets(core: &UnsatCore) -> BTreeSet<String> {
    let mut facets = BTreeSet::new();
    facets.insert(core.rejected.facet.clone());
    for constraint in &core.conflicting_constraints {
        for facet in &constraint.facets {
            facets.insert(facet.facet.clone());
        }
    }
    facets
}

/// The recovery instruction of D5. It names both escapes, because a pre-existing
/// violation rejects even the write that would repair it: `rollback_dirty` is
/// deliberately not constraint-checked, and `set_parameters_atomically` can move
/// two facets at once through a state neither could reach alone.
fn recovery_hint(facet: &str) -> String {
    format!(
        "Choose a value for facet '{facet}' consistent with the current configuration; to \
         leave a state no single write can leave, use set-parameters-atomically to move the \
         facets together, or rollback-dirty to return to the committed base"
    )
}

/// Build the fail-closed rejection for an internal solver fault on a modeled
/// write (ADR-0030 D4). Carries the selection surface's internal-fault code so a
/// client can distinguish a fault from a typed constraint violation.
fn solver_fault_rejection(param_key: &str) -> ConstraintRejection {
    ConstraintRejection {
        diagnostic: Diagnostic {
            code: E_SELECTION_ENGINE_DIVERGENCE.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "The solver faulted while validating the write to facet '{param_key}'"
            ),
            source_id: None,
            entity_path: Some(format!("facet:{param_key}")),
            hint: Some(
                "Recompile the model so a usable .ccm sibling is emitted, then re-open \
                 against the refreshed snapshot"
                    .to_string(),
            ),
        },
        core: None,
        core_facets: BTreeSet::new(),
    }
}

/// `set_parameter` with the ADR-0017-amendment constraint check layered on. The
/// compiler operation runs FIRST and its failures are returned unchanged, so
/// `E_RUNTIME_TYPE_MISMATCH`, `E_RUNTIME_LIMIT_VIOLATION`,
/// `E_RUNTIME_LIFECYCLE_IMMUTABLE`, `E_RUNTIME_UNKNOWN_PATH` and
/// `E_RUNTIME_ARTIFACT_UNKNOWN` all fire ahead of the constraint check and are
/// reported as themselves (D1). Only on success is the RESULTING snapshot tested
/// for satisfiability; a violation discards that result entirely and returns the
/// rejection envelope, so the caller's snapshot is unchanged.
///
/// This wrapper lives here rather than in `cli_adapter` because the C ABI
/// staticlib must call it too and its crate root deliberately excludes the
/// clap-based CLI shell (`c_abi_lib.rs`). One enforcement point, both transports
/// — the same unification configflux-u32v performed for open.
pub(crate) fn set_parameter_with_solver_validation(
    request: SetParameterRequest,
) -> SetParameterResult {
    let path = request.path.clone();
    let identity = SnapshotIdentity::of(&request.runtime_snapshot);
    let result = set_parameter(request);
    if result.status != OperationStatus::Ok {
        return result;
    }
    let Some(written) = result.runtime_snapshot.as_ref() else {
        return result;
    };
    let Some(rejection) = enforce_write_constraints(written, std::slice::from_ref(&path)) else {
        return result;
    };
    rejected_set_parameter(&identity, path, rejection)
}

/// `set_parameters_atomically` with the same check, applied to the batch AS A
/// WHOLE. Writes that are individually valid but jointly violate a constraint are
/// rejected together: the old per-write loop validated each against the unmodified
/// request snapshot, so a jointly-violating pair passed twice.
pub(crate) fn set_parameters_atomically_with_solver_validation(
    request: SetParametersAtomicallyRequest,
) -> SetParametersAtomicallyResult {
    let paths: Vec<String> = request.writes.iter().map(|write| write.path.clone()).collect();
    let identity = SnapshotIdentity::of(&request.runtime_snapshot);
    let result = set_parameters_atomically(request);
    if result.status != OperationStatus::Ok {
        return result;
    }
    let Some(written) = result.runtime_snapshot.as_ref() else {
        return result;
    };
    let Some(rejection) = enforce_write_constraints(written, &paths) else {
        return result;
    };
    rejected_set_parameters_atomically(&identity, written, &paths, rejection)
}

/// `commit_configuration` with the same check (D6). Promoting a dirty entry to
/// the committed overlay leaves every effective value unchanged, so the
/// assignment is invariant under commit and this can never fire for a session
/// whose writes all went through the enforced path. It is implemented anyway as
/// the defense-in-depth boundary for snapshots that did not: the runtime CLI is
/// stateless and accepts any snapshot on stdin, and commit is the operation that
/// makes state durable.
pub(crate) fn commit_configuration_with_solver_validation(
    request: CommitConfigurationRequest,
) -> CommitConfigurationResult {
    let identity = SnapshotIdentity::of(&request.runtime_snapshot);
    let result = commit_configuration(request);
    if result.status != OperationStatus::Ok {
        return result;
    }
    let Some(written) = result.runtime_snapshot.as_ref() else {
        return result;
    };
    let Some(rejection) = enforce_write_constraints(written, &[]) else {
        return result;
    };
    rejected_commit_configuration(&identity, rejection)
}

/// The three identity fields every rejection envelope echoes. Captured from the
/// request before the compiler consumes it, because the compiler operation takes
/// the snapshot by value and a rejection returns none of it back. Holding just
/// these keeps the accepted path — by far the common one — from cloning an entire
/// `RuntimeSnapshot`, which carries the whole resolved output, on every write.
struct SnapshotIdentity {
    model_hash: String,
    resolve_hash: String,
    scope: String,
}

impl SnapshotIdentity {
    fn of(snapshot: &RuntimeSnapshot) -> Self {
        Self {
            model_hash: snapshot.model_hash.clone(),
            resolve_hash: snapshot.resolve_hash.clone(),
            scope: snapshot.scope.clone(),
        }
    }
}

fn rejection_report(rejection: &ConstraintRejection) -> DiagnosticsReport {
    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![rejection.diagnostic.clone()],
        error_count: 1,
        warning_count: 0,
    }
}

fn rejected_set_parameter(
    identity: &SnapshotIdentity,
    path: String,
    rejection: ConstraintRejection,
) -> SetParameterResult {
    let diagnostics = rejection_report(&rejection);
    SetParameterResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: identity.model_hash.clone(),
        resolve_hash: identity.resolve_hash.clone(),
        scope: identity.scope.clone(),
        path,
        runtime_snapshot: None,
        parameter: None,
        unsat_core: rejection.core,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn rejected_set_parameters_atomically(
    identity: &SnapshotIdentity,
    snapshot: &RuntimeSnapshot,
    paths: &[String],
    rejection: ConstraintRejection,
) -> SetParametersAtomicallyResult {
    // D4: name the writes that PARTICIPATE in the violation — the batch paths
    // whose facet the core mentions — so a jointly-violating pair names both
    // rather than arbitrarily blaming whichever was validated first. When the
    // core names none of them (or there is no core at all), the whole batch is
    // listed; a single individually-invalid write therefore still yields just
    // that path, exactly as today.
    let mut participating: Vec<String> = paths
        .iter()
        .filter(|path| {
            bound_facet(snapshot, path)
                .is_some_and(|facet| rejection.core_facets.contains(&facet))
        })
        .cloned()
        .collect();
    if participating.is_empty() {
        participating = paths.to_vec();
    }
    participating.sort();
    participating.dedup();

    let diagnostics = rejection_report(&rejection);
    SetParametersAtomicallyResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: identity.model_hash.clone(),
        resolve_hash: identity.resolve_hash.clone(),
        scope: identity.scope.clone(),
        runtime_snapshot: None,
        applied_count: 0,
        rejected_paths: participating,
        dirty_generation_max: 0,
        unsat_core: rejection.core,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn rejected_commit_configuration(
    identity: &SnapshotIdentity,
    rejection: ConstraintRejection,
) -> CommitConfigurationResult {
    let diagnostics = rejection_report(&rejection);
    CommitConfigurationResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: identity.model_hash.clone(),
        resolve_hash: identity.resolve_hash.clone(),
        scope: identity.scope.clone(),
        runtime_snapshot: None,
        commit_id: None,
        base_configuration_id: None,
        target_configuration_id: None,
        changed_paths: Vec::new(),
        delta_manifest: None,
        unsat_core: rejection.core,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

