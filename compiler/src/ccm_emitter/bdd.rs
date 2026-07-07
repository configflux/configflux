// SPDX-License-Identifier: BUSL-1.1

use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

use crate::resource_budget::MEMO_CAP_FLOOR;

pub(super) const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
pub(super) const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;
pub(super) const FALSE_REF: u32 = 0;
pub(super) const TRUE_REF: u32 = 1;

/// Default per-table cap for the three apply-memo HashMaps
/// (`not_memo`, `and_memo`, `or_memo`). bd-93oj observed >7 GB RSS at
/// the 10k+cross-tree=50/50 fixture before bounding. Each entry is
/// roughly 24-32 B amortised in `HashMap`, so 1,048,576 entries caps
/// each table around 32-48 MB, an order of magnitude below the
/// envelope. The memos are caches — clearing them is semantically
/// safe; the canonical `unique` table is NOT bounded.
///
/// Tunable per-builder via [`BddBuilder::with_memo_cap`].
///
/// configflux-9pjy.2 / ADR-0039: widened from `pub(super)` to
/// `pub(crate)` so the soft-budget derivation
/// (`resource_budget::derive_knobs`) can anchor its memo-table byte
/// model to the same default cap, re-exported via the `ccm_emitter`
/// boundary. Crate-internal; not part of the public API.
pub(crate) const DEFAULT_MEMO_CAP: usize = 1 << 20;

/// Per-build profile counters (configflux-d49v).
///
/// Always tracked; only emitted to stderr when the
/// `CONFIGFLUX_BDD_BUILDER_PROFILE` env var is set. Tracking cost
/// is a handful of `usize` reads + `usize::max` per clear; on the
/// 10k × 50/50 fixture this adds well under 1 % to wall time and
/// nothing to peak RSS (the struct is `Copy`, ~80 B inline in
/// `BddBuilder`). The output is the data point that the
/// configflux-91xq contingency choice (rung-2.5 algorithmic vs
/// rung-2-tris CUDD-side build vs rung-3 partitioning) is gated on.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct BddProfile {
    peak_unique: usize,
    peak_nodes: usize,
    peak_not_memo: usize,
    peak_and_memo: usize,
    peak_or_memo: usize,
    clear_memos_count: u64,
    per_table_clear_count: u64,
    clause_count: u64,
    first_clause_delta_unique: Option<i64>,
    last_clause_delta_unique: Option<i64>,
    peak_clause_delta_unique: i64,
    prev_clause_unique_len: usize,
}

/// Record of the live adaptive memo-cap shrink (configflux-9pjy.4 /
/// ADR-0039 §5). Surfaced up the emit chain so the compile summary can
/// report how far the budget pushed the cache down. Byte-neutral: the
/// shrink only changes cache size, never the emitted BDD bytes (the memos
/// are caches, not the canonical `unique` table — the
/// `aggressive_memo_eviction_preserves_bdd_bytes` invariant is the
/// guarantee).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoAdaptation {
    /// Number of times the live `memo_cap` was halved in response to an
    /// over-budget RSS sample. `0` when no RSS budget was set or the
    /// budget was never approached.
    pub(crate) shrink_count: u32,
    /// The `memo_cap` the builder started with (the static derived cap, or
    /// [`DEFAULT_MEMO_CAP`] when unbudgeted).
    pub(crate) initial_memo_cap: usize,
    /// The `memo_cap` the builder finished with — `< initial_memo_cap`
    /// exactly when `shrink_count > 0`, floored at [`MEMO_CAP_FLOOR`].
    pub(crate) final_memo_cap: usize,
}

/// RSS-budget threshold numerator/denominator: shrink when sampled RSS
/// exceeds `0.85 ×` the budget (ADR-0039 §5). Expressed as an integer
/// ratio so the comparison reads no float and stays exact.
const RSS_SHRINK_TRIGGER_NUM: u64 = 85;
const RSS_SHRINK_TRIGGER_DEN: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawNode {
    var_index: u32,
    low: u32,
    high: u32,
}

