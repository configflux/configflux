// SPDX-License-Identifier: BUSL-1.1
//
// The one-shot `cfx explain` verb (ADR-0042 §1, plan-launch-readiness-v2 §3):
// given the same inputs as `cfx resolve`, locate the choice that makes the
// selection unsatisfiable and print WHY — the minimal conflicting-constraint set
// (ADR-0031's labeled unsat core) as human text (default) or as the existing
// explain JSON envelope UNMODIFIED (`--format json`).
//
// How the "why" is obtained (reuse, not reimplement — ADR-0042 §2):
//   * `open -> init` and the file+flag choice merge come from the shared
//     `pipeline::open_and_init` prologue.
//   * the choices are then applied ONE AT A TIME with the SOLVER-authoritative
//     `session_compose::apply` — the exact seam the interpreter `select` command
//     routes through. The FIRST solver-rejected choice is the one to explain,
//     against the (satisfiable) state accumulated from the prior choices. Using
//     the solver apply, not the compiler's legacy narrowing, is what makes cfx
//     find the same conflict the interpreter would (ADR-0030): a cross-facet
//     exclusion the compiler under-models is still caught here.
//   * the labeled minimal core comes from `session_compose::explain`, which
//     sources it from `solver::Session::explain_rejection` (ADR-0031 D3). Its
//     `ExplainRejectionResult` is emitted BYTE-FOR-BYTE for `--format json`, so
//     `cfx explain --format json` is byte-identical to the interpreter `explain`
//     envelope for the same request.
//
// `cfx` reaches the solver ONLY through `session_compose` — no direct `//solver`
// edge (ADR-0003 §2 amendment). `session_compose` requires a usable `.ccm`
// (ADR-0030 hard precondition) and fails closed with a stable diagnostic when
// none is reachable; that surfaces here as a status-Error explanation → exit 2.

use std::collections::BTreeSet;
use std::path::Path;

use compiler::loader_api::{
    resolve_from_selection, ApplySelectionRequest, ExplainRejectionRequest, ExplainRejectionResult,
    ResolveFromSelectionRequest, ResolveResult, SelectionDelta,
};
use compiler::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};

use crate::pipeline::{self, OpenedModel, PipelineError, SelectPair};

/// The outcome of `cfx explain`.
pub enum ExplainOutcome {
    /// Every choice applied cleanly under the solver AND the resulting selection
    /// resolves — the selection is satisfiable, so there is nothing to explain
    /// (exit 3, ADR-0042 §3).
    Satisfiable,
    /// A choice was rejected; `result` is the solver-authoritative explanation.
    /// On a genuine conflict it carries a populated `unsat_core` with `status:
    /// ok` (exit 0, ADR-0031 D2/D3); when the explain operation could not run at
    /// all (no usable `.ccm`, solver fault) it is `status: error` (exit 2).
    Explained(ExplainRejectionResult),
    /// The solver's per-choice apply gate accepted every choice, but the
    /// selection is unsatisfiable in the RESOLVE context: an active model
    /// condition references a tag that no choice or context tag binds. This is
    /// the exact verdict `cfx resolve` reaches (the E_RESOLVE_CONTEXT_UNSATISFIED
    /// family); explain must AGREE and explain it — name the unbound tag(s) —
    /// rather than report "satisfiable; nothing to explain" (configflux-sc69).
    ResolveContextUnsatisfied(ResolveContextConflict),
}

/// Why a selection the solver apply gate accepts is still unsatisfiable at
/// resolve time (configflux-sc69). Carries the resolve failure envelope verbatim
/// — emitted UNMODIFIED for `--format json` (an existing schema, ADR-0042 §3) —
/// plus the tag names parsed out of the failing condition for the human text.
pub struct ResolveContextConflict {
    /// The resolve failure envelope, emitted UNMODIFIED for `--format json`.
    pub resolve_result: ResolveResult,
    /// Tag identifiers the failing condition references that no current choice or
    /// context tag binds (parsed from the diagnostic; may be empty when the
    /// condition cannot be recovered from the message — the caller then shows the
    /// raw diagnostic, which still carries the condition text).
    pub unbound_tags: Vec<String>,
}

