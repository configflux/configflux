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
// # Which model that CCM is (configflux-nnwa)
//
// Loadable is not the same as correct. The open also binds the artifact to the
// snapshot: the `.ccm`'s `bound_model_hash` must equal the snapshot's
// `model_hash`, or the open fails closed. Without that, a snapshot's lineage
// fields could claim model A while the session decided every write against
// model B — a correctness gap, not a diagnostic one, because the write path
// accepts and rejects against whatever `ccm_ref` happens to point at.
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
// # Which parameter is a facet's handle (ADR-0064 D5)
//
// The projection reads the DECLARED binding the compiler put on the resolved
// parameter (`ResolvedParameter.facet`), reached through `bound_facet` below.
// It used to guess the facet from the path's last segment, a many-to-one
// mapping nothing in the model declared; ADR-0064 replaces the guess with a
// declaration and retires the skip-on-disagreement rule that contained it.
//
// `CuddBackend` is the concrete backend per ADR-0017 §1 (the backend the
// release gate measures, ADR-0015).

use compiler::product_api::OperationStatus;
use compiler::runtime_api::{runtime_open, RuntimeOpenRequest, RuntimeOpenResult, RuntimeSnapshot};
use compiler::schema::Value;
use solver::{Ccm, CuddBackend, Session};
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
///
/// A SECOND precondition is the lineage bind described in the module header
/// (configflux-nnwa): the loaded `.ccm`'s `bound_model_hash` must equal the
/// snapshot's `model_hash`, or the open fails closed. It reuses
/// `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` — to this caller the model they
/// asked for is still not available, and a second code would split one "the
/// `.ccm` is not the one this session needs" class in two.
///
/// Since ADR-0060 D6 a THIRD precondition is layered on, in the same place and
/// for the same reason: if the request supplied a `closed_facet_domains` table,
/// every `{facet}.{value}` pair in it must appear in the loaded `.ccm` symbol
/// table, or the open fails closed with
/// `E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN` and no snapshot. An ABSENT table stays
/// valid and degrades to asserted-only attribution (D7) — failing an open on a
/// missing DIAGNOSTIC input would break every existing integrator for a
/// message-quality feature.
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
    if !ccm_usable_for_open(&snapshot.ccm_ref) {
        return runtime_open_solver_model_unavailable(&result);
    }
    // configflux-nnwa: loaded ONCE here and threaded into the D6 check below,
    // which used to read the same directory a second time. The check above
    // just proved this load succeeds, so the `Err` arm fails closed rather
    // than unwrapping: a race that swapped the artifact between the two reads
    // is exactly the case an open must not paper over.
    let Ok(ccm) = Session::<CuddBackend>::load_ccm(Path::new(snapshot.ccm_ref.trim())) else {
        return runtime_open_solver_model_unavailable(&result);
    };
    if !ccm_bound_to_model(&ccm, &snapshot.model_hash) {
        return runtime_open_solver_model_mismatched(&result);
    }
    // ADR-0060 D6: a SUPPLIED closed-facet domain table is checked against the
    // `.ccm` the session will actually decide against. Every other field on the
    // open contract already fails closed on corruption — `ccm_ref` through the
    // precondition above, `choices`/`defaulted_choices` through the
    // `resolve_hash` recompute — so an unvalidated table would be the first
    // field whose corruption degraded behaviour SILENTLY. Anchoring the check
    // to the loaded artifact rather than to `model_hash` is deliberate: it is
    // the model that produces the rejection the table will attribute.
    if let Some(unknown) = unknown_facet_domain_symbol(snapshot, &ccm) {
        return runtime_open_facet_domain_unknown(&result, &unknown);
    }
    result
}

/// Whether the loaded `.ccm` is bound to the model the snapshot claims
/// (configflux-nnwa): the artifact's `bound_model_hash`, which the loader
/// already decoded to 32 bytes, against `model_hash` decoded from the snapshot.
/// A `model_hash` that is not 64-char lowercase hex names no compiled model at
/// all, so it is not bound to this one — the compiler's open validation rejects
/// that shape first, and this does not assume it always will.
fn ccm_bound_to_model(ccm: &Ccm, model_hash: &str) -> bool {
    let Some(claimed) = decode_sha256_hex(model_hash) else {
        return false;
    };
    claimed == ccm.bound_model_hash()
}