pub(super) struct BddBuilder {
    nodes: Vec<RawNode>,
    unique: HashMap<(u32, u32, u32), u32>,
    not_memo: HashMap<u32, u32>,
    and_memo: HashMap<(u32, u32), u32>,
    or_memo: HashMap<(u32, u32), u32>,
    /// Per-table cap. When any of `not_memo` / `and_memo` / `or_memo`
    /// reaches this size, the table is cleared in full and rebuilt
    /// from scratch (a "clear-when-full" eviction policy). Per
    /// bd-93oj, this is preferable to a per-op LRU because the
    /// per-clause apply traffic is highly local — the cost of
    /// rebuilding a recently cleared cache is dominated by hits on
    /// the still-canonical `unique` table, not on the memos.
    memo_cap: usize,
    /// Soft RSS budget in **KiB** (matching [`crate::proc_rss`]'s unit),
    /// or `None` for the byte-identical default path
    /// (configflux-9pjy.4 / ADR-0039 §5). When `Some`, the builder samples
    /// RSS at the `clear_memos` clause boundary and halves `memo_cap` (down
    /// to [`MEMO_CAP_FLOOR`]) whenever the sample exceeds
    /// `0.85 ×` the budget — trading more clear/rebuild cycles for a lower
    /// resident peak. `None` ⇒ no sampling, no shrink, identical behaviour
    /// to before this field existed.
    rss_budget_kib: Option<u64>,
    /// RSS sampler. Defaults to [`crate::proc_rss::read_vm_rss_kib`]; the
    /// test seam overrides it so the adaptive-shrink mechanism can be
    /// exercised deterministically without depending on `/proc` (which is
    /// absent on non-Linux and non-deterministic on Linux). Never called
    /// when `rss_budget_kib` is `None`, so the unbudgeted path pays
    /// nothing.
    rss_sampler: fn() -> Option<u64>,
    /// The `memo_cap` the builder was constructed with — the baseline the
    /// adaptive shrink reduces from (configflux-9pjy.4).
    initial_memo_cap: usize,
    /// Count of live `memo_cap` halvings so far (configflux-9pjy.4).
    /// Surfaced via [`BddBuilder::adaptation`].
    shrink_count: u32,
    /// Build telemetry; emitted on stderr at end of build when the
    /// `CONFIGFLUX_BDD_BUILDER_PROFILE` env var is set
    /// (configflux-d49v). Tracking is unconditional but cheap.
    profile: BddProfile,
}

impl Default for BddBuilder {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            unique: HashMap::new(),
            not_memo: HashMap::new(),
            and_memo: HashMap::new(),
            or_memo: HashMap::new(),
            memo_cap: DEFAULT_MEMO_CAP,
            rss_budget_kib: None,
            rss_sampler: crate::proc_rss::read_vm_rss_kib,
            initial_memo_cap: DEFAULT_MEMO_CAP,
            shrink_count: 0,
            profile: BddProfile::default(),
        }
    }
}

impl BddBuilder {
    /// Construct a builder with a custom apply-memo cap.
    ///
    /// The eviction-correctness regression test in
    /// `compiler::ccm_emitter::tests` uses this to force frequent clears
    /// (cap=1) and assert byte-stable output. configflux-9pjy.2 /
    /// ADR-0039 promotes it to production: the soft resource budget
    /// derives a `memo_cap` (`resource_budget::derive_knobs`) that the
    /// emit chain threads down to here as `Some(cap)`. The cap is a
    /// pure cache lever — a smaller cap only costs recomputation, never
    /// output bytes (the memos are caches, not the canonical `unique`
    /// table; the byte-stability tests pin `cap=1` ≡ default output).
    /// Callers that pass no budget continue to use
    /// [`BddBuilder::default`] (`memo_cap = DEFAULT_MEMO_CAP`).
    pub(crate) fn with_memo_cap(memo_cap: usize) -> Self {
        let cap = memo_cap.max(1);
        Self {
            memo_cap: cap,
            initial_memo_cap: cap,
            ..Self::default()
        }
    }

    /// Construct a builder with a custom apply-memo cap **and** a soft RSS
    /// budget that drives the live adaptive shrink (configflux-9pjy.4 /
    /// ADR-0039 §5).
    ///
    /// `rss_budget_kib` is the soft target peak resident set in **KiB**
    /// (the unit [`crate::proc_rss::read_vm_rss_kib`] reports). When `Some`,
    /// the builder samples RSS at each `clear_memos` clause boundary and
    /// halves the live `memo_cap` — down to [`MEMO_CAP_FLOOR`] — whenever
    /// the sample exceeds `0.85 ×` the budget, trading wall-clock for a
    /// lower peak. When `None` this is exactly [`BddBuilder::with_memo_cap`]
    /// — no sampling, byte-identical output.
    ///
    /// Byte-neutrality (ADR-0039 §8): the shrink only resizes caches; the
    /// canonical `unique` table and node arena are untouched, so the
    /// emitted bytes are identical to the same build without an RSS budget.
    pub(crate) fn with_memo_cap_and_rss_budget(
        memo_cap: usize,
        rss_budget_kib: Option<u64>,
    ) -> Self {
        let cap = memo_cap.max(1);
        Self {
            memo_cap: cap,
            initial_memo_cap: cap,
            rss_budget_kib,
            ..Self::default()
        }
    }

