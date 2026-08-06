// SPDX-License-Identifier: BUSL-1.1
//
// The `runtime-open` `.ccm` precondition (configflux-dj7f, per ADR-0030 D2) and
// the projection of a `RuntimeSnapshot` onto the facet assignment a session
// knows (configflux-jraj, per the 2026-08-03 amendment to ADR-0017 §3).
//
// # Where this lives and why
//
// The constraint-evaluation core is the `solver` crate. The compiler must never
// import `solver` (ADR-0003 §2 hard rule), so neither the open precondition nor
// the write-path constraint check can live in `compiler::runtime_api`. Both live
// in the `runtime` crate, which already depends on `compiler` and on `solver`.
//
// This module answers "what does this session know?". `write_enforcement`
// answers "what does enforcement do with that?" — it replays this assignment,
// decides satisfiability, and composes the rejection envelopes. The split keeps
// the projection callable from `explain_rejection`, which needs the same view of
// the session but none of the write machinery.
//
// # CCM precondition at open time (ADR-0030 D2)
//
// `runtime-open` enforces that a usable `.ccm` is reachable for the snapshot
// (`ccm_usable_for_open`). This is NEW behavior: pre-ADR-0030, `runtime-open`
// never touched the `.ccm` — the session was built lazily inside each
// `set_parameter` from `snapshot.ccm_ref`. Hoisting the integrity check to open
// time means the write path no longer needs an availability skip: after a valid
// open, an empty/unloadable `ccm_ref` cannot recur, so the only remaining skips
// are the division-of-labor cases (non-string value, non-`component.*.param.*`
// path, unconstrained facet).
//
// # Session lifecycle (ADR-0017 §3)
//
// Ephemeral, re-derived from the `RuntimeSnapshot` on every call — there is no
// resident session and no new serialization blob in the snapshot. The original
// wiring re-derived it from `snapshot.choices` alone, which is the defect the
// amendment corrects: `choices` is populated once at open and never touched by a
// write, so a sibling written DURING the session was invisible to the next
// write's check. `session_assignment` below is the corrected projection.
//
// `CuddBackend` is the concrete backend per ADR-0017 §1 (the backend the
// release gate measures, ADR-0015).

use compiler::product_api::OperationStatus;
use compiler::runtime_api::{runtime_open, RuntimeOpenRequest, RuntimeOpenResult, RuntimeSnapshot};
use compiler::schema::Value;
use solver::{CuddBackend, Session};
use std::collections::{BTreeMap, BTreeSet};
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

/// The session's total known assignment (ADR-0017 amendment D2): a
/// `facet -> option` map built from the snapshot by the precedence
///
///   1. `dirty_overlay` — uncommitted in-session writes
///   2. `committed_overlay` — committed in-session writes
///   3. `choices` — the facet selections the session was opened with
///   4. the baseline `resolved_output` value of the parameter bound to the facet
///
/// Tiers 1, 2 and 4 mirror the precedence `effective_parameter_value`
/// (`compiler/src/runtime_api/shared_ops.rs`) already uses for a parameter's
/// effective value. That function is a private `fn` `include!`d into
/// `compiler::runtime_api` and is not callable from here — and the runtime needs
/// a *facet-keyed* projection rather than a per-path lookup, so the precedence
/// is reimplemented rather than exported. The two must agree on the ORDER; that
/// agreement is the invariant, not a shared code path.
///
/// `choices` outranks the baseline because the two can disagree: an explicit
/// selection is a commitment, a baseline value is only what resolution left
/// behind when nothing overrode it. The baseline belongs at the bottom because
/// dropping it would treat an already-conflicting resolved value as unset and
/// accept a write that makes the live configuration violate the model — and on
/// an all-defaults open, where `choices` is empty, it is the only source of the
/// sibling values a cross-facet constraint is evaluated against.
///
/// Facets absent from the `.ccm` symbol table are dropped, as they are today.
pub(crate) fn session_assignment(
    session: &Session<CuddBackend>,
    snapshot: &RuntimeSnapshot,
) -> BTreeMap<String, String> {
    let tiers = [
        overlay_tier(&snapshot.dirty_overlay),
        overlay_tier(&snapshot.committed_overlay),
        Tier::from_pairs(
            snapshot
                .choices
                .iter()
                .map(|(facet, option)| (facet.clone(), option.clone())),
        ),
        baseline_tier(snapshot),
    ];

    // A facet whose contributing paths disagree is omitted from the assignment
    // ENTIRELY — treated as unset and therefore free (D3) — so an ambiguous
    // facet can never produce a spurious rejection and map-iteration order can
    // never decide whether a write is accepted.
    let ambiguous: BTreeSet<&String> = tiers
        .iter()
        .flat_map(|tier| tier.ambiguous.iter())
        .collect();

    let mut assignment = BTreeMap::new();
    for tier in &tiers {
        for (facet, option) in &tier.agreed {
            if ambiguous.contains(facet) || assignment.contains_key(facet) {
                continue;
            }
            if !facet_present(session, facet) {
                continue;
            }
            assignment.insert(facet.clone(), option.clone());
        }
    }
    assignment
}

