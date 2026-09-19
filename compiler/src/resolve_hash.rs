// SPDX-License-Identifier: BUSL-1.1

//! The ONE `resolve_hash` recipe: its pre-image, its field order, its
//! skip-if-empty rule, and the hash over the bytes those produce.
//!
//! `resolve_hash` is computed on two paths. The loader emits it when a resolve
//! succeeds (`loader_api::resolve_from_selection`), and the runtime recomputes
//! it when a caller bridges that resolve into a session
//! (`runtime_api::runtime_open`), rejecting the open with
//! `E_RUNTIME_HASH_MISMATCH` when the two disagree.
//!
//! Until configflux-y2ai those two paths each carried their OWN copy of the
//! pre-image struct and the hashing. The copies agreed, and a comment on each
//! told the next editor to keep them in lockstep — which is a convention, not a
//! mechanism. A single edit to one copy would have produced
//! `E_RUNTIME_HASH_MISMATCH` on every selection-bearing open in the product,
//! wearing the shape of a caller mistake while being drift between two
//! implementations of the same recipe. This module is that mechanism: there is
//! one pre-image, so there is nothing left to keep in lockstep.
//!
//! ## What the cross-validation proves, and what it never proved
//!
//! ADR-0047 §5 and ADR-0057 §D6 describe the runtime side as an INDEPENDENT
//! reimplementation, and the older comments framed the `runtime_open` check as
//! two recipes agreeing. Read literally that framing was always weaker than it
//! sounded: the recipe is keyless and public, so agreement between two copies of
//! it was never evidence of authenticity, and the copies were written by
//! transcription rather than derived separately. What the check actually
//! establishes — and still establishes, unchanged, over one recipe — is that the
//! caller forwarded the SAME six inputs the loader hashed. A mismatch is proof
//! that an input moved between resolve and open (configflux-9991 closed on
//! exactly that reading). Sharing the recipe leaves that property intact and
//! removes the failure mode where the two sides disagree about the recipe
//! itself.
//!
//! ## Layering
//!
//! This module is a LEAF. It imports nothing from `loader_api` or `runtime_api`,
//! and both import it, so consolidation added no dependency direction that did
//! not already exist. Keep it that way: a `use crate::loader_api::…` here would
//! make the runtime's dependency on the recipe a dependency on the loader.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::ir::sha256_hex;
use crate::product_api::PRODUCT_SCHEMA_VERSION;

/// The selection-state pre-image: the nested `selection_state` object inside
/// [`ResolveHashCanonical`], and the WHOLE pre-image of `selection_state_hash`
/// (`loader_api::compute_selection_state_hash`).
///
/// One struct serving both is deliberate. `selection_state_hash` identifies the
/// user's selection, and `resolve_hash` folds that same identity in as a nested
/// object; two structs would let the nested object drift away from the thing it
/// claims to be a copy of.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SelectionStateCanonical<'a> {
    pub(crate) schema_version: u32,
    pub(crate) model_hash: &'a str,
    pub(crate) scope: &'a str,
    pub(crate) context_tags: &'a BTreeMap<String, String>,
    pub(crate) choices: &'a BTreeMap<String, String>,
}

/// The `resolve_hash` pre-image. FIELD ORDER IS THE CONTRACT — serde emits the
/// fields in declaration order, the bytes below are hashed as written, and every
/// pinned `resolve_hash` in the scenario goldens, the byte-stability baselines
/// and the shipped examples was produced by this exact order. Reordering a field
/// rotates every one of them.
///
/// `resolved_output` must ALREADY be canonicalized by the caller (recursively
/// key-sorted objects). The loader holds the canonical form because it just
/// built it; the runtime receives the payload raw off the wire and canonicalizes
/// it on the way in. Both canonicalizers are idempotent and byte-identical, so
/// the value reaching this struct is the same on both paths.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResolveHashCanonical<'a> {
    pub(crate) schema_version: u32,
    pub(crate) model_hash: &'a str,
    pub(crate) scope: &'a str,
    pub(crate) selection_state: SelectionStateCanonical<'a>,
    pub(crate) resolved_output: &'a serde_json::Value,
    // ADR-0047 §5: the auto-bound-default provenance is folded in with the SAME
    // skip-if-empty rule the field carries on `ResolveResult`. Appended LAST and
    // omitted when empty, so a facet-free model's pre-image bytes are unchanged
    // by that feature. `SelectionState` (pure user input) is deliberately
    // untouched — the default is a resolve-time act, recorded here, not a
    // mutation of the user's selection.
    #[serde(skip_serializing_if = "ref_btreemap_is_empty")]
    pub(crate) defaulted_choices: &'a BTreeMap<String, String>,
    // ADR-0057 §D6: folded with the SAME skip-if-empty rule, appended LAST after
    // `defaulted_choices`. Skip-if-empty is what keeps every model where nothing
    // was defaulted or implied byte-identical to its pre-feature pre-image, so
    // dropping the attribute rotates hashes the goldens pin.
    #[serde(skip_serializing_if = "ref_btreemap_is_empty")]
    pub(crate) implied_choices: &'a BTreeMap<String, String>,
}