/// Decode a 64-character lowercase-hex SHA-256 into its 32 bytes, or `None` for
/// any other spelling. Mirrors the solver loader's own decode — uppercase is
/// refused, not folded — so both sides of the comparison agree on what a digest
/// is; the solver's copy is crate-private, which is why this is not a call.
fn decode_sha256_hex(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 32 * 2 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        let high = hex_digit_lower(bytes[index * 2])?;
        let low = hex_digit_lower(bytes[index * 2 + 1])?;
        *slot = (high << 4) | low;
    }
    Some(out)
}

/// Decode one lowercase hex digit. `None` for uppercase `A-F`, non-hex ASCII,
/// and any non-ASCII byte.
fn hex_digit_lower(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// The first `{facet}.{value}` pair in the snapshot's closed-facet domain table
/// that the bound `.ccm` symbol table does not carry, or `None` when every pair
/// is accounted for (ADR-0060 D6).
///
/// This verifies each supplied facet and value EXISTS in the bound model. It
/// cannot verify the facet is CLOSED: `Symbols` carries the roster but no
/// cardinality (putting closed-ness there would change `.ccm` emission and
/// rotate `ccm_hash` — the ratified hard stop). A caller can therefore still
/// supply an open facet's genuine roster and obtain a completion the declaring
/// model would not sanction. That residual is bounded by construction —
/// attribution consults domains only for facets a core clause already mentions,
/// and completes only when exactly one declared value is un-negated — and it is
/// the honest limit of what a rosterless artifact can check.
///
/// An ABSENT table is not checked and not an error: it degrades to
/// asserted-only attribution (D7). Only a table the model cannot account for is.
///
/// `ccm` is the artifact the caller already loaded for the lineage bind
/// (configflux-nnwa); this used to re-read it from `ccm_ref`, so one open read
/// the same directory twice and could, in principle, check the two
/// preconditions against two different artifacts.
fn unknown_facet_domain_symbol(snapshot: &RuntimeSnapshot, ccm: &Ccm) -> Option<String> {
    if snapshot.closed_facet_domains.is_empty() {
        return None;
    }
    let symbols = ccm.symbols()?;
    // `variable_order` is walked once into a set: a domain table has tens to low
    // hundreds of pairs and the symbol table is the same order, so the pairwise
    // scan `facet_present` uses would be quadratic for no reason here.
    let known: BTreeSet<&str> = symbols.variable_order().collect();
    for (facet, values) in snapshot.closed_facet_domains.iter() {
        for value in values {
            let symbol = format!("{facet}.{value}");
            if !known.contains(symbol.as_str()) {
                return Some(symbol);
            }
        }
    }
    None
}

/// Build the fail-closed `runtime-open` error envelope for ADR-0060 D6,
/// mirroring the D2 builder's field layout. `symbol` names the offending
/// `{facet}.{value}` — a model-declaration identifier, never a parameter value,
/// so this carries no request payload content.
fn runtime_open_facet_domain_unknown(ok: &RuntimeOpenResult, symbol: &str) -> RuntimeOpenResult {
    let diagnostics = compiler::product_api::DiagnosticsReport {
        schema_version: compiler::product_api::PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![compiler::product_api::Diagnostic {
            code: compiler::runtime_api::E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN.to_string(),
            severity: compiler::product_api::DiagnosticSeverity::Error,
            message: format!(
                "runtime_open_request.closed_facet_domains names '{symbol}', which the                  solver model bound to this session does not carry"
            ),
            source_id: None,
            entity_path: Some("runtime_open_request.closed_facet_domains".to_string()),
            hint: Some(
                "Project closed_facet_domains from the same resolve result the rest of                  the open request came from, and open against the solver model that                  resolve was compiled with"
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

/// The `.ccm` named by `ccm_ref` loads, but is bound to a different model than
/// the snapshot's `model_hash` names (configflux-nnwa). The message names the
/// CLASS and echoes NEITHER hash: both are caller-held values already in the
/// request, so repeating them back tells an operator nothing they do not have.
fn runtime_open_solver_model_mismatched(ok: &RuntimeOpenResult) -> RuntimeOpenResult {
    runtime_open_solver_model_error(
        ok,
        "runtime-open requires the .ccm solver model the snapshot was compiled \
         against; the .ccm named by ccm_ref is bound to a different model than \
         the snapshot's model_hash",
        "Open against the .ccm emitted beside the compiled package the snapshot's \
         model_hash names, or re-resolve and re-open against the model this .ccm \
         belongs to",
    )
}

/// Build the fail-closed `runtime-open` error envelope (ADR-0030 D2), mirroring
/// the compiler's `runtime_open_failed` field layout. Reuses the identity
/// fields (`model_hash`/`resolve_hash`/`scope`) the successful open computed so
/// the rejection still attributes to the right model.
fn runtime_open_solver_model_unavailable(ok: &RuntimeOpenResult) -> RuntimeOpenResult {
    runtime_open_solver_model_error(
        ok,
        "runtime-open requires a usable .ccm solver model; the snapshot's ccm_ref \
         is empty, unloadable, or carries no symbol table",
        "Recompile the model so a usable .ccm sibling is emitted, then re-open \
         against the refreshed snapshot",
    )
}

/// The one envelope both `.ccm` refusals return, so the two cannot drift on
/// anything but their wording: same code, same `entity_path` (`ccm_ref` is the
/// field the operator changes either way), same field layout.
fn runtime_open_solver_model_error(
    ok: &RuntimeOpenResult,
    message: &str,
    hint: &str,
) -> RuntimeOpenResult {
    let diagnostics = compiler::product_api::DiagnosticsReport {
        schema_version: compiler::product_api::PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![compiler::product_api::Diagnostic {
            code: compiler::runtime_api::E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE.to_string(),
            severity: compiler::product_api::DiagnosticSeverity::Error,
            message: message.to_string(),
            source_id: None,
            entity_path: Some("runtime_open_request.ccm_ref".to_string()),
            hint: Some(hint.to_string()),
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

/// The session's total known assignment (ADR-0017 amendment D2, with D2's
/// facet<->parameter bridge replaced by ADR-0064 D5): a `facet -> option` map
/// built from the snapshot by the precedence
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
/// Every tier reads the DECLARED binding (ADR-0064 D1): a parameter is a
/// facet's handle only when the model says `facet: <name>` on it. A parameter
/// that merely shares a facet's name contributes nothing here, and a write to
/// it is never constraint-checked.
///
/// Facets absent from the `.ccm` symbol table are dropped, as they are today.
///
/// Fails with `Err` when one tier holds two bound paths for a single facet
/// carrying DIFFERENT values. ADR-0064 D2.4 makes that a compile-time refusal,
/// so no snapshot that passed the open-time `resolve_hash` check can carry it;
/// observed anyway, the caller fails CLOSED (D5.2) instead of silently omitting
/// the facet — which is exactly what the retired skip-on-disagreement rule did.
pub(crate) fn session_assignment(
    session: &Session<CuddBackend>,
    snapshot: &RuntimeSnapshot,
) -> Result<BTreeMap<String, String>, DivergentBinding> {
    let tiers = [
        overlay_tier(snapshot, &snapshot.dirty_overlay)?,
        overlay_tier(snapshot, &snapshot.committed_overlay)?,
        // `choices` is already facet-keyed and a map, so it cannot collide with
        // itself; it needs no projection.
        snapshot.choices.clone(),
        baseline_tier(snapshot)?,
    ];

    let mut assignment = BTreeMap::new();
    for tier in &tiers {
        for (facet, option) in tier {
            if assignment.contains_key(facet) {
                continue;
            }
            if !facet_present(session, facet) {
                continue;
            }
            assignment.insert(facet.clone(), option.clone());
        }
    }
    Ok(assignment)
}

/// Two parameters declaring the same facet, found holding different values
/// within one precedence tier — the state ADR-0064 D2.4 refuses at compile time
/// (at most one parameter in a model binds a given facet).
///
/// It is therefore unreachable for a snapshot that passed the open-time
/// `resolve_hash` check. Reaching it means the snapshot and the model it claims
/// to come from disagree, which is an engine divergence (ADR-0030 D4) and not a
/// policy question — so both the write and the explain surface fail CLOSED on
/// it.
#[derive(Debug)]
pub(crate) struct DivergentBinding {
    pub(crate) facet: String,
    pub(crate) first: String,
    pub(crate) second: String,
}

impl DivergentBinding {
    /// The one wording both fail-closed envelopes carry, so the write path and
    /// `explain-rejection` report one divergence one way.
    pub(crate) fn message(&self) -> String {
        format!(
            "Facet '{}' is declared by two parameters holding different values \
             ('{}' and '{}'), which no compiled model can produce",
            self.facet, self.first, self.second
        )
    }
}

/// Fold one precedence tier's `(facet, option)` candidates into a map, failing
/// closed on the collision D2.4 makes impossible. Two candidates that AGREE are
/// not a collision: the same facet can legitimately be reached twice within a
/// tier as long as both say the same thing.
fn tier_from_pairs(
    pairs: impl Iterator<Item = (String, String)>,
) -> Result<BTreeMap<String, String>, DivergentBinding> {
    let mut tier: BTreeMap<String, String> = BTreeMap::new();
    for (facet, option) in pairs {
        if let Some(existing) = tier.get(&facet) {
            if existing != &option {
                return Err(DivergentBinding {
                    first: existing.clone(),
                    second: option,
                    facet,
                });
            }
            continue;
        }
        tier.insert(facet, option);
    }
    Ok(tier)
}

/// Project an overlay (`scope_root -> path -> value`) onto facet candidates.
/// Only a `Value::String` written to a path whose parameter DECLARES a facet
/// binding names a `{facet}.{value}` symbol; everything else is a scalar the
/// solver does not model.
fn overlay_tier(
    snapshot: &RuntimeSnapshot,
    overlay: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<BTreeMap<String, String>, DivergentBinding> {
    tier_from_pairs(overlay.values().flatten().filter_map(|(path, value)| {
        let Value::String(option) = value else {
            return None;
        };
        bound_facet(snapshot, path).map(|facet| (facet, option.clone()))
    }))
}

/// Project the baseline resolved output onto facet candidates — D2's tier 4,
/// keyed by the parameter's DECLARED facet rather than by its name (ADR-0064
/// D5.2). A parameter that declares no binding contributes nothing, so a scalar
/// that happens to share a facet's name can no longer stand in for that facet's
/// value.
fn baseline_tier(snapshot: &RuntimeSnapshot) -> Result<BTreeMap<String, String>, DivergentBinding> {
    tier_from_pairs(
        snapshot
            .resolved_output
            .values()
            .flat_map(|scope| scope.components.values())
            .flat_map(|component| component.params.values())
            .filter_map(|parameter| match (&parameter.facet, &parameter.value) {
                (Some(facet), Value::String(option)) => Some((facet.clone(), option.clone())),
                _ => None,
            }),
    )
}

/// The facet a write path is the DECLARED handle for (ADR-0064 D1/D5.1), or
/// `None` when the parameter declares no binding — in which case the path is
/// not a facet write at all: it is never constraint-checked, and its
/// type/limit/lifecycle path is unchanged.
///
/// The ONE definition, shared by the overlay projection, the write path and
/// `explain-rejection`, so those surfaces cannot drift on what a facet write
/// is. It replaces `parse_param_key`, which named the facet by the path's last
/// segment: a many-to-one guess nothing in the model declared, under which two
/// paths could land on one facet and disagree, and the facet was then dropped
/// from the assignment entirely (configflux-jraj's skip rule, retired here).
///
/// The grammar is `component.<id>.param.<key>` exactly as before; the
/// scope-qualified `<scope_root>/…` spelling is stripped upstream, so the
/// overlay keys this reads are already bare paths. The owning scope root is
/// located the way `find_parameter_scope_root`
/// (`compiler/src/runtime_api/shared_ops.rs`) locates it for a write — the one
/// scope root of `resolved_output` whose component carries the key — and, like
/// the precedence above, that rule is mirrored rather than shared, because the
/// function is a private `fn` `include!`d into `compiler::runtime_api`. A path
/// carried by more than one scope root is ambiguous; the compiler operation
/// refuses it with `E_RUNTIME_OPEN_INVALID` before enforcement ever runs, so
/// there is no one binding to read and this returns `None`.
pub(crate) fn bound_facet(snapshot: &RuntimeSnapshot, path: &str) -> Option<String> {
    let mut parts = path.split('.');
    if parts.next()? != "component" {
        return None;
    }
    let component_id = parts.next()?;
    if parts.next()? != "param" {
        return None;
    }
    let param_key = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let mut found = None;
    for scope in snapshot.resolved_output.values() {
        let Some(parameter) = scope
            .components
            .get(component_id)
            .and_then(|component| component.params.get(param_key))
        else {
            continue;
        };
        if found.is_some() {
            return None;
        }
        found = Some(parameter);
    }
    found?.facet.clone()
}

/// Whether the loaded CCM's symbol table contains a facet named `facet`
/// (i.e. some `{facet}.{value}` symbol is present). Mirrors the prefix
/// convention `Session::valid_options` uses. Lifted beside the path-to-facet
/// mapping (configflux-jraj) so both surfaces share one definition, and kept
/// by ADR-0064 D5.3 as the division-of-labour gate: a declared binding to a
/// facet the loaded `.ccm` does not carry is not enforced, exactly as before.
pub(crate) fn facet_present(session: &Session<CuddBackend>, facet: &str) -> bool {
    let Some(symbols) = session.ccm().symbols() else {
        return false;
    };
    let prefix = format!("{facet}.");
    symbols.variable_order().any(|sym| sym.starts_with(&prefix))
}