    /// Test seam: as [`BddBuilder::with_memo_cap_and_rss_budget`] but with
    /// an injectable RSS sampler so the adaptive shrink can be exercised
    /// deterministically without depending on `/proc`. Production code
    /// always uses the real sampler via the constructors above.
    #[cfg(test)]
    pub(super) fn with_memo_cap_rss_budget_and_sampler_for_test(
        memo_cap: usize,
        rss_budget_kib: Option<u64>,
        rss_sampler: fn() -> Option<u64>,
    ) -> Self {
        let cap = memo_cap.max(1);
        Self {
            memo_cap: cap,
            initial_memo_cap: cap,
            rss_budget_kib,
            rss_sampler,
            ..Self::default()
        }
    }

    /// Snapshot of the live adaptive memo-cap shrink (configflux-9pjy.4).
    /// Read after a build to surface adaptation in the compile summary.
    pub(super) fn adaptation(&self) -> MemoAdaptation {
        MemoAdaptation {
            shrink_count: self.shrink_count,
            initial_memo_cap: self.initial_memo_cap,
            final_memo_cap: self.memo_cap,
        }
    }

    /// Sample RSS and, if over the soft budget, halve the live `memo_cap`
    /// (configflux-9pjy.4 / ADR-0039 §5). Called from [`Self::clear_memos`]
    /// — the existing once-per-top-level-clause boundary and the single
    /// apply-loop sampling site (no second sampling site is added,
    /// ADR-0039 §7). No-op (and no sample taken) when no RSS budget is set,
    /// so the unbudgeted path is unchanged.
    ///
    /// Mechanism: when the sampled RSS exceeds `0.85 ×` the budget and the
    /// cap is still above [`MEMO_CAP_FLOOR`], the cap is halved (floored).
    /// A smaller cap makes the per-table `>= memo_cap` clears in
    /// `not`/`and`/`or` fire sooner — more clear/rebuild cycles, lower
    /// resident RAM, more wall-clock. The cap never rises again within a
    /// build (the resident peak only grows), and never drops below the
    /// floor (ADR-0039 §3 Risks: a tight budget must not thrash to zero).
    fn maybe_shrink_memo_cap(&mut self) {
        let Some(budget_kib) = self.rss_budget_kib else {
            return;
        };
        if self.memo_cap <= MEMO_CAP_FLOOR {
            return;
        }
        let Some(rss_kib) = (self.rss_sampler)() else {
            // No reading available (non-Linux / malformed status): cannot
            // act on a budget we cannot measure. The hard guard remains
            // the real ceiling (ADR-0039 §5).
            return;
        };
        let trigger_kib = budget_kib.saturating_mul(RSS_SHRINK_TRIGGER_NUM) / RSS_SHRINK_TRIGGER_DEN;
        if rss_kib > trigger_kib {
            self.memo_cap = (self.memo_cap / 2).max(MEMO_CAP_FLOOR);
            self.shrink_count = self.shrink_count.saturating_add(1);
        }
    }

