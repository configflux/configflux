// SPDX-License-Identifier: BUSL-1.1

//! Compile-time progress signal — configflux-9pjy.3, per ADR-0039 §7 and
//! ADR-0005 Amendment 2.
//!
//! A **weighted phase model** for a product compile: each phase carries a
//! fixed weight (summing to exactly 1.0) and the overall percent is the
//! sum of completed-phase weights plus the current phase's weight times
//! its intra-phase fraction. The BDD apply loop dominates (weight 0.65)
//! and is the only phase reported at fine grain; its intra-phase fraction
//! is `processed_clauses / total_clauses`.
//!
//! ## Byte-stability (ADR-0005 Amendment 2)
//!
//! Progress is a **side-channel**. It is delivered via the [`ProgressSink`]
//! callback (and, at the CLI, `--progress plain|json`); it **never** enters
//! `ccm.manifest.json`, `ccm.symbols.json`, `ccm.bdd.bin`, or
//! `partition-manifest.json`. The default sink is [`NullSink`] (a no-op),
//! and a compile run with no sink wired is bit-for-bit identical to a run
//! today. Emitting events is purely observational: it changes only what is
//! written to the side channel, never the artifact bytes, and it never
//! influences the order or arguments of the BDD builder calls.
//!
//! ## The clause-share equivalence
//!
//! ADR-0039 §7 specifies that across partitions the apply-loop weight is
//! split by each partition's clause share (`clause_indices.len() /
//! total_clauses`) and within a partition by `processed / expressions.len()`.
//! The contribution of a partition `p` to the apply fraction is therefore
//! `(total_p / total) * (processed_p / total_p) = processed_p / total`.
//! Summed over partitions this collapses to `processed_clauses /
//! total_clauses` — exactly the ADR's stated fine grain. The tracker uses
//! that closed form: callers report the **cumulative** processed clause
//! count across all partitions, so a per-partition reset is unnecessary and
//! the apply fraction is always monotonic.

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// The phases of a product compile, in execution order. The BDD apply
/// loop (`BddApplyLoop`) is weighted highest because it dominates wall
/// time and peak memory (ADR-0039 Context); it is the only phase reported
/// at fine grain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Source chunk ingest (parse + merge into the in-memory model).
    Ingest,
    /// Chunk merge / graph assembly.
    Merge,
    /// Link + verify (graph integrity, condition harvest).
    Link,
    /// Variable-order computation for the BDD.
    VarOrder,
    /// The BDD apply loop — the dominant phase (weight 0.65).
    BddApplyLoop,
    /// Manifest / symbols / bdd serialization + disk write.
    Serialize,
}

impl Phase {
    /// All phases in execution order. The slice order defines which
    /// phases are "before" a given phase for the completed-weight sum.
    pub const ORDER: [Phase; 6] = [
        Phase::Ingest,
        Phase::Merge,
        Phase::Link,
        Phase::VarOrder,
        Phase::BddApplyLoop,
        Phase::Serialize,
    ];

    /// The fixed weight of this phase. The weights sum to exactly 1.0
    /// (pinned by [`tests::phase_weights_sum_to_one`]): ingest 0.05,
    /// merge 0.05, link 0.05, var_order 0.10, **apply 0.65**, serialize
    /// 0.10 (ADR-0039 §7).
    pub fn weight(self) -> f32 {
        match self {
            Phase::Ingest => 0.05,
            Phase::Merge => 0.05,
            Phase::Link => 0.05,
            Phase::VarOrder => 0.10,
            Phase::BddApplyLoop => 0.65,
            Phase::Serialize => 0.10,
        }
    }

    /// The cumulative weight of every phase strictly before this one in
    /// [`Phase::ORDER`] — the "completed weight" baseline once this phase
    /// is entered.
    fn preceding_weight(self) -> f32 {
        let mut sum = 0.0_f32;
        for &p in &Phase::ORDER {
            if p == self {
                break;
            }
            sum += p.weight();
        }
        sum
    }
}