/// Compose `open -> init -> (solver apply)* -> explain` for `cfx explain`.
/// `selects` are the parsed `--select` pairs merged on top of the optional
/// selection file (file first, flags override — ADR-0042 §2). The merged choice
/// map is walked in deterministic (`BTreeMap`-sorted) order.
pub fn run(
    model: &Path,
    selection_file: Option<&Path>,
    selects: &[SelectPair],
) -> Result<ExplainOutcome, PipelineError> {
    let OpenedModel {
        handle,
        scope,
        context_tags,
        choices,
        base_state,
    } = pipeline::open_and_init(model, selection_file, selects)?;

    // Apply each choice with the solver-authoritative `session_compose::apply`
    // (the interpreter `select` seam). The first REJECT is the choice to
    // explain; the state accumulated before it is the satisfiable base.
    let mut state = base_state;
    for (facet, option) in &choices {
        let applied = session_compose::apply(ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: scope.clone(),
            selection_state: state.clone(),
            selection_delta: SelectionDelta {
                facet: facet.clone(),
                option: option.clone(),
            },
        });
        match applied.status {
            OperationStatus::Ok => {
                state = applied.selection_state.ok_or_else(|| {
                    PipelineError::usage("select accepted but returned no selection state")
                })?;
            }
            OperationStatus::Error => {
                // This choice is unsatisfiable under `state`. Explain it: the
                // decision and the labeled minimal core are the solver's,
                // carried through `session_compose::explain` (ADR-0031 D3). The
                // resulting envelope is byte-identical to the interpreter's for
                // the same request.
                let result = session_compose::explain(ExplainRejectionRequest {
                    schema_version: PRODUCT_SCHEMA_VERSION,
                    model_handle: handle.clone(),
                    scope: scope.clone(),
                    selection_state: state,
                    rejected_option: SelectionDelta {
                        facet: facet.clone(),
                        option: option.clone(),
                    },
                });
                return Ok(ExplainOutcome::Explained(result));
            }
        }
    }

    // The solver apply gate accepted every choice — but that gate decides only
    // boolean facet satisfiability. It does NOT run the resolve-context
    // condition evaluation `cfx resolve` performs, where an active condition
    // that references an unbound tag makes the selection unsatisfiable. Run that
    // SAME check now — the exact `resolve_from_selection` call the resolve
    // pipeline composes — so explain reaches resolve's verdict instead of
    // falsely reporting the selection satisfiable (configflux-2awb.6 / sc69).
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope,
        selection_state: state,
    });
    if resolved.status == OperationStatus::Error {
        // Classify with the SAME family `cfx resolve` uses (`pipeline::classify`):
        // the unsatisfiable family becomes an explanation, so any code resolve
        // treats as unsat — E_RESOLVE_CONTEXT_UNSATISFIED today, and whatever a
        // later ADR adds to that family — flows through here automatically. Any
        // other failure (loader/model fault) is an operation error the caller
        // surfaces as-is (exit 2).
        let classified = pipeline::classify(&resolved.diagnostics);
        if classified.unsatisfiable {
            let bound: BTreeSet<&str> = choices
                .keys()
                .chain(context_tags.keys())
                .map(String::as_str)
                .collect();
            let message = resolved
                .diagnostics
                .diagnostics
                .first()
                .map(|d| d.message.as_str())
                .unwrap_or("");
            let unbound_tags = unbound_tags(message, &bound);
            return Ok(ExplainOutcome::ResolveContextUnsatisfied(
                ResolveContextConflict {
                    resolve_result: resolved,
                    unbound_tags,
                },
            ));
        }
        return Err(classified);
    }

    // Every choice applied AND the selection resolves — it is satisfiable.
    Ok(ExplainOutcome::Satisfiable)
}