    /// Emit a single structured `BDD-PROFILE:` block on stderr if the
    /// env var `CONFIGFLUX_BDD_BUILDER_PROFILE` is set. Called once
    /// at end of build (after the clause AND-fold loop, before
    /// `serialize`). Output is for human + grep'able log inspection
    /// only; never parsed into byte-stable artifacts.
    ///
    /// Estimated bytes: hashbrown ≈ 56 B per `unique` entry at ~50 %
    /// load (12 B key + 4 B val + group/tombstone overhead);
    /// `nodes` is `Vec<RawNode>` at exactly 16 B per entry. These
    /// estimates ARE rough — the architect-Q1 verdict is that this
    /// telemetry is enough to disambiguate which contingency rung
    /// the configflux-91xq escalation should pick.
    pub(super) fn dump_profile(&self) {
        if std::env::var_os("CONFIGFLUX_BDD_BUILDER_PROFILE").is_none() {
            return;
        }
        let p = self.profile;
        let peak_unique = p.peak_unique.max(self.unique.len());
        let peak_nodes = p.peak_nodes.max(self.nodes.len());
        let est_unique_mb = (peak_unique.saturating_mul(56)) / (1024 * 1024);
        let est_nodes_mb = (peak_nodes.saturating_mul(16)) / (1024 * 1024);
        eprintln!(
            "BDD-PROFILE:\n  \
             peak_unique           = {} entries (~{} MB at 56 B/entry)\n  \
             peak_nodes            = {} entries (~{} MB at 16 B/entry)\n  \
             peak_not_memo         = {} entries\n  \
             peak_and_memo         = {} entries\n  \
             peak_or_memo          = {} entries\n  \
             clear_memos_count     = {}\n  \
             per_table_clear_count = {}\n  \
             clause_count          = {}\n  \
             first_clause_delta_u  = {:?}\n  \
             last_clause_delta_u   = {:?}\n  \
             peak_clause_delta_u   = {}",
            peak_unique,
            est_unique_mb,
            peak_nodes,
            est_nodes_mb,
            p.peak_not_memo,
            p.peak_and_memo,
            p.peak_or_memo,
            p.clear_memos_count,
            p.per_table_clear_count,
            p.clause_count,
            p.first_clause_delta_unique,
            p.last_clause_delta_unique,
            p.peak_clause_delta_unique,
        );
    }

