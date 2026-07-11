// SPDX-License-Identifier: BUSL-1.1

mod conditions;
pub mod ccm_emitter;
mod compiler_core;
// configflux-wbzw: CUDD-side BDD construction path. Single-importer
// module per ADR-0011 + Amendment 1 §A1.3 — the only file under
// `compiler/` permitted to `use cudd_sys::*`. Crate-internal: the
// public surface is via `ccm_emitter::build_ccm_artifact_with_construction`
// (opt-in `--construction=cudd`); compiler-external callers do not see
// any CUDD types.
//
// configflux-4jmc: gated behind the default-off `cudd` feature so the lean
// default build compiles neither this module nor its `cudd_sys` import (and
// thus never links the CUDD C backend). `mod checkpoint;` and `#[cfg(test)]
// mod tests;` are declared INSIDE `cudd_build/mod.rs`, so they gate out with
// the parent — no dangling crate-root module decls.
#[cfg(feature = "cudd")]
mod cudd_build;
mod ingest_merge;
mod link_verify;
pub mod ir;
pub mod loader_api;
pub mod prelude;
// configflux-9pjy.3 / ADR-0039 §7: process RSS lookup, lifted out of the
// `cudd`-gated `cudd_build/checkpoint.rs` so the compile-time progress
// signal can sample RSS on the lean (CUDD-free) default build too. Imports
// no `cudd_sys` — the single-CUDD-importer rule (ADR-0011 Amendment 1
// §A1.3) is preserved.
mod proc_rss;
// configflux-9pjy.3 / ADR-0039 §7 + ADR-0005 Amendment 2: the compile-time
// progress signal (weighted phase model + `ProgressSink` callback). A
// SEPARATE stream — it never enters the byte-stable artifact; the default
// `NullSink` keeps the unbudgeted compile byte-identical.
pub mod progress;
pub mod product_api;
// configflux-pq2w.1 / ADR-0044 D1: tool-identity provenance sidecar written
// next to the CMP/CCM artifacts. Deterministic + non-hashed — never enters a
// hash preimage; wall-clock is opt-in via `--stamp-time`.
pub mod provenance_sidecar;
pub mod resolved_models;
pub mod resolver;
// configflux-9pjy.2 / ADR-0039: soft product-compile resource budget +
// pure derivation. The budget surfaces on `product_api::CompileModelRequest`
// and (via the CLI) on `--max-rss-mb` / `--max-threads`; `derive_knobs`
// maps it to the internal memo-cap / cluster-size / threads knobs.
pub mod resource_budget;
pub mod runtime_api;
pub mod sync_transport;
pub mod telemetry_sink;
#[cfg(test)]
mod lib_tests;
#[cfg(test)]
mod scenario_test_support;
#[cfg(test)]
mod scenario_loop0_tests;
#[cfg(test)]
mod scenario_loop10_tests;
#[cfg(test)]
mod scenario_loop3_tests;
#[cfg(test)]
mod scenario_loop4_tests;
#[cfg(test)]
mod scenario_loop5_tests;
#[cfg(test)]
mod scenario_loop6_tests;
#[cfg(test)]
mod scenario_loop7_tests;
#[cfg(test)]
mod scenario_loop9_tests;
#[cfg(test)]
mod scenario_resource_budget_tests;
#[cfg(test)]
mod scenario_byte_stability_tests;
#[cfg(test)]
mod scenario_diamond_tests;
pub mod schema;

pub use compiler_core::{verify_ir_dir, Compiler};

// configflux-p2sz.4 / ADR-0034 D3: the model scrubber (`tools/model_scrubber`)
// reuses the in-crate condition grammar to pseudonymize identifiers embedded in
// `condition` strings via parse → substitute → re-serialize (a regex rename is
// prohibited). The `conditions` module stays otherwise private; only these
// three pure, AST-based entry points are exported. They add no compiler→solver
// coupling (ADR-0003 §2).
pub mod condition_rewrite {
    pub use crate::conditions::{
        condition_identifiers, rewrite_condition_identifiers, ConditionIdentifiers,
    };
}
