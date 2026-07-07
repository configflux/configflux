// SPDX-License-Identifier: BUSL-1.1

//! Per-stage wall-clock instrumentation for the CCM emitter
//! (configflux-o89x).
//!
//! Lives next to `ccm_emitter.rs` so the build pipeline's stages and
//! their timers stay in one logical unit, while keeping the parent
//! file under its source-line budget. See `ccm_emitter.rs` for the
//! `_with_timings` public entry points that consume this module.
//!
//! The contract is binary: when a caller passes
//! `Option<&mut StageTimings>::None` into the inner build functions,
//! `stage()` returns straight to the body — no `Instant::now()`
//! call is issued. This is what keeps `build_ccm_artifact` and
//! `emit_ccm_dir*` zero-cost when no profiling is requested. The
//! brief for configflux-o89x and the architect constraint from
//! configflux-c0vf both require this.
//!
//! # Where the data goes
//!
//! Stage data is stderr/log only. It is NEVER serialised into
//! `ccm.manifest.json`, `ccm.symbols.json`, or `ccm.bdd.bin` —
//! ADR-0005 §6 byte-stability stays intact. The
//! `tools/gen_synthetic --profile` flag is the documented consumer.

use anyhow::Result;
use std::time::{Duration, Instant};

/// Per-stage wall-clock breakdown of a CCM build.
///
/// Populated only by the `_with_timings` entry points exported from
/// `ccm_emitter.rs`. The `disk_write` field is meaningful only on
/// `emit_ccm_dir_with_timings` — the in-memory build path leaves it
/// at `Duration::ZERO`. All other stages are measured on both
/// in-memory and dir-write paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageTimings {
    pub clause_parse: Duration,
    pub var_order: Duration,
    pub bdd_apply_loop: Duration,
    pub manifest_serialize: Duration,
    pub disk_write: Duration,
}

impl StageTimings {
    /// Sum of every stage in this breakdown. Useful for the
    /// "% of total" column in the breakdown table.
    pub fn total(&self) -> Duration {
        self.clause_parse
            + self.var_order
            + self.bdd_apply_loop
            + self.manifest_serialize
            + self.disk_write
    }
}

/// Run `body`. If `t` is `Some`, accumulate body wall-time into the
/// field selected by `pick`. When `None`, no `Instant::now()` is
/// invoked — the legacy entry points stay zero-cost.
#[inline]
pub(super) fn stage<T, F, P>(
    t: &mut Option<&mut StageTimings>,
    pick: P,
    body: F,
) -> Result<T>
where
    F: FnOnce() -> Result<T>,
    P: FnOnce(&mut StageTimings) -> &mut Duration,
{
    if let Some(s) = t.as_deref_mut() {
        let start = Instant::now();
        let out = body()?;
        *pick(s) += start.elapsed();
        Ok(out)
    } else {
        body()
    }
}