/// A single progress event emitted to a [`ProgressSink`]. This is the
/// shape of one line in the `--progress json` JSON-lines stream (ADR-0005
/// Amendment 2: the only newly-parseable surface; the schema is owned by
/// ADR-0039).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ProgressEvent {
    /// The phase this event reports.
    pub phase: Phase,
    /// Overall completion in `[0.0, 1.0]` — completed-phase weights plus
    /// the current phase's weight times its intra-phase fraction.
    /// Monotonic non-decreasing across a compile; terminates at exactly
    /// `1.0`.
    pub pct: f32,
    /// Sampled resident set size in MiB, or `None` where `/proc` is
    /// unavailable (non-Linux). Observational only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_mb: Option<u64>,
    /// Clauses processed so far (cumulative across partitions in the
    /// apply phase; `0` outside it).
    pub processed_clauses: usize,
    /// Total clauses in the model (known up front from the parsed clause
    /// list; `multi_part::emit_multi_part` passes `parsed.len()`).
    pub total_clauses: usize,
    /// Estimated seconds remaining (`elapsed / pct * (1 - pct)`), or
    /// `None` before any progress is made (`pct == 0`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_s: Option<f64>,
}

/// A sink for [`ProgressEvent`]s. The default behaviour is a no-op
/// ([`NullSink`]); a compile with the null sink is byte-identical to one
/// with no progress at all (ADR-0005 Amendment 2).
pub trait ProgressSink {
    /// Called once per emitted progress event. Implementations must not
    /// mutate compile state — the sink is observational.
    fn on_event(&self, event: &ProgressEvent);
}

/// The no-op default sink. Wired wherever a caller does not supply a real
/// sink, so the default compile path runs unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl ProgressSink for NullSink {
    fn on_event(&self, _event: &ProgressEvent) {}
}

/// Summary of a compile's progress, surfaced on `CompileResult`
/// (`progress_summary`) only when a sink was wired — `None` otherwise so
/// the default wire form is unchanged (ADR-0005 Amendment 2). Records the
/// furthest phase reached and the final sampled RSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressSummary {
    /// The furthest phase the compile reached (normally
    /// [`Phase::Serialize`] on success).
    pub peak_phase: Phase,
    /// The last sampled RSS in MiB, or `None` if never sampled / non-Linux.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub final_rss_mb: Option<u64>,
}

/// Drives the weighted phase model and emits [`ProgressEvent`]s to a
/// sink. Owns the start [`Instant`] for ETA. Pure with respect to the
/// artifact: it samples RSS (via [`crate::proc_rss`]) and the clock, but
/// only to populate side-channel events — it never touches compile output.
///
/// Lifetime `'a` ties the tracker to the borrowed sink for the duration
/// of a compile, mirroring how `memo_cap` threads down the emit chain.
pub struct ProgressTracker<'a> {
    sink: &'a dyn ProgressSink,
    started: Instant,
    /// The furthest phase entered so far (for the summary's peak phase).
    peak_phase: Phase,
    /// The last sampled RSS, carried into the summary.
    last_rss_mb: Option<u64>,
    /// The most recent `total_clauses` reported (so the terminal Serialize
    /// event can reuse the model's clause count without the caller — which
    /// does not know it — re-supplying it).
    last_total_clauses: usize,
}

impl<'a> ProgressTracker<'a> {
    /// Create a tracker that emits to `sink`, starting the ETA clock now.
    pub fn new(sink: &'a dyn ProgressSink) -> Self {
        Self {
            sink,
            started: Instant::now(),
            peak_phase: Phase::Ingest,
            last_rss_mb: None,
            last_total_clauses: 0,
        }
    }

    /// Overall completion for `phase` at intra-phase fraction
    /// `intra` (clamped to `[0.0, 1.0]`): the cumulative weight of all
    /// preceding phases plus `phase.weight() * intra`. Pure helper,
    /// exposed for unit testing the monotonicity / termination contract.
    pub fn overall_pct(phase: Phase, intra: f32) -> f32 {
        let intra = intra.clamp(0.0, 1.0);
        phase.preceding_weight() + phase.weight() * intra
    }