/// One precedence tier's facet candidates, split into the facets whose
/// contributing paths agree on a value and the facets whose paths disagree.
#[derive(Default)]
struct Tier {
    agreed: BTreeMap<String, String>,
    ambiguous: BTreeSet<String>,
}

impl Tier {
    fn from_pairs(pairs: impl Iterator<Item = (String, String)>) -> Self {
        let mut tier = Self::default();
        for (facet, option) in pairs {
            if tier.ambiguous.contains(&facet) {
                continue;
            }
            match tier.agreed.get(&facet) {
                Some(existing) if existing == &option => {}
                Some(_) => {
                    tier.agreed.remove(&facet);
                    tier.ambiguous.insert(facet);
                }
                None => {
                    tier.agreed.insert(facet, option);
                }
            }
        }
        tier
    }
}

/// Project an overlay (`scope_root -> canonical_path -> value`) onto facet
/// candidates. Only a `Value::String` at a `component.<id>.param.<key>` path can
/// name a `{facet}.{value}` symbol; everything else is a scalar the solver does
/// not model.
fn overlay_tier(overlay: &BTreeMap<String, BTreeMap<String, Value>>) -> Tier {
    Tier::from_pairs(overlay.values().flatten().filter_map(|(path, value)| {
        let Value::String(option) = value else {
            return None;
        };
        parse_param_key(path).map(|facet| (facet.to_string(), option.clone()))
    }))
}

/// Project the baseline resolved output onto facet candidates — D2's tier 4.
/// The keys differ from the overlays': `resolved_output` is
/// `scope_root -> components -> params`, keyed by bare param name, where an
/// overlay is keyed by the full dotted path.
fn baseline_tier(snapshot: &RuntimeSnapshot) -> Tier {
    Tier::from_pairs(
        snapshot
            .resolved_output
            .values()
            .flat_map(|scope| scope.components.values())
            .flat_map(|component| component.params.iter())
            .filter_map(|(param_key, parameter)| match &parameter.value {
                Value::String(option) => Some((param_key.clone(), option.clone())),
                _ => None,
            }),
    )
}

/// Extract the `param_key` from a `component.<id>.param.<param_key>` path.
/// Returns `None` for any other shape (those are not runtime parameter writes
/// the solver governs). The ONE definition: `explain_rejection.rs` carried a
/// verbatim copy until configflux-jraj lifted it here, so the `set-parameter`
/// and `explain-rejection` surfaces cannot drift on what a facet write is.
///
/// The mapping is many-to-one — two paths in different scope roots can land on
/// one facet. `session_assignment` CONTAINS that with D2's skip-on-disagreement
/// rule rather than resolving it; a declared parameter→facet binding is tracked
/// separately as configflux-tcp5.
pub(crate) fn parse_param_key(path: &str) -> Option<&str> {
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
/// convention `Session::valid_options` uses. Lifted alongside `parse_param_key`
/// (configflux-jraj) so both surfaces share one definition.
pub(crate) fn facet_present(session: &Session<CuddBackend>, facet: &str) -> bool {
    let Some(symbols) = session.ccm().symbols() else {
        return false;
    };
    let prefix = format!("{facet}.");
    symbols.variable_order().any(|sym| sym.starts_with(&prefix))
}