/// The selection-provenance fields folded into [`ResolveHashCanonical`], in
/// pre-image order — the ONE list a remediation hint may name.
///
/// It exists because the runtime's `E_RUNTIME_HASH_MISMATCH` hint used to spell
/// its own list inline and drifted from the pre-image the moment ADR-0047 §5
/// added `defaulted_choices` (configflux-j2jj: the hint kept naming only
/// `context_tags` and `choices`, so it pointed users away from the one field
/// they had actually dropped). A hint built from this const cannot drift: the
/// next field added to the pre-image is added here, and every message that names
/// the set updates with it.
///
/// It lives beside the struct rather than inside `loader_api` because the hint
/// that consumes it is emitted by `runtime_api`; the const belongs to the
/// pre-image, not to either caller. `loader_api` re-exports it, so the public
/// path `compiler::loader_api::RESOLVE_HASH_SELECTION_FIELDS` is unchanged.
pub const RESOLVE_HASH_SELECTION_FIELDS: &str =
    "context_tags, choices, defaulted_choices, implied_choices";

/// `skip_serializing_if` predicate for a borrowed `&BTreeMap` field: serde hands
/// the closure `&(&BTreeMap)`, so the double reference auto-derefs to the map's
/// own `is_empty`. Keeps the resolve-hash pre-image byte-identical for models
/// with no defaulted or implied bindings (ADR-0047 §5 skip-if-empty invariant).
fn ref_btreemap_is_empty(map: &&BTreeMap<String, String>) -> bool {
    map.is_empty()
}

/// Hash the resolve-hash pre-image over its six inputs.
///
/// This is the whole recipe. Both the loader's emission and the runtime's
/// recomputation reach it; neither reimplements any part of it. The outer
/// `schema_version` is the product's, not a caller input, so it is stamped here
/// rather than passed — a caller cannot get it wrong.
///
/// `selection_state` is taken as the already-built pre-image object because the
/// two callers source its fields differently and both spellings are load-bearing.
/// The loader passes the caller's `SelectionState` verbatim, including its own
/// `schema_version`/`model_hash`/`scope`; the runtime has no `SelectionState` and
/// substitutes the product schema version and the request's outer
/// `model_hash`/`scope`. Those agree because `validate_selection_state` refuses a
/// resolve whose inner state disagrees with the outer handle and scope, which is
/// precisely why the runtime may substitute them.
///
/// `resolved_output` must already be canonical — see [`ResolveHashCanonical`].
///
/// `Err` only where the pre-image cannot be serialized, which for a struct of
/// strings, maps and an already-decoded `Value` is unreachable in practice; it is
/// propagated rather than unwrapped because both callers own a diagnostic
/// envelope for it.
pub(crate) fn compute_resolve_hash(
    model_hash: &str,
    scope: &str,
    selection_state: SelectionStateCanonical<'_>,
    resolved_output: &serde_json::Value,
    defaulted_choices: &BTreeMap<String, String>,
    implied_choices: &BTreeMap<String, String>,
) -> Result<String> {
    let canonical = ResolveHashCanonical {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash,
        scope,
        selection_state,
        resolved_output,
        defaulted_choices,
        implied_choices,
    };

    let bytes =
        serde_json::to_vec(&canonical).context("Failed to canonicalize resolve hash payload")?;
    Ok(sha256_hex(&bytes))
}