    /// Emit a progress event for `phase` at intra-phase fraction `intra`.
    /// `processed_clauses` / `total_clauses` populate the event's clause
    /// counters (both `0`/`total` outside the apply phase). Samples RSS
    /// and computes ETA, then forwards to the sink.
    pub fn emit(
        &mut self,
        phase: Phase,
        intra: f32,
        processed_clauses: usize,
        total_clauses: usize,
    ) {
        if phase_index(phase) > phase_index(self.peak_phase) {
            self.peak_phase = phase;
        }
        if total_clauses > 0 {
            self.last_total_clauses = total_clauses;
        }
        let pct = Self::overall_pct(phase, intra);
        let rss_mb = crate::proc_rss::read_vm_rss_kib().map(kib_to_mib);
        if rss_mb.is_some() {
            self.last_rss_mb = rss_mb;
        }
        let eta_s = estimate_eta_s(self.started.elapsed().as_secs_f64(), pct);
        let event = ProgressEvent {
            phase,
            pct,
            rss_mb,
            processed_clauses,
            total_clauses,
            eta_s,
        };
        self.sink.on_event(&event);
    }

    /// Emit the boundary event marking `phase` complete (intra-phase
    /// fraction `1.0`). Convenience over [`ProgressTracker::emit`].
    pub fn emit_phase_complete(&mut self, phase: Phase, total_clauses: usize) {
        let processed = if phase == Phase::BddApplyLoop {
            total_clauses
        } else {
            0
        };
        self.emit(phase, 1.0, processed, total_clauses);
    }

    /// Emit an apply-loop event from the **cumulative** processed clause
    /// count (across all partitions). The apply intra-phase fraction is
    /// `processed_clauses / total_clauses` — the closed form of the
    /// per-partition clause-share weighting (see the module docs). Safe
    /// for `total_clauses == 0` (fraction treated as complete).
    pub fn emit_apply_progress(&mut self, processed_clauses: usize, total_clauses: usize) {
        let intra = if total_clauses == 0 {
            1.0
        } else {
            (processed_clauses as f32 / total_clauses as f32).clamp(0.0, 1.0)
        };
        self.emit(Phase::BddApplyLoop, intra, processed_clauses, total_clauses);
    }

    /// Finish the compile: emit the terminal [`Phase::Serialize`]
    /// completion event (overall `pct == 1.0`) and return the summary.
    ///
    /// `total_clauses` is a fallback for the final event's clause counter;
    /// when the apply band already reported a model clause count, that
    /// remembered value wins (the emitter knows `parsed.len()`, the API
    /// caller does not). Either way the pct terminates at exactly 1.0
    /// because the phase weights are fixed.
    pub fn finish(mut self, total_clauses: usize) -> ProgressSummary {
        let total = if self.last_total_clauses > 0 {
            self.last_total_clauses
        } else {
            total_clauses
        };
        self.emit_phase_complete(Phase::Serialize, total);
        ProgressSummary {
            peak_phase: self.peak_phase,
            final_rss_mb: self.last_rss_mb,
        }
    }
}

/// Bounded apply-loop emission cadence: emit a progress event at most
/// once every `PROGRESS_CLAUSE_INTERVAL` top-level clauses (reusing the
/// existing `clear_memos` boundary as the single sampling site, ADR-0039
/// §7). Mirrors the `cudd_build` checkpoint default interval.
pub const PROGRESS_CLAUSE_INTERVAL: usize = 100;

/// Per-partition apply-loop progress handle, threaded into the in-crate
/// BDD apply loop alongside `memo_cap`. It binds a [`ProgressTracker`] to
/// one partition's clause-share base offset so the apply loop can report
/// **cumulative** processed clauses without knowing the partition layout.
///
/// The apply loop calls [`ApplyProgress::on_clear`] at the existing
/// `clear_memos` boundary (the single sampling site, ADR-0039 §7) with
/// the count of clauses processed *within this partition* so far; the
/// handle adds `base` (clauses in prior partitions) and forwards the
/// global cumulative count to the tracker at the bounded cadence.
///
/// `None` of this type ⇒ the apply loop's exact pre-progress code path
/// runs (byte-identical default, ADR-0005 Amendment 2).
pub struct ApplyProgress<'t, 'a> {
    tracker: &'t mut ProgressTracker<'a>,
    /// Clauses processed in all prior partitions (the base offset for
    /// this partition's cumulative count).
    base: usize,
    /// Total clauses across the whole model (`parsed.len()`).
    total: usize,
}

