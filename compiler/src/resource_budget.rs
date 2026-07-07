// SPDX-License-Identifier: BUSL-1.1

//! Soft product-compile resource budget — configflux-9pjy.2, per
//! ADR-0039 §1–§4.
//!
//! A [`ResourceBudget`] is a **soft target** an operator sets to trade
//! wall-clock for a lower peak RSS. It is orthogonal to the hard guard
//! (ADR-0010/0023): it never kills and never refuses to run. When the
//! budget is unset, the compile is **byte-for-byte identical to today**
//! (ADR-0039 §8) — [`derive_knobs`] returns the existing defaults and no
//! internal knob is applied.
//!
//! [`derive_knobs`] is **pure**: it reads no environment, no `/proc`,
//! and no clock. This is load-bearing (ADR-0039 §2) — the derivation is
//! deterministic and unit-testable, and live RSS sampling is confined to
//! the later adaptive hook (ADR-0039 §5, configflux-9pjy.4), never to
//! this function.

use serde::{Deserialize, Serialize};

use crate::ccm_emitter::DEFAULT_MEMO_CAP;

/// Lower bound on the derived per-table apply-memo cap (ADR-0039 §3
/// "Risks" mitigation). A budget set too tight could otherwise drive the
/// memo cap toward zero and thrash (the memos cleared on essentially
/// every apply). The floor bounds the worst-case slowdown; the hard
/// guard (ADR-0010) remains the real ceiling if a compile blows past the
/// soft budget anyway. Chosen at 4096 entries — small enough to honour a
/// tight budget, large enough that the apply loop still amortises cache
/// hits across a clause's intermediate frontier.
pub const MEMO_CAP_FLOOR: usize = 4096;

/// Fixed overhead reserved from `max_rss_mb` before allocating the
/// remainder across the memo tables and the projected unique table
/// (ADR-0039 §3). Covers the chunk/IR working set that exists regardless
/// of BDD size. Bytes, not MB.
const OVERHEAD_FLOOR_BYTES: u64 = 64 * 1024 * 1024;

/// Per-entry byte estimates, anchored to the measured sizes documented
/// in `ccm_emitter/bdd.rs` (the `dump_profile` cost model and
/// `DEFAULT_MEMO_CAP` rationale):
///
/// - unique table ≈ 56 B/entry (hashbrown at ~50 % load: 12 B key +
///   4 B val + group/tombstone overhead),
/// - node store ≈ 16 B/entry (`Vec<RawNode>`, exactly 16 B),
/// - **three** apply-memo tables (`not_memo` / `and_memo` / `or_memo`)
///   ≈ 24–32 B/entry each. At the default `1 << 20` per-table cap that
///   is ≈ 96–144 MB **combined** — the 3× factor is baked into
///   [`MEMO_TABLES_BYTES_PER_CAP_ENTRY`] below (per-table cost × 3),
///   because the cap is per-table but the footprint is three tables.
const UNIQUE_BYTES_PER_ENTRY: u64 = 56;

/// Combined cost of one unit of `memo_cap` across all THREE apply-memo
/// tables. Using 32 B/entry per table (the high end of the 24–32 B
/// range, for a conservative — i.e. safe-side — RAM estimate) times the
/// three tables = 96 B per cap-unit. At `memo_cap = 1 << 20` this models
/// ≈ 96 MB, the low end of the documented 96–144 MB combined footprint.
const MEMO_TABLES_BYTES_PER_CAP_ENTRY: u64 = 96;

/// A soft resource budget for a product compile (ADR-0039 §1). Both
/// fields are optional; an all-`None` budget (the [`Default`]) derives
/// the existing unbudgeted knobs and changes nothing.
///
/// `max_rss_mb` is the soft target peak resident set in mebibytes.
/// `max_threads` is plumbed through to the parallel paths where it has
/// effect (the CUDD compile path and the runtime solver session); on a
/// pure in-crate compile it is inert (ADR-0039 §6, honestly narrow —
/// the warning + `CapacityHints` plumb land in a later phase).
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceBudget {
    /// Soft target peak RSS in MiB. `None` ⇒ no RAM target; the default
    /// (`1 << 20`) memo cap and single-partition collapse are used.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_rss_mb: Option<u64>,
    /// Requested maximum thread count for the parallel paths. `None` ⇒
    /// unset (existing single-threaded default).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_threads: Option<u32>,
}

