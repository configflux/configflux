// SPDX-License-Identifier: BUSL-1.1

mod conditions;
pub mod ccm_emitter;
// configflux-py7w: the diagnostic code a refusal carries, as data on the error.
// Crate-internal on purpose — the CODE is public (`product_api`'s `E_*`
// constants, frozen in docs/interface-contracts.md §3.4), the carrier is not.
mod coded_error;
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
// configflux-secb.4 / ADR-0057 §D9: the per-chunk interface summary every
// new link/verify check is written over. `link_verify` is a single file, so
// the summary gets its own sibling module rather than a submodule. ADR-0058's
// object header is this type serialized — which is precisely why the checks
// are written over it now rather than over the merged `Config`.
pub mod interface_summary;
// configflux-p0jz.2 / ADR-0058 §D4: the link stages, the emit they share with
// `compile`, and the object reader stage 3 uses.
mod link;
mod link_emit;
mod link_load;
// configflux-p0jz.3 / ADR-0058 §D5: the lockfile — a pin format the linker
// CHECKS and never fetches from. Public because the CLI holds `--lock-source`
// to the same unit-name rule the file's own keys are held to, and because the
// file format is a contract a reader outside this crate may want to parse.
pub mod link_lock;
mod link_verify;
pub mod ir;
pub mod loader_api;
// configflux-secb.5 / ADR-0057 §D4: the one place a `derive` table or an
// `accepts` list becomes an attributed root conjunct. Public because the three
// readers of a compiled model — the emitter, the selection loader, and the
// resolve loader — must lower identically or `options`, `explain` and `resolve`
// would disagree about what the model says.
pub mod lowering;
// configflux-p0jz.1 / ADR-0058 §D2 + §A1: the object header — one unit's
// interface, content-addressed. Public because the linker, the lockfile and
// any tool that inspects an object read this type; `object_compile` is the
// `compile-object` entry point the CLI drives.
pub mod object;
pub mod object_compile;
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
// configflux-y2ai: the ONE `resolve_hash` recipe. A LEAF module — it imports
// nothing from `loader_api` or `runtime_api`, and both import it, so the
// loader's emission and the runtime's cross-validating recomputation share one
// pre-image instead of two copies that could drift apart. Private: the only
// item that was ever public reaches callers through `loader_api`'s re-export.
mod resolve_hash;
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
// configflux-4vb0 / ADR-0056 Amendment 1: a chunk file's name is the SHA-256 of
// its own content, asserted over every scenario pack and every shipped example.
#[cfg(test)]
mod chunk_address_tests;
#[cfg(test)]
mod model_identity_tests;
// configflux-dw9i: the MAX_CHAIN_DEPTH ceiling on the authored `overrides`
// chain, entered through the three validators that walk it.
#[cfg(test)]
mod override_depth_tests;
#[cfg(test)]
mod source_digest_tests;
#[cfg(test)]
mod scenario_test_support;
#[cfg(test)]
mod scenario_baseline_tests;
// configflux-secb.6 / ADR-0057 D7: requirement delivery in the resolved
// snapshot, and the fail-closed message when the binding is undecided.
#[cfg(test)]
mod resolver_requires_tests;
#[cfg(test)]
mod scenario_byte_stability_tests;
#[cfg(test)]
mod scenario_diamond_tests;
#[cfg(test)]
mod scenario_early_binding_tests;
#[cfg(test)]
mod scenario_inspect_tests;
#[cfg(test)]
mod scenario_resolve_tests;
#[cfg(test)]
mod scenario_resource_budget_tests;
#[cfg(test)]
mod scenario_runtime_tests;
#[cfg(test)]
mod scenario_scale_tests;
#[cfg(test)]
mod scenario_selection_tests;
#[cfg(test)]
mod scenario_software_bom_tests;
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
