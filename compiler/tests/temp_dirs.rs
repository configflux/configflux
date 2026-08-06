// SPDX-License-Identifier: BUSL-1.1
//
// Collision-proof temp-dir naming for the `compiler/tests/` integration
// crates. configflux-rvpb.
//
// The in-crate unit tests get this from `compiler::scenario_test_support`,
// but that module is `#[cfg(test)]` and `pub(crate)`, so the integration
// tests — each its own crate — cannot see it. This file is the same
// primitive, `#[path]`-included per crate the way `solver/tests/fixture_v2.rs`
// already is; Bazel wires it into each test target's `srcs`.
//
// This file is NOT a test by itself. The determinism assertions for the
// mechanism live with the original in `compiler/src/scenario_test_support.rs`.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-process monotonic discriminator for temp-dir names.
///
/// This is the uniqueness primitive: `fetch_add` hands out a value at most
/// once per process, so two names built from it can never be equal. Each
/// including crate gets its own copy of this static, which is fine — the
/// crates are separate test binaries, and `pid` separates those.
static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Create a temp dir that cannot collide with any other path this process
/// hands out, and return it.
///
/// Determinism (configflux-q5rr, configflux-rvpb): the old name was
/// `<prefix>-<label>-<pid>-<nanos>`. `<pid>` is constant inside one test
/// binary and the Rust harness runs `#[test]` functions on parallel threads,
/// so the name was unique only when the caller happened to pass a distinct
/// `<label>` *or* the clock happened to tick between the two reads. The
/// atomic `seq` removes the race by construction; `nanos` is retained only as
/// a triage aid in the path name and no longer carries the uniqueness
/// guarantee, so a degenerate clock degrades readability rather than
/// correctness.
///
/// The leaf is created with `create_dir` rather than `create_dir_all` so the
/// no-collision invariant is *enforced*, not merely reasoned about: if a name
/// were ever handed out twice the second create would fail loudly instead of
/// silently sharing a directory with another test. For the same reason the
/// callers' old `remove_dir_all` pre-wipe is gone — it was the collision
/// *amplifier*, deleting a concurrent test's tree rather than reporting the
/// clash.
pub fn unique_temp_dir(prefix: &str, label: &str) -> PathBuf {
    let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!(
        "{}-{}-{}-{}-{}",
        prefix,
        label,
        std::process::id(),
        seq,
        nanos
    ));
    if let Some(parent) = base.parent() {
        std::fs::create_dir_all(parent).expect("create temp dir parent");
    }
    std::fs::create_dir(&base).expect("mkdir tempdir");
    base
}
