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

use std::path::Path;

use compiler::loader_api::{
    ApplySelectionRequest, ExplainRejectionRequest, ExplainRejectionResult, SelectionDelta,
};
use compiler::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};

use crate::pipeline::{self, OpenedModel, PipelineError, SelectPair};

/// The outcome of `cfx explain`.
pub enum ExplainOutcome {
    /// Every choice applied cleanly under the solver — the selection is
    /// satisfiable, so there is nothing to explain (exit 3, ADR-0042 §3).
    Satisfiable,
    /// A choice was rejected; `result` is the solver-authoritative explanation.
    /// On a genuine conflict it carries a populated `unsat_core` with `status:
    /// ok` (exit 0, ADR-0031 D2/D3); when the explain operation could not run at
    /// all (no usable `.ccm`, solver fault) it is `status: error` (exit 2).
    Explained(ExplainRejectionResult),
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
        choices,
        base_state,
        ..
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

    // Every choice applied under the solver — the selection is satisfiable.
    Ok(ExplainOutcome::Satisfiable)
}