/// Internal knobs derived from a [`ResourceBudget`] (ADR-0039 §2). All
/// three are consumed downstream: `memo_cap` bounds the in-crate apply
/// memos, `cluster_size` (when `Some`) feeds the rung-3 partitioner
/// under the ADR-0012 Amendment 1 precedence rule, and `threads` is the
/// pass-through of `max_threads`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedKnobs {
    /// Per-table apply-memo cap for the in-crate `BddBuilder`. Equals
    /// [`DEFAULT_MEMO_CAP`] when `max_rss_mb` is `None`; otherwise the
    /// budget-derived value, floored at [`MEMO_CAP_FLOOR`].
    pub memo_cap: usize,
    /// Budget-derived cluster size, set only when the projected unique
    /// table alone would exceed the budget (ADR-0039 §3). `None`
    /// preserves single-partition collapse. Subject to the ADR-0012
    /// Amendment 1 precedence rule (explicit `--cluster-size` wins).
    pub cluster_size: Option<usize>,
    /// Pass-through of [`ResourceBudget::max_threads`] (ADR-0039 §2).
    pub threads: Option<u32>,
}

/// Derive internal compile knobs from a soft budget (ADR-0039 §2/§3).
///
/// **Pure**: reads no environment, no `/proc`, no clock. `total_vars_hint`
/// is the caller's estimate of the model's distinct-variable count (the
/// projected unique-table size); pass `None` when unknown.
///
/// Policy (ADR-0039 §3):
/// - `max_rss_mb = None` ⇒ `{ memo_cap: DEFAULT_MEMO_CAP, cluster_size:
///   None, threads: max_threads }` — the unbudgeted, byte-identical
///   path.
/// - Otherwise: reserve [`OVERHEAD_FLOOR_BYTES`] from the budget, then
///   split the remainder across the memo tables and the projected unique
///   table. The resulting `memo_cap` is **monotonic** in `max_rss_mb`
///   (smaller budget ⇒ non-increasing cap) and floored at
///   [`MEMO_CAP_FLOOR`]. If the projected unique table alone
///   (`total_vars_hint × 56 B`) would exceed the post-overhead budget,
///   derive a `cluster_size` that keeps each partition's projected
///   unique footprint under the ceiling.
///
/// The model is a *target*, not a correctness invariant — an imperfect
/// estimate degrades performance, never output bytes (ADR-0039 §3/§8).
pub fn derive_knobs(budget: &ResourceBudget, total_vars_hint: Option<usize>) -> DerivedKnobs {
    let threads = budget.max_threads;

    // No RAM target ⇒ unbudgeted, byte-identical path (ADR-0039 §2):
    // the existing default memo cap, single-partition collapse, threads
    // passed through untouched.
    let Some(max_rss_mb) = budget.max_rss_mb else {
        return DerivedKnobs {
            memo_cap: DEFAULT_MEMO_CAP,
            cluster_size: None,
            threads,
        };
    };

    // Reserve the fixed overhead floor (chunks + IR) from the budget
    // before allocating the remainder (ADR-0039 §3). A budget at or below
    // the overhead floor leaves nothing for the BDD, so the memo cap
    // drops straight to its floor and the unique-table check below uses a
    // zero remainder (every nontrivial model then wants partitioning).
    let budget_bytes = max_rss_mb.saturating_mul(1024 * 1024);
    let remainder_bytes = budget_bytes.saturating_sub(OVERHEAD_FLOOR_BYTES);

    // Split the remainder between the three apply-memo tables and the
    // projected unique table. The memo tables are a cache lever (shrinking
    // them only costs recomputation, never correctness — ADR-0039 §3), so
    // they take a bounded share and the unique projection drives the
    // partition decision. We give the memo tables up to half the
    // remainder, capped at the existing default; the unique projection is
    // checked against the full remainder (the unique table is the
    // non-evictable peak the partitioner exists to bound).
    let memo_budget_bytes = remainder_bytes / 2;
    let memo_cap_from_budget = (memo_budget_bytes / MEMO_TABLES_BYTES_PER_CAP_ENTRY) as usize;

    // memo_cap is monotonic in the budget (a smaller budget gives a
    // smaller `memo_budget_bytes`, hence a non-increasing cap), capped
    // above by the unbudgeted default and floored below by
    // `MEMO_CAP_FLOOR` so a tight budget cannot thrash to zero
    // (ADR-0039 §3 Risks).
    let memo_cap = memo_cap_from_budget
        .min(DEFAULT_MEMO_CAP)
        .max(MEMO_CAP_FLOOR);

    // Derive a cluster_size only when the projected unique table alone
    // would exceed the post-overhead budget (ADR-0039 §3). The projection
    // is `total_vars_hint × UNIQUE_BYTES_PER_ENTRY`; when it fits (or no
    // hint is available) single-partition collapse is preserved
    // (`cluster_size = None`).
    let cluster_size = match total_vars_hint {
        Some(vars) if vars > 0 => {
            let projected_unique_bytes = (vars as u64).saturating_mul(UNIQUE_BYTES_PER_ENTRY);
            if projected_unique_bytes > remainder_bytes {
                // Choose the largest per-partition variable count whose
                // projected unique footprint stays under the remaining
                // budget, clamped to at least 1 (a partition must hold at
                // least one variable) and strictly below the full count
                // (otherwise it is not a partition). `remainder_bytes`
                // may be zero on a sub-overhead budget, in which case the
                // per-partition size floors at 1.
                let per_partition = (remainder_bytes / UNIQUE_BYTES_PER_ENTRY) as usize;
                Some(per_partition.max(1).min(vars.saturating_sub(1)).max(1))
            } else {
                None
            }
        }
        _ => None,
    };

    DerivedKnobs {
        memo_cap,
        cluster_size,
        threads,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_knobs_default_budget_is_unbudgeted() {
        // ADR-0039 §2: an all-None budget derives the existing defaults
        // and changes nothing — memo_cap = DEFAULT_MEMO_CAP, no derived
        // cluster_size, no threads. This is the byte-identical path.
        let knobs = derive_knobs(&ResourceBudget::default(), None);
        assert_eq!(knobs.memo_cap, DEFAULT_MEMO_CAP);
        assert_eq!(knobs.cluster_size, None);
        assert_eq!(knobs.threads, None);
    }

    #[test]
    fn derive_knobs_none_rss_keeps_default_memo_cap() {
        // max_rss_mb = None means "no RAM target": the memo cap stays at
        // the default and no cluster_size is derived, even with a hint.
        // max_threads passes through.
        let budget = ResourceBudget {
            max_rss_mb: None,
            max_threads: Some(4),
        };
        let knobs = derive_knobs(&budget, Some(100_000));
        assert_eq!(knobs.memo_cap, DEFAULT_MEMO_CAP);
        assert_eq!(knobs.cluster_size, None);
        assert_eq!(knobs.threads, Some(4));
    }

    #[test]
    fn derive_knobs_memo_cap_monotonic_in_budget() {
        // ADR-0039 §3: a smaller max_rss_mb yields a NON-INCREASING
        // memo_cap. Sweep a descending sequence of budgets and assert
        // the derived cap never rises as the budget shrinks.
        let budgets_mb = [4096_u64, 2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4, 2, 1];
        let mut prev_cap = usize::MAX;
        for &mb in &budgets_mb {
            let budget = ResourceBudget {
                max_rss_mb: Some(mb),
                max_threads: None,
            };
            let cap = derive_knobs(&budget, None).memo_cap;
            assert!(
                cap <= prev_cap,
                "memo_cap must be non-increasing as budget shrinks: \
                 budget={mb} MB gave cap={cap}, previous (larger) budget gave {prev_cap}"
            );
            prev_cap = cap;
        }
    }

    #[test]
    fn derive_knobs_memo_cap_never_exceeds_default() {
        // A large finite budget should not derive a cap LARGER than the
        // unbudgeted default — the default is the ceiling, the budget can
        // only shrink it.
        let budget = ResourceBudget {
            max_rss_mb: Some(1_000_000),
            max_threads: None,
        };
        let cap = derive_knobs(&budget, None).memo_cap;
        assert!(
            cap <= DEFAULT_MEMO_CAP,
            "derived cap {cap} must not exceed DEFAULT_MEMO_CAP {DEFAULT_MEMO_CAP}"
        );
    }

    #[test]
    fn derive_knobs_memo_cap_floor_holds() {
        // ADR-0039 §3 Risks: an extreme-tiny budget must not drive the
        // memo cap to thrash-zero. The floor (MEMO_CAP_FLOOR) holds.
        let budget = ResourceBudget {
            max_rss_mb: Some(1),
            max_threads: None,
        };
        let cap = derive_knobs(&budget, Some(10_000)).memo_cap;
        assert!(
            cap >= MEMO_CAP_FLOOR,
            "memo_cap {cap} fell below the floor {MEMO_CAP_FLOOR} at a 1 MB budget"
        );
        assert!(cap > 0, "memo_cap must never be zero");
    }

    #[test]
    fn derive_knobs_threads_passthrough() {
        // ADR-0039 §2: threads in DerivedKnobs is a pure pass-through of
        // budget.max_threads, independent of the RAM target.
        for mb in [None, Some(8_u64), Some(4096)] {
            let budget = ResourceBudget {
                max_rss_mb: mb,
                max_threads: Some(7),
            };
            assert_eq!(derive_knobs(&budget, None).threads, Some(7));
        }
    }

    #[test]
    fn derive_knobs_derives_cluster_size_when_unique_exceeds() {
        // ADR-0039 §3: when the projected unique table alone would
        // exceed the post-overhead budget, derive a cluster_size that
        // bounds each partition's projected unique footprint. A tight
        // budget with a large variable hint must produce Some(size) that
        // is at most the full variable count (a real reduction).
        let budget = ResourceBudget {
            max_rss_mb: Some(128),
            max_threads: None,
        };
        let hint = 5_000_000_usize; // huge projected unique table
        let knobs = derive_knobs(&budget, Some(hint));
        let size = knobs
            .cluster_size
            .expect("a tight budget with a huge var hint must derive a cluster_size");
        assert!(size >= 1, "derived cluster_size must be at least 1");
        assert!(
            size < hint,
            "derived cluster_size {size} must be a real reduction below the hint {hint}"
        );
    }

    #[test]
    fn derive_knobs_no_cluster_size_when_unique_fits() {
        // The complement: a generous budget whose projected unique table
        // fits comfortably must NOT derive a cluster_size (single
        // partition preserved). A handful of variables under a 4 GB
        // budget trivially fits.
        let budget = ResourceBudget {
            max_rss_mb: Some(4096),
            max_threads: None,
        };
        assert_eq!(derive_knobs(&budget, Some(50)).cluster_size, None);
    }

    #[test]
    fn derive_knobs_is_pure_deterministic() {
        // ADR-0039 §2: "pure" — same inputs always yield the same
        // DerivedKnobs, with no hidden env/clock/proc dependency.
        let budget = ResourceBudget {
            max_rss_mb: Some(777),
            max_threads: Some(3),
        };
        let a = derive_knobs(&budget, Some(123_456));
        let b = derive_knobs(&budget, Some(123_456));
        assert_eq!(a, b);
    }
}