impl<'t, 'a> ApplyProgress<'t, 'a> {
    /// Bind a tracker to a partition starting at cumulative clause offset
    /// `base`, within a model of `total` clauses.
    pub fn new(tracker: &'t mut ProgressTracker<'a>, base: usize, total: usize) -> Self {
        Self {
            tracker,
            base,
            total,
        }
    }

    /// Report `processed_in_partition` clauses done in the current
    /// partition. Forwards the global cumulative count
    /// (`base + processed_in_partition`) to the tracker. Called by the
    /// apply loop at each `clear_memos` boundary; the tracker itself does
    /// not throttle, so the apply loop applies the
    /// [`PROGRESS_CLAUSE_INTERVAL`] cadence before calling.
    pub fn on_clear(&mut self, processed_in_partition: usize) {
        let cumulative = self.base.saturating_add(processed_in_partition);
        self.tracker.emit_apply_progress(cumulative, self.total);
    }
}

/// Position of a phase in [`Phase::ORDER`], for "furthest reached"
/// comparisons.
fn phase_index(phase: Phase) -> usize {
    Phase::ORDER.iter().position(|&p| p == phase).unwrap_or(0)
}

/// KiB → MiB with round-half-up (matches the CUDD checkpoint formatter).
fn kib_to_mib(kib: u64) -> u64 {
    (kib + 512) / 1024
}