    /// Drop all apply-memo entries while leaving the canonical
    /// `unique` table and the node arena intact. Called by
    /// `build_bdd_bin` between top-level clause `and()`s — the
    /// clause boundary is a natural reset point because subsequent
    /// `apply` recursions seldom re-visit pairs from the previous
    /// clause's intermediate frontier. Also called internally when
    /// any single memo table reaches `memo_cap`.
    pub(super) fn clear_memos(&mut self) {
        // d49v telemetry: clear_memos() is the canonical clause
        // boundary signal in build_bdd_bin_with_builder (it is the
        // only call site outside per-table cap clears in not/and/or).
        // Record peaks BEFORE clearing so the highest pre-clear sizes
        // survive into the dumped profile.
        let unique_now = self.unique.len();
        let nodes_now = self.nodes.len();
        let not_now = self.not_memo.len();
        let and_now = self.and_memo.len();
        let or_now = self.or_memo.len();
        self.profile.peak_unique = self.profile.peak_unique.max(unique_now);
        self.profile.peak_nodes = self.profile.peak_nodes.max(nodes_now);
        self.profile.peak_not_memo = self.profile.peak_not_memo.max(not_now);
        self.profile.peak_and_memo = self.profile.peak_and_memo.max(and_now);
        self.profile.peak_or_memo = self.profile.peak_or_memo.max(or_now);
        let delta = unique_now as i64 - self.profile.prev_clause_unique_len as i64;
        if self.profile.first_clause_delta_unique.is_none() {
            self.profile.first_clause_delta_unique = Some(delta);
        }
        self.profile.last_clause_delta_unique = Some(delta);
        if delta > self.profile.peak_clause_delta_unique {
            self.profile.peak_clause_delta_unique = delta;
        }
        self.profile.prev_clause_unique_len = unique_now;
        self.profile.clause_count += 1;
        self.profile.clear_memos_count += 1;
        self.not_memo.clear();
        self.and_memo.clear();
        self.or_memo.clear();
        // configflux-9pjy.4 / ADR-0039 §5: the live adaptive lever. At this
        // same clause boundary (the single apply-loop sampling site), sample
        // RSS and, if over the soft budget, halve the live memo_cap toward
        // the floor. No-op when no RSS budget is set, so the unbudgeted path
        // is byte- and behaviour-identical. Byte-neutral when active: only
        // the cache size changes, never `unique`/`nodes`/the emitted bytes.
        self.maybe_shrink_memo_cap();
        // configflux-d49v: optional periodic checkpoint so a build
        // killed by resource_guard at clause N still leaves the most
        // recent profile snapshot in the log (otherwise dump_profile
        // only fires at end-of-build). Interval defaults to 100; set
        // CONFIGFLUX_BDD_BUILDER_PROFILE_CHECKPOINT to a positive
        // integer to override. Tied to the same env-var gate as
        // dump_profile() — opt-in only, byte-stable behaviour
        // unchanged when not set.
        if std::env::var_os("CONFIGFLUX_BDD_BUILDER_PROFILE").is_some() {
            let interval: u64 = std::env::var("CONFIGFLUX_BDD_BUILDER_PROFILE_CHECKPOINT")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|n: &u64| *n > 0)
                .unwrap_or(100);
            if self.profile.clause_count % interval == 0 {
                eprintln!(
                    "BDD-CHECKPOINT: clause={} unique={} nodes={} not={} and={} or={}",
                    self.profile.clause_count,
                    self.unique.len(),
                    self.nodes.len(),
                    self.profile.peak_not_memo,
                    self.profile.peak_and_memo,
                    self.profile.peak_or_memo,
                );
            }
        }
    }

    pub(super) fn var(&mut self, var_index: u32) -> u32 {
        self.mk_node(var_index, FALSE_REF, TRUE_REF)
    }

    pub(super) fn not(&mut self, node: u32) -> u32 {
        match node {
            FALSE_REF => TRUE_REF,
            TRUE_REF => FALSE_REF,
            _ => {
                if let Some(cached) = self.not_memo.get(&node) {
                    return *cached;
                }
                let raw = self.raw(node);
                let low = self.not(raw.low);
                let high = self.not(raw.high);
                let out = self.mk_node(raw.var_index, low, high);
                if self.not_memo.len() >= self.memo_cap {
                    self.profile.peak_not_memo =
                        self.profile.peak_not_memo.max(self.not_memo.len());
                    self.profile.per_table_clear_count += 1;
                    self.not_memo.clear();
                }
                self.not_memo.insert(node, out);
                out
            }
        }
    }

    pub(super) fn and(&mut self, a: u32, b: u32) -> u32 {
        let (a, b) = ordered_pair(a, b);
        if let Some(cached) = self.and_memo.get(&(a, b)) {
            return *cached;
        }
        let out = self.apply(a, b, true);
        if self.and_memo.len() >= self.memo_cap {
            self.profile.peak_and_memo =
                self.profile.peak_and_memo.max(self.and_memo.len());
            self.profile.per_table_clear_count += 1;
            self.and_memo.clear();
        }
        self.and_memo.insert((a, b), out);
        out
    }

    pub(super) fn or(&mut self, a: u32, b: u32) -> u32 {
        let (a, b) = ordered_pair(a, b);
        if let Some(cached) = self.or_memo.get(&(a, b)) {
            return *cached;
        }
        let out = self.apply(a, b, false);
        if self.or_memo.len() >= self.memo_cap {
            self.profile.peak_or_memo =
                self.profile.peak_or_memo.max(self.or_memo.len());
            self.profile.per_table_clear_count += 1;
            self.or_memo.clear();
        }
        self.or_memo.insert((a, b), out);
        out
    }

    pub(super) fn serialize(&self, root: u32, var_count: u32) -> Vec<u8> {
        let (nodes, root_id) = self.ordered_nodes(root);
        let mut bytes = Vec::with_capacity(24 + nodes.len() * 16);
        bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
        bytes.push(CCM_BDD_BIN_VERSION);
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes.extend_from_slice(&var_count.to_le_bytes());
        bytes.extend_from_slice(&(nodes.len() as u32 + 2).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&root_id.to_le_bytes());
        push_node(
            &mut bytes,
            TERMINAL_VAR_INDEX,
            TERMINAL_FALSE,
            TERMINAL_FALSE,
        );
        push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
        for node in nodes {
            push_node(&mut bytes, node.var_index, node.low, node.high);
        }
        bytes
    }

    fn mk_node(&mut self, var_index: u32, low: u32, high: u32) -> u32 {
        if low == high {
            return low;
        }
        let key = (var_index, low, high);
        if let Some(id) = self.unique.get(&key) {
            return *id;
        }
        let id = self.nodes.len() as u32 + 2;
        self.nodes.push(RawNode {
            var_index,
            low,
            high,
        });
        self.unique.insert(key, id);
        id
    }

    fn apply(&mut self, a: u32, b: u32, is_and: bool) -> u32 {
        if let (Some(av), Some(bv)) = (terminal_value(a), terminal_value(b)) {
            return bool_ref(if is_and { av && bv } else { av || bv });
        }
        let top = match (self.var_index(a), self.var_index(b)) {
            (Some(x), Some(y)) => x.min(y),
            (Some(x), None) | (None, Some(x)) => x,
            (None, None) => unreachable!("terminal case handled above"),
        };
        let (a_low, a_high) = self.cofactor(a, top);
        let (b_low, b_high) = self.cofactor(b, top);
        let low = if is_and {
            self.and(a_low, b_low)
        } else {
            self.or(a_low, b_low)
        };
        let high = if is_and {
            self.and(a_high, b_high)
        } else {
            self.or(a_high, b_high)
        };
        self.mk_node(top, low, high)
    }

    fn cofactor(&self, node: u32, var_index: u32) -> (u32, u32) {
        if self.var_index(node) == Some(var_index) {
            let raw = self.raw(node);
            (raw.low, raw.high)
        } else {
            (node, node)
        }
    }

    fn ordered_nodes(&self, root: u32) -> (Vec<RawNode>, u32) {
        if root == FALSE_REF {
            return (Vec::new(), TERMINAL_FALSE);
        }
        if root == TRUE_REF {
            return (Vec::new(), TERMINAL_TRUE);
        }
        let mut reachable = BTreeMap::<u32, Vec<u32>>::new();
        self.collect_reachable(root, &mut reachable);
        let mut remap = HashMap::<u32, u32>::new();
        let mut out = Vec::new();
        for (_var, ids) in reachable.iter().rev() {
            let mut ids = ids.clone();
            ids.sort_by_key(|id| {
                let raw = self.raw(*id);
                (child_id(raw.low, &remap), child_id(raw.high, &remap))
            });
            for id in ids {
                let raw = self.raw(id);
                let node = RawNode {
                    var_index: raw.var_index,
                    low: child_id(raw.low, &remap),
                    high: child_id(raw.high, &remap),
                };
                remap.insert(id, out.len() as u32 + 2);
                out.push(node);
            }
        }
        (out, *remap.get(&root).expect("root must be reachable"))
    }

    fn collect_reachable(&self, node: u32, out: &mut BTreeMap<u32, Vec<u32>>) {
        if terminal_value(node).is_some() {
            return;
        }
        let raw = self.raw(node);
        if out.entry(raw.var_index).or_default().contains(&node) {
            return;
        }
        self.collect_reachable(raw.low, out);
        self.collect_reachable(raw.high, out);
        out.entry(raw.var_index).or_default().push(node);
    }

    fn var_index(&self, node: u32) -> Option<u32> {
        terminal_value(node)
            .is_none()
            .then(|| self.raw(node).var_index)
    }

    fn raw(&self, node: u32) -> RawNode {
        self.nodes[(node - 2) as usize]
    }
}

