// SPDX-License-Identifier: BUSL-1.1

//! Process resident-set-size lookup — configflux-9pjy.3, per ADR-0039 §7.
//!
//! A single, non-feature-gated helper that reads the current process's
//! `VmRSS` from `/proc/self/status`. It was lifted out of
//! `cudd_build/checkpoint.rs` (which is `cudd`-feature-gated) so the
//! compile-time progress signal (`progress.rs`) can sample RSS on the
//! lean, CUDD-free default build as well as the CUDD path.
//!
//! This module imports **no** `cudd_sys` — the single-CUDD-importer rule
//! (ADR-0011 + Amendment 1 §A1.3, only `cudd_build/mod.rs` may import
//! `cudd_sys`) is preserved: reading `/proc` is pure `std::fs` and is
//! unrelated to CUDD. The CUDD checkpoint formatter now calls this util
//! across the single-importer boundary instead of carrying its own copy.

/// Read `VmRSS:` from `/proc/self/status` in KiB. Returns `None` on
/// non-Linux (no `/proc/self/status`) or on a malformed status file; the
/// caller decides how to render the absent reading (the CUDD checkpoint
/// line prints `rss_mb=n/a`, the progress signal emits `rss_mb: None`).
pub(crate) fn read_vm_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let value = rest.split_whitespace().next()?;
            return value.parse::<u64>().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: on Linux the reader returns a positive value; on a
    /// platform without `/proc/self/status` it returns `None`. Both
    /// outcomes are acceptable — the helper must never panic and must
    /// never return `Some(0)` for a live process on Linux.
    #[test]
    fn read_vm_rss_kib_smoke() {
        match read_vm_rss_kib() {
            Some(kib) => assert!(kib > 0, "a live process must have a positive VmRSS"),
            None => {}
        }
    }
}