/// Parse the tag identifiers a resolve-context diagnostic blames, keeping only
/// the ones nothing binds. The `E_RESOLVE_CONTEXT_UNSATISFIED` message embeds
/// the failing condition in single quotes, e.g.
///   `Failed to evaluate condition: 'region == 'eu''`
/// The condition is the span between the first and last single quote; within it,
/// tag identifiers are `[a-z][a-z0-9_]*` runs OUTSIDE any quoted literal (the
/// ADR-0008 condition grammar — quoted spans are string values, not tags), minus
/// the `true`/`false` keywords. An identifier already in `bound` (a choice or
/// context tag) is not unbound. The result is sorted and de-duplicated. Best
/// effort: an empty result just means the caller shows the raw diagnostic, which
/// still carries the condition text.
fn unbound_tags(message: &str, bound: &BTreeSet<&str>) -> Vec<String> {
    let (Some(start), Some(end)) = (message.find('\''), message.rfind('\'')) else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    let condition = message[start + 1..end].as_bytes();
    let mut tags: BTreeSet<String> = BTreeSet::new();
    let mut i = 0;
    while i < condition.len() {
        let byte = condition[i];
        // A quoted string literal is a value, not a tag — skip it wholesale.
        if byte == b'\'' || byte == b'"' {
            i += 1;
            while i < condition.len() && condition[i] != byte {
                i += 1;
            }
            i += 1; // step past the closing quote (or off the end)
            continue;
        }
        // An identifier starts with [a-z] and continues with [a-z0-9_].
        if byte.is_ascii_lowercase() {
            let s = i;
            i += 1;
            while i < condition.len()
                && (condition[i].is_ascii_lowercase()
                    || condition[i].is_ascii_digit()
                    || condition[i] == b'_')
            {
                i += 1;
            }
            let ident = &message[start + 1 + s..start + 1 + i];
            if ident != "true" && ident != "false" && !bound.contains(ident) {
                tags.insert(ident.to_string());
            }
            continue;
        }
        i += 1;
    }
    tags.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(names: &[&str]) -> BTreeSet<&'static str> {
        // Leak is fine in a unit test; keeps the &str borrow simple.
        names.iter().map(|n| Box::leak(n.to_string().into_boxed_str()) as &str).collect()
    }

    #[test]
    fn unbound_tags_names_the_unbound_condition_tag() {
        // The exact E_RESOLVE_CONTEXT_UNSATISFIED message the S1 repro produces.
        let msg = "Failed to evaluate condition: 'region == 'eu''";
        let got = unbound_tags(msg, &bound(&["cooling_brand", "cooling_model", "pump_type"]));
        assert_eq!(got, vec!["region".to_string()]);
    }

    #[test]
    fn unbound_tags_excludes_bound_tags_and_quoted_values() {
        // `variant` is bound; only `region` (referenced, unbound) is reported.
        // The quoted `'heavy'` / `'eu'` values must never be mistaken for tags.
        let msg = "Failed to evaluate condition: 'variant == 'heavy' && region != 'eu''";
        let got = unbound_tags(msg, &bound(&["variant"]));
        assert_eq!(got, vec!["region".to_string()]);
    }

    #[test]
    fn unbound_tags_sorts_and_dedups_multiple_unbound_tags() {
        let msg = "Failed to evaluate condition: 'zone == 'a' || region == 'b' || zone == 'c''";
        let got = unbound_tags(msg, &bound(&[]));
        assert_eq!(got, vec!["region".to_string(), "zone".to_string()]);
    }

    #[test]
    fn unbound_tags_ignores_boolean_keywords() {
        let msg = "Failed to evaluate condition: 'true && region == 'eu''";
        let got = unbound_tags(msg, &bound(&[]));
        assert_eq!(got, vec!["region".to_string()]);
    }

    #[test]
    fn unbound_tags_empty_when_no_condition_span() {
        // A message with no single-quoted span yields nothing (best effort).
        let got = unbound_tags("some other resolve failure", &bound(&[]));
        assert!(got.is_empty());
    }
}