pub(super) fn node_count(bdd_bin: &[u8]) -> Result<u32> {
    if bdd_bin.len() < 16 {
        bail!("ccm.bdd.bin header is truncated");
    }
    Ok(u32::from_le_bytes([
        bdd_bin[12],
        bdd_bin[13],
        bdd_bin[14],
        bdd_bin[15],
    ]))
}

pub(super) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn push_node(bytes: &mut Vec<u8>, var_index: u32, low: u32, high: u32) {
    bytes.extend_from_slice(&var_index.to_le_bytes());
    bytes.extend_from_slice(&low.to_le_bytes());
    bytes.extend_from_slice(&high.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&[0, 0, 0]);
}

fn child_id(id: u32, remap: &HashMap<u32, u32>) -> u32 {
    match id {
        FALSE_REF => 0,
        TRUE_REF => 1,
        _ => *remap
            .get(&id)
            .expect("child must be assigned before parent"),
    }
}

fn ordered_pair(a: u32, b: u32) -> (u32, u32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn terminal_value(node: u32) -> Option<bool> {
    match node {
        FALSE_REF => Some(false),
        TRUE_REF => Some(true),
        _ => None,
    }
}

fn bool_ref(value: bool) -> u32 {
    if value {
        TRUE_REF
    } else {
        FALSE_REF
    }
}

#[cfg(test)]
pub(super) fn eval(builder: &BddBuilder, node: u32, assignments: &BTreeMap<u32, bool>) -> bool {
    if let Some(value) = terminal_value(node) {
        return value;
    }
    let raw = builder.raw(node);
    if *assignments.get(&raw.var_index).unwrap_or(&false) {
        eval(builder, raw.high, assignments)
    } else {
        eval(builder, raw.low, assignments)
    }
}
