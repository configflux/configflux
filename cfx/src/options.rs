// SPDX-License-Identifier: BUSL-1.1
//
// The one-shot `cfx options` verb (ADR-0042 §1, plan-launch-readiness-v2 §4):
// list every facet and the currently-valid options after applying a (possibly
// empty) partial selection — the guided-walk primitive.
//
// It reuses the shared `pipeline::prepare` (open -> init-selection-state ->
// apply-selection*) to build the selection state, enumerates the facet universe
// with the loader-only `list_selection_facets`, then routes the per-facet
// post-selection valid set through `session_compose::options` — the
// solver-authoritative composition the interpreter's `options` envelope also
// consumes (ADR-0042 §2 amended, ADR-0003 §2 amended). This is what makes the
// listing byte-identical to the interpreter envelope path AFTER a partial
// selection: the still-valid set is the solver's answer (`Session::valid_options`,
// ADR-0017 §4 / ADR-0030), not the compiler's legacy narrowing — which for an
// override-gated facet under-reports (S1: `cooling_model` after
// `cooling_brand=hydra` is the authoritative `["a9","x200"]`, not `["x200"]`).
//
// `cfx` reaches the solver ONLY through `session_compose`; it takes no direct
// `//solver` edge (ADR-0003 §2 amendment). `session_compose` requires a usable
// `.ccm` (ADR-0030 hard precondition) and fails closed with a stable diagnostic
// when none is reachable; that error is mapped onto the exit contract below,
// exactly as any other non-OK `options` result.

use std::path::Path;

use compiler::loader_api::{
    list_selection_facets, GetSelectionOptionsRequest, GetSelectionOptionsResult,
};
use compiler::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};

use crate::pipeline::{self, PipelineError, SelectPair};

/// One facet's listing: the facet name, whether it is already fixed (a
/// `--select` choice or an immutable context-tag pin) or still open, and the
/// raw per-facet `GetSelectionOptionsResult` (emitted UNMODIFIED for
/// `--format json`, ADR-0042 §3).
pub struct FacetListing {
    pub facet: String,
    /// The chosen/pinned option, or `None` when the facet is still open.
    pub selected: Option<String>,
    pub result: GetSelectionOptionsResult,
}

/// The successful outcome of `cfx options`: one `FacetListing` per model facet,
/// in sorted facet order (the order `list_selection_facets` returns).
pub struct OptionsOutcome {
    pub facets: Vec<FacetListing>,
}

/// Compose `open -> init -> apply* -> (list facets) -> options*` for
/// `cfx options`. `selects` are the parsed `--select` pairs applied on top of
/// the optional selection file (file first, flags override — ADR-0042 §2).
pub fn run(
    model: &Path,
    selection_file: Option<&Path>,
    selects: &[SelectPair],
) -> Result<OptionsOutcome, PipelineError> {
    let prepared = pipeline::prepare(model, selection_file, selects)?;

    let facet_names = list_selection_facets(&prepared.handle)
        .map_err(|err| PipelineError::usage(format!("unable to list model facets ({err})")))?;

    let mut facets = Vec::with_capacity(facet_names.len());
    for facet in facet_names {
        let result = session_compose::options(GetSelectionOptionsRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: prepared.handle.clone(),
            scope: prepared.scope.clone(),
            selection_state: prepared.state.clone(),
            facet: facet.clone(),
            // Pruned-reason detail is a legacy-only convenience and is absent
            // from the interpreter's default `options` envelope; keep it off so
            // the emitted JSON is byte-identical to the envelope path.
            include_pruned_reasons: false,
        });
        // The facet came from the model's own facet universe, so `options` is
        // expected to succeed; a non-OK result is a loader/integrity fault or a
        // fail-closed solver-model-unavailable condition (ADR-0030), not a
        // selection conflict. `classify` maps it onto the exit contract.
        if result.status != OperationStatus::Ok {
            return Err(pipeline::classify(&result.diagnostics));
        }
        // A facet is "selected" when it carries an explicit choice, and pinned
        // (also shown as selected) when an immutable context tag fixes it.
        let selected = prepared
            .choices
            .get(&facet)
            .or_else(|| prepared.context_tags.get(&facet))
            .cloned();
        facets.push(FacetListing {
            facet,
            selected,
            result,
        });
    }

    Ok(OptionsOutcome { facets })
}
