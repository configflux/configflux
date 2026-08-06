// SPDX-License-Identifier: BUSL-1.1

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct TempDirGuard {
    pub(crate) path: PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

/// Per-process monotonic discriminator for scenario temp-dir names.
///
/// This is the uniqueness primitive: `fetch_add` hands out a value at most once
/// per process, so two names built from it can never be equal.
static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a temp-dir path that cannot collide with any other path this process
/// hands out. Does not create the directory — see [`unique_temp_dir`].
///
/// Determinism (configflux-q5rr): the old name was
/// `<prefix>-<label>-<pid>-<nanos>`. `<pid>` is constant inside one test binary
/// and the Rust harness runs `#[test]` functions on parallel threads, so the
/// name was unique only when the caller happened to pass a distinct `<label>`
/// *or* the clock happened to tick between the two reads. Two baseline tests
/// sharing one helper (and therefore one label) got the same directory and
/// emitted into each other's IR — one saw `chunk_count` 3 instead of 2, the
/// other lost a chunk mid-read. The atomic `seq` removes the race by
/// construction; `nanos` is retained only as a triage aid in the path name and
/// no longer carries the uniqueness guarantee, so a degenerate clock degrades
/// readability rather than correctness.
///
/// This mirrors the fix already landed for the same flake class in
/// `runtime/src/tests.rs` (configflux-6gzn).
pub(crate) fn unique_temp_path(prefix: &str, label: &str) -> PathBuf {
    let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "{}-{}-{}-{}-{}",
        prefix,
        label,
        std::process::id(),
        seq,
        nanos
    ))
}

/// Create a collision-proof temp dir and return a guard that removes it on drop.
///
/// The leaf is created with `create_dir` rather than `create_dir_all` so the
/// no-collision invariant is *enforced*, not merely reasoned about: if a name
/// were ever handed out twice the second create would fail loudly instead of
/// silently sharing a directory with another test.
pub(crate) fn unique_temp_dir(prefix: &str, label: &str) -> Result<TempDirGuard> {
    let path = unique_temp_path(prefix, label);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create temp dir parent '{}'", parent.display()))?;
    }
    std::fs::create_dir(&path)
        .with_context(|| format!("Failed to create temp dir '{}'", path.display()))?;
    Ok(TempDirGuard { path })
}

#[cfg(test)]
mod tests {
    use super::{unique_temp_dir, unique_temp_path};
    use std::collections::BTreeSet;

    /// Two calls with an identical prefix *and* label must not share a
    /// directory. Deterministic: the assertion holds off the atomic counter, so
    /// it does not depend on the wall clock ticking between the two calls —
    /// which is exactly the assumption that made configflux-q5rr flaky.
    #[test]
    fn unique_temp_path_differs_for_identical_prefix_and_label() {
        let first = unique_temp_path("cfx-q5rr", "same-label");
        let second = unique_temp_path("cfx-q5rr", "same-label");
        assert_ne!(first, second, "temp paths collided: {}", first.display());
    }

    /// The configflux-q5rr shape: several harness threads asking for the same
    /// prefix and label concurrently must each get their own directory.
    #[test]
    fn unique_temp_dir_differs_across_concurrent_threads() {
        const THREADS: usize = 16;

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                std::thread::spawn(|| {
                    unique_temp_dir("cfx-q5rr-parallel", "same-label").expect("create temp dir")
                })
            })
            .collect();
        let guards: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("temp dir thread panicked"))
            .collect();

        let distinct: BTreeSet<_> = guards.iter().map(|guard| guard.path.clone()).collect();
        assert_eq!(
            distinct.len(),
            THREADS,
            "temp dirs collided across threads: {distinct:?}"
        );
        for guard in &guards {
            assert!(guard.path.is_dir(), "missing dir '{}'", guard.path.display());
        }
    }
}