/// ETA in seconds: `elapsed / pct * (1 - pct)` once `pct > 0`; `None`
/// before any progress (`pct <= 0`) or once complete (`pct >= 1.0`, no
/// time remaining). Clamped non-negative.
fn estimate_eta_s(elapsed_s: f64, pct: f32) -> Option<f64> {
    let pct = pct as f64;
    if pct <= 0.0 || pct >= 1.0 {
        return None;
    }
    Some((elapsed_s / pct * (1.0 - pct)).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A sink that records every event for assertions.
    #[derive(Default)]
    struct RecordingSink {
        events: RefCell<Vec<ProgressEvent>>,
    }

    impl ProgressSink for RecordingSink {
        fn on_event(&self, event: &ProgressEvent) {
            self.events.borrow_mut().push(*event);
        }
    }

    /// ADR-0039 §7: the phase weights must sum to exactly 1.0 so a
    /// completed compile reaches 100%.
    #[test]
    fn phase_weights_sum_to_one() {
        let sum: f32 = Phase::ORDER.iter().map(|p| p.weight()).sum();
        assert!(
            (sum - 1.0).abs() < f32::EPSILON,
            "phase weights must sum to 1.0, got {sum}"
        );
    }

    /// The headline acceptance (ADR-0039 §7 / plan §2.2): feeding a
    /// synthetic clause/partition sequence, the overall pct is monotonic
    /// non-decreasing and terminates at exactly 1.0 (100%).
    #[test]
    fn synthetic_apply_sequence_is_monotonic_and_terminates_at_100() {
        let sink = RecordingSink::default();
        let mut tracker = ProgressTracker::new(&sink);

        // total_clauses across two partitions: 30 + 70 = 100.
        let total = 100;

        // Pre-apply phase boundaries.
        tracker.emit_phase_complete(Phase::Ingest, total);
        tracker.emit_phase_complete(Phase::Merge, total);
        tracker.emit_phase_complete(Phase::Link, total);
        tracker.emit_phase_complete(Phase::VarOrder, total);

        // Apply loop: partition 0 has 30 clauses, partition 1 has 70.
        // Report cumulative processed at the clear_memos boundary.
        let partition_sizes = [30_usize, 70];
        let mut processed = 0;
        for size in partition_sizes {
            for _ in 0..size {
                processed += 1;
                tracker.emit_apply_progress(processed, total);
            }
        }

        // Serialize + terminal.
        let summary = tracker.finish(total);

        let events = sink.events.borrow();
        assert!(!events.is_empty(), "tracker must emit events");

        // Monotonic non-decreasing pct across every event.
        let mut prev = f32::NEG_INFINITY;
        for ev in events.iter() {
            assert!(
                ev.pct + f32::EPSILON >= prev,
                "pct must be monotonic non-decreasing: {} then {}",
                prev,
                ev.pct
            );
            assert!(
                (0.0..=1.0 + f32::EPSILON).contains(&ev.pct),
                "pct out of range: {}",
                ev.pct
            );
            prev = ev.pct;
        }

        // Terminates at EXACTLY 1.0.
        let last = events.last().unwrap();
        assert_eq!(
            last.pct, 1.0,
            "final progress event must be exactly 1.0 (100%), got {}",
            last.pct
        );
        assert_eq!(last.phase, Phase::Serialize);
        assert_eq!(summary.peak_phase, Phase::Serialize);
    }

    /// `overall_pct` places the apply phase between the var-order
    /// completion baseline and the serialize baseline, and rises with the
    /// intra-phase fraction.
    #[test]
    fn overall_pct_apply_spans_its_weight_band() {
        let base = ProgressTracker::overall_pct(Phase::BddApplyLoop, 0.0);
        let mid = ProgressTracker::overall_pct(Phase::BddApplyLoop, 0.5);
        let full = ProgressTracker::overall_pct(Phase::BddApplyLoop, 1.0);
        // Baseline = sum of ingest+merge+link+var_order = 0.25.
        assert!((base - 0.25).abs() < 1e-6, "apply baseline should be 0.25, got {base}");
        // Full apply = 0.25 + 0.65 = 0.90 (serialize is the remaining 0.10).
        assert!((full - 0.90).abs() < 1e-6, "apply-complete should be 0.90, got {full}");
        assert!(base < mid && mid < full, "apply pct must rise with intra fraction");
    }

    /// The apply intra-phase fraction is the closed form of the
    /// per-partition clause-share weighting: a partition's contribution
    /// is proportional to its clause share, and unequal partitions sum to
    /// the same apply-complete value regardless of the split.
    #[test]
    fn multi_partition_clause_share_weighting() {
        // Two different partition splits of the same 100 clauses must
        // reach the identical apply-complete pct (0.90).
        for split in [[10_usize, 90], [50, 50], [1, 99]] {
            let sink = RecordingSink::default();
            let mut tracker = ProgressTracker::new(&sink);
            let total: usize = split.iter().sum();
            let mut processed = 0;
            for size in split {
                processed += size;
                tracker.emit_apply_progress(processed, total);
            }
            let last_apply = sink.events.borrow().last().copied().unwrap();
            assert!(
                (last_apply.pct - 0.90).abs() < 1e-6,
                "apply-complete pct must be 0.90 for split {split:?}, got {}",
                last_apply.pct
            );
            assert_eq!(last_apply.processed_clauses, total);
        }
    }

    /// ETA is absent before any progress (`pct == 0`) and present once
    /// progress is made, and is never negative.
    #[test]
    fn eta_is_none_at_zero_and_some_after_progress() {
        assert_eq!(estimate_eta_s(0.0, 0.0), None, "no ETA before progress");
        assert_eq!(estimate_eta_s(5.0, 0.0), None, "no ETA at pct 0 even with elapsed");
        assert_eq!(estimate_eta_s(5.0, 1.0), None, "no ETA once complete");
        let eta = estimate_eta_s(10.0, 0.5).expect("ETA after progress");
        assert!(eta >= 0.0, "ETA must be non-negative, got {eta}");
        // At 50% with 10s elapsed, ~10s remain.
        assert!((eta - 10.0).abs() < 1e-6, "expected ~10s ETA at 50%, got {eta}");
    }

    /// The null sink accepts events without panicking and observes
    /// nothing — the byte-identical default path (ADR-0005 Amendment 2).
    #[test]
    fn null_sink_is_noop() {
        let sink = NullSink;
        let mut tracker = ProgressTracker::new(&sink);
        tracker.emit_phase_complete(Phase::Ingest, 0);
        tracker.emit_apply_progress(0, 0);
        let summary = tracker.finish(0);
        // finish() advanced peak to Serialize; nothing panicked.
        assert_eq!(summary.peak_phase, Phase::Serialize);
    }

    /// `kib_to_mib` rounds half up, matching the CUDD checkpoint
    /// formatter it shares a contract with.
    #[test]
    fn kib_to_mib_rounds_half_up() {
        assert_eq!(kib_to_mib(0), 0);
        assert_eq!(kib_to_mib(1024), 1);
        assert_eq!(kib_to_mib(1536), 2);
    }
}
