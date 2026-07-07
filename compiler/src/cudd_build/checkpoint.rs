// SPDX-License-Identifier: BUSL-1.1
//
// CUDD-CHECKPOINT formatting and process-RSS lookup helpers
// (configflux-8l3b). Sibling of `cudd_build/mod.rs`; split out so the
// `mod.rs` line budget (742 lines, grandfathered per ADR-0011 +
// Amendment 1 §A1.3 single-importer rule) is not blown by the periodic
// snapshot infrastructure.
//
// This file MUST NOT import `cudd_sys`. The single-importer review
// rule applies: only `cudd_build/mod.rs` may import `cudd_sys` under
// `compiler/`. This file takes CUDD inspector values (peak nodes,
// live keys, dead) by value from the caller and only handles:
//   1. Env-var gate decoding (`CONFIGFLUX_CUDD_BUILDER_PROFILE` and
//      the `_CHECKPOINT=N` interval override).
//   2. Formatting the process RSS into the `rss_mb` field (the
//      `/proc/self/status` read itself now lives in the shared,
//      CUDD-free `crate::proc_rss` util — configflux-9pjy.3 — so the
//      compile-time progress signal samples RSS the same way).
//   3. The single `eprintln!` of the `CUDD-CHECKPOINT:` line.
//
// Output is grep-only — never parsed into byte-stable artifacts
// per ADR-0005 §6.

use std::time::Instant;

/// CUDD inspector values pulled by the caller in `mod.rs` (where the
/// `cudd_sys` import lives) and handed across the single-importer
/// boundary to this file's formatter.
pub(super) struct CheckpointSnapshot {
    pub clause_count: u64,
    pub peak_node_count: core::ffi::c_long,
    pub live_keys: core::ffi::c_uint,
    pub dead: core::ffi::c_uint,
}

/// Default checkpoint interval (clauses per CUDD-CHECKPOINT line).
/// Mirrors the in-crate `BddBuilder::clear_memos` interval default
/// from configflux-d49v.
const DEFAULT_INTERVAL: u64 = 100;

/// Decide whether to emit a checkpoint on this clause boundary. The
/// gate is `CONFIGFLUX_CUDD_BUILDER_PROFILE`; the interval default is
/// 100, overrideable to any positive integer via
/// `CONFIGFLUX_CUDD_BUILDER_PROFILE_CHECKPOINT`.
fn should_emit(clause_count: u64) -> bool {
    if std::env::var_os("CONFIGFLUX_CUDD_BUILDER_PROFILE").is_none() {
        return false;
    }
    let interval: u64 = std::env::var("CONFIGFLUX_CUDD_BUILDER_PROFILE_CHECKPOINT")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &u64| *n > 0)
        .unwrap_or(DEFAULT_INTERVAL);
    clause_count % interval == 0
}

/// Format a `VmRSS` reading into the `rss_mb` field of the
/// CUDD-CHECKPOINT line. `None` → `"n/a"`; `Some(kib)` → KiB rounded
/// to MiB to one decimal place is overkill, so we use integer MiB
/// with banker's rounding.
fn rss_mb_string(kib: Option<u64>) -> String {
    match kib {
        Some(kib) => ((kib + 512) / 1024).to_string(),
        None => "n/a".to_string(),
    }
}

/// Emit a CUDD-CHECKPOINT line on stderr if the env gate is set and
/// the clause count crosses the configured interval boundary. No-op
/// otherwise. `build_start` is the `Instant` captured at the start of
/// the AND-fold loop; the formatted `wall_s` is the elapsed time
/// since then.
pub(super) fn emit_if_enabled(snapshot: CheckpointSnapshot, build_start: Instant) {
    if !should_emit(snapshot.clause_count) {
        return;
    }
    let wall = build_start.elapsed().as_secs_f64();
    let rss = rss_mb_string(crate::proc_rss::read_vm_rss_kib());
    eprintln!(
        "CUDD-CHECKPOINT: clause={cc} peak_node_count={pnc} \
         live_keys={lk} dead={dead} wall_s={wall:.3} rss_mb={rss}",
        cc = snapshot.clause_count,
        pnc = snapshot.peak_node_count,
        lk = snapshot.live_keys,
        dead = snapshot.dead,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `should_emit` is silent when the env gate is unset. This is
    /// the "telemetry-must-not-perturb-default-path" invariant.
    #[test]
    fn should_emit_is_false_when_env_unset() {
        std::env::remove_var("CONFIGFLUX_CUDD_BUILDER_PROFILE");
        assert!(!should_emit(100));
        assert!(!should_emit(0));
        assert!(!should_emit(12036));
    }

    /// `rss_mb_string` rounds correctly and falls back to `"n/a"`.
    #[test]
    fn rss_mb_string_rounds_and_falls_back() {
        assert_eq!(rss_mb_string(None), "n/a");
        // 1024 KiB = 1 MiB exactly.
        assert_eq!(rss_mb_string(Some(1024)), "1");
        // 1536 KiB = 1.5 MiB → rounds to 2 (round half up).
        assert_eq!(rss_mb_string(Some(1536)), "2");
        // 0 KiB rounds to 0 MiB.
        assert_eq!(rss_mb_string(Some(0)), "0");
    }

    // The `/proc/self/status` read smoke test moved with the lifted util
    // to `crate::proc_rss::tests::read_vm_rss_kib_smoke` (configflux-9pjy.3).
    // This file keeps only the `rss_mb` formatting test, which is its own
    // responsibility across the single-importer boundary.
}
