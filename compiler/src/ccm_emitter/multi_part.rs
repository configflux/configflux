// SPDX-License-Identifier: BUSL-1.1

//! Multi-part v2 CCM wire-format emission — configflux-vmlb, per
//! ADR-0012 §4 and ADR-0005 Amendment 1 §11–§16.
//!
//! This module owns the top-level `<out>/ccm/` layout: it runs the
//! partitioner from `super::partitioner`, emits one sibling
//! `partition-NNNN/` subdirectory per cluster (plus an optional
//! `partition-bridge/`), writes the top-level `ccm.manifest.json` /
//! `ccm.symbols.json` / `partition-manifest.json`, and computes the
//! ADR-0012 §5 top-level `ccm_hash` chain.
//!
//! The v2 layout is **always multi-part**, even for single-partition
//! models — small inputs trivially produce
//! `partitions: ["partition-0000"], has_bridge: false`.
//!
//! Determinism: same `(ConditionModel, heuristic, construction,
//! cluster_size)` produces bit-identical output bytes on disk. The
//! partitioner is deterministic (configflux-0qo3); per-partition
//! emission reuses the v1 byte-stable hash recipe (ADR-0005 §5) with
//! the v2 domain tag; the top-level hash chains over per-partition
//! hashes in emission order.

use super::partitioner::{partition, BridgeSpec, ClusterSpec};
use super::timings::{stage, StageTimings};
use super::var_order::{compute_variable_order, VarOrderHeuristic};
use super::{
    algorithm_params_pub, build_bdd_bin_via_construction, json_line, node_count_of,
    parse_condition_model, serialize_per_partition_manifest, serialize_symbols, ConditionExpr,
    ConditionModel, Construction, MemoAdaptation, CCM_HASH_DOMAIN_TAG,
    CCM_PARTITION_MANIFEST_SCHEMA_VERSION, CCM_SCHEMA_VERSION,
};
// configflux-9pjy.3 / ADR-0039 §7: the compile-time progress signal. The
// emitter drives the VarOrder → BddApplyLoop band; `None` keeps the
// byte-identical default path (ADR-0005 Amendment 2).
use crate::progress::{ApplyProgress, Phase, ProgressTracker};
use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Filename of the partition index living in `<out>/ccm/`.
pub(crate) const PARTITION_MANIFEST_FILENAME: &str = "partition-manifest.json";

/// Directory name for the bridge partition (literal; not numeric).
pub(crate) const BRIDGE_PARTITION_DIR: &str = "partition-bridge";

/// Cross-partition advisory threshold (configflux-9pjy.4 / ADR-0039 §5):
/// if a partition's post-build RSS reaches `0.85 ×` the soft budget, record
/// an advisory that the effective `cluster_size` was too large for the
/// budget. Same `0.85` ratio the live memo shrink uses; an integer
/// numerator/denominator keeps the comparison float-free.
const ADVISORY_TRIGGER_NUM: u64 = 85;
const ADVISORY_TRIGGER_DEN: u64 = 100;

/// Aggregated soft-budget outcome of a multi-part emission
/// (configflux-9pjy.4 / ADR-0039 §5). Metadata only — it never enters the
/// byte-stable artifact (ADR-0005 §6); the emitter returns it so the
/// compile summary can surface (a) how far the live memo shrink pushed the
/// cap and (b) the next-run "cluster_size too large for the budget"
/// advisory. The [`Default`] (all-zero / advisory `false`) is what the
/// unbudgeted path produces.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EmitBudgetOutcome {
    /// Total live memo-cap halvings summed across all partitions.
    pub(crate) memo_shrink_count: u32,
    /// The smallest final memo cap any partition finished at (the deepest
    /// the budget pushed the cache), or `None` when no partition shrank.
    pub(crate) min_final_memo_cap: Option<usize>,
    /// `true` when at least one partition's post-build RSS approached the
    /// soft budget — the next-run hint that `cluster_size` was too large.
    /// Advisory only: the layout is NOT re-partitioned mid-run (ADR-0012
    /// determinism).
    pub(crate) cluster_size_too_large: bool,
    /// The effective `cluster_size` this emission actually used, recorded so
    /// the advisory can name a concrete value for the operator's next run.
    /// Filled by `compiler_core` (the owner of the explicit-vs-derived
    /// precedence decision); `None` on the unbudgeted path. `usize::MAX`
    /// means single-partition collapse (no `--cluster-size`).
    pub(crate) effective_cluster_size: Option<usize>,
}

/// Per-partition build telemetry (ADR-0012 §Consequences). Emitted to
/// stderr only when `CONFIGFLUX_CUDD_BUILDER_PROFILE` is set; never
/// written into byte-stable artifacts (ADR-0005 §6).
fn emit_partition_telemetry(cluster_index: Option<usize>, var_count: usize, node_count: u64) {
    if std::env::var_os("CONFIGFLUX_CUDD_BUILDER_PROFILE").is_none() {
        return;
    }
    let label = match cluster_index {
        Some(i) => format!("{i:04}"),
        None => "bridge".to_string(),
    };
    eprintln!(
        "CUDD-PARTITION: cluster={label} var_count={var_count} peak_nodes={node_count}"
    );
}

/// Emit a v2 multi-part CCM under `<dir>/`. Always multi-part, even
/// for single-partition models (single-partition collapse still uses
/// the `partition-0000/` subdirectory layout).
///
/// `cluster_size = usize::MAX` collapses to a single partition and
/// no bridge — every existing FAMA fixture trivially takes this
/// branch (ADR-0012 §2).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_multi_part(
    model: &ConditionModel,
    dir: &Path,
    heuristic: VarOrderHeuristic,
    construction: Construction,
    cluster_size: usize,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    t: Option<&mut StageTimings>,
) -> Result<EmitBudgetOutcome> {
    emit_multi_part_with_progress(
        model,
        dir,
        heuristic,
        construction,
        cluster_size,
        memo_cap,
        rss_budget_kib,
        None,
        t,
    )
}

/// As [`emit_multi_part`], plus the optional compile-time progress tracker
/// (configflux-9pjy.3 / ADR-0039 §7). The tracker drives the VarOrder →
/// BddApplyLoop band: it marks variable ordering complete on entry to the
/// partition loop and threads a per-partition [`ApplyProgress`] (carrying
/// each partition's clause-share base offset) into the apply loop, so the
/// reported `pct` advances by `processed_clauses / total_clauses` across
/// the whole model regardless of the partition split. The terminal
/// Serialize completion (pct 1.0) is emitted by the caller's
/// `ProgressTracker::finish`. `progress = None` keeps the byte-identical
/// default path (ADR-0005 Amendment 2).
///
/// configflux-9pjy.4 / ADR-0039 §5: `rss_budget_kib` (the soft RSS budget
/// in KiB) threads into each partition's apply loop to drive the live
/// memo-cap shrink, and the returned [`EmitBudgetOutcome`] aggregates the
/// per-partition adaptation plus the cross-partition advisory. The advisory
/// is **next-run metadata only** — the partition layout is byte-stable and
/// is never re-split mid-run (ADR-0012 determinism). `rss_budget_kib = None`
/// keeps the byte-identical default path and yields the zeroed outcome.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_multi_part_with_progress(
    model: &ConditionModel,
    dir: &Path,
    heuristic: VarOrderHeuristic,
    construction: Construction,
    cluster_size: usize,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    mut progress: Option<&mut ProgressTracker>,
    mut t: Option<&mut StageTimings>,
) -> Result<EmitBudgetOutcome> {
    let parsed = stage(&mut t, |s| &mut s.clause_parse, || {
        parse_condition_model(model)
    })?;
    let total_clauses = parsed.len();
    let plan = partition(&parsed, cluster_size);

    stage(&mut t, |s| &mut s.disk_write, || {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create CCM dir '{}'", dir.display()))
    })?;

    // configflux-9pjy.3: variable ordering precedes the apply loop; mark
    // the VarOrder phase complete so the reported pct sits at the apply
    // baseline before the first clause is folded.
    if let Some(p) = progress.as_deref_mut() {
        p.emit_phase_complete(Phase::VarOrder, total_clauses);
    }

    let mut per_partition_hashes: Vec<[u8; 32]> = Vec::with_capacity(plan.clusters.len() + 1);
    let mut union_symbols: Vec<(String, String)> = Vec::new();
    let mut total_var_count: u64 = 0;
    let mut total_node_count: u64 = 0;
    // Cumulative clauses processed in prior partitions — the base offset
    // each partition's ApplyProgress adds to its in-partition count so the
    // global apply fraction is monotonic across partition boundaries.
    let mut clauses_done: usize = 0;
    // configflux-9pjy.4: fold the per-partition adaptation + advisory into
    // the emit-wide outcome as we go (metadata; never touches output bytes).
    let mut outcome = EmitBudgetOutcome::default();
    // The advisory threshold in KiB, computed once when an RSS budget is set.
    let advisory_trigger_kib = rss_budget_kib
        .map(|b| b.saturating_mul(ADVISORY_TRIGGER_NUM) / ADVISORY_TRIGGER_DEN);

    for (idx, cluster) in plan.clusters.iter().enumerate() {
        let sub_dir = dir.join(format!("partition-{idx:04}"));
        let digest = emit_cluster_partition(
            model,
            &parsed,
            cluster,
            &sub_dir,
            heuristic,
            construction,
            memo_cap,
            rss_budget_kib,
            Some(idx),
            progress.as_deref_mut().map(|p| (p, clauses_done, total_clauses)),
            t.as_deref_mut(),
        )?;
        clauses_done = clauses_done.saturating_add(cluster.clause_indices.len());
        fold_partition_outcome(&mut outcome, &digest, advisory_trigger_kib);
        per_partition_hashes.push(digest.ccm_hash_raw);
        total_var_count = total_var_count.saturating_add(digest.var_count as u64);
        total_node_count = total_node_count.saturating_add(digest.node_count);
        for v in &cluster.variables {
            union_symbols.push(v.clone());
        }
    }

    let has_bridge = plan.bridge.is_some();
    if let Some(bridge) = &plan.bridge {
        let sub_dir = dir.join(BRIDGE_PARTITION_DIR);
        let digest = emit_bridge_partition(
            model,
            &parsed,
            bridge,
            &sub_dir,
            heuristic,
            construction,
            memo_cap,
            rss_budget_kib,
            progress.as_deref_mut().map(|p| (p, clauses_done, total_clauses)),
            t.as_deref_mut(),
        )?;
        clauses_done = clauses_done.saturating_add(bridge.clause_indices.len());
        let _ = clauses_done;
        fold_partition_outcome(&mut outcome, &digest, advisory_trigger_kib);
        per_partition_hashes.push(digest.ccm_hash_raw);
        total_var_count = total_var_count.saturating_add(digest.var_count as u64);
        total_node_count = total_node_count.saturating_add(digest.node_count);
        for v in &bridge.variables {
            union_symbols.push(v.clone());
        }
    }

    // Build dedup'd union symbols for the top-level convenience
    // surface (per-partition `ccm.symbols.json` files are
    // authoritative; the top-level is a sorted union).
    let (top_symbols_json, top_manifest_bytes, partition_manifest_bytes, partition_count_label) =
        stage(&mut t, |s| &mut s.manifest_serialize, || {
            let mut seen: BTreeMap<(String, String), ()> = BTreeMap::new();
            for v in union_symbols.drain(..) {
                seen.insert(v, ());
            }
            let union_symbols: Vec<(String, String)> =
                seen.into_iter().map(|(k, _)| k).collect();
            let top_symbols_json = serialize_symbols(&union_symbols)?;

            let partition_names: Vec<String> = (0..plan.clusters.len())
                .map(|i| format!("partition-{i:04}"))
                .chain(if has_bridge {
                    Some(BRIDGE_PARTITION_DIR.to_string()).into_iter()
                } else {
                    None.into_iter()
                })
                .collect();

            // ADR-0054 §5.4 roster. `root_index` is the constraint's
            // position in `model.constraints`, which `parse_condition_model`
            // preserves as a contiguous block in the root AND-fold.
            // Synthesized cardinality conjuncts are NOT rostered.
            let roster: Vec<ConstraintRosterEntry> = model
                .constraints
                .iter()
                .enumerate()
                .map(|(i, (id, condition))| ConstraintRosterEntry {
                    condition,
                    id,
                    root_index: i as u32,
                })
                .collect();

            // Top-level pre-image: canonical JSON of the top-level
            // manifest WITHOUT `ccm_hash` per ADR-0005 §5 Step 3
            // elision.
            let top_pre = TopLevelPreimage {
                algorithm: construction.algorithm_tag(),
                algorithm_params: &algorithm_params_pub(heuristic, construction),
                bound_model_hash: &model.bound_model_hash,
                constraints: &roster,
                node_count: total_node_count,
                partition_manifest: PARTITION_MANIFEST_FILENAME,
                schema_version: CCM_SCHEMA_VERSION,
                var_count: total_var_count as u32,
            };
            let top_manifest_canon = serde_json::to_vec(&top_pre)?;

            // partition-manifest pre-image: canonical JSON WITHOUT
            // `top_level_ccm_hash` per ADR-0012 §5 Step 2.
            let pm_preimage = PartitionManifestPreimage {
                has_bridge,
                partitions: &partition_names,
                schema_version: CCM_PARTITION_MANIFEST_SCHEMA_VERSION,
            };
            let pm_canon = serde_json::to_vec(&pm_preimage)?;

            // ADR-0012 §5 Step 3 hash assembly.
            let mut hasher = Sha256::new();
            hasher.update(CCM_HASH_DOMAIN_TAG);
            hasher.update(&top_manifest_canon);
            hasher.update(b"\n");
            hasher.update(&pm_canon);
            hasher.update(b"\n");
            for h in &per_partition_hashes {
                hasher.update(h);
            }
            let top_level_ccm_hash_raw: [u8; 32] = hasher.finalize().into();
            let top_level_ccm_hash_hex = hex32(&top_level_ccm_hash_raw);

            let top_manifest_bytes = json_line(&TopLevelManifestOut {
                algorithm: construction.algorithm_tag(),
                algorithm_params: &algorithm_params_pub(heuristic, construction),
                bound_model_hash: &model.bound_model_hash,
                ccm_hash: &top_level_ccm_hash_hex,
                constraints: &roster,
                construction_wall_time_us: 0,
                emitted_at: "1970-01-01T00:00:00Z",
                node_count: total_node_count,
                partition_manifest: PARTITION_MANIFEST_FILENAME,
                schema_version: CCM_SCHEMA_VERSION,
                var_count: total_var_count as u32,
            })?;
            let partition_manifest_bytes = json_line(&PartitionManifestOut {
                has_bridge,
                partitions: &partition_names,
                schema_version: CCM_PARTITION_MANIFEST_SCHEMA_VERSION,
                top_level_ccm_hash: &top_level_ccm_hash_hex,
            })?;

            Ok((
                top_symbols_json,
                top_manifest_bytes,
                partition_manifest_bytes,
                partition_names.len(),
            ))
        })?;

    stage(&mut t, |s| &mut s.disk_write, || {
        write_file(dir, "ccm.manifest.json", top_manifest_bytes)?;
        write_file(dir, "ccm.symbols.json", top_symbols_json)?;
        write_file(dir, PARTITION_MANIFEST_FILENAME, partition_manifest_bytes)?;
        Ok(())
    })?;

    emit_partition_telemetry(None, partition_count_label, total_node_count);

    Ok(outcome)
}

/// Fold one partition's [`PartitionDigest`] into the emit-wide
/// [`EmitBudgetOutcome`] (configflux-9pjy.4): accumulate the live memo-cap
/// shrink count, track the smallest final cap any partition reached, and
/// raise the cross-partition advisory if this partition's post-build RSS
/// reached the advisory threshold. Pure metadata accounting — it reads only
/// the digest's adaptation/RSS fields and never touches emitted bytes.
fn fold_partition_outcome(
    outcome: &mut EmitBudgetOutcome,
    digest: &PartitionDigest,
    advisory_trigger_kib: Option<u64>,
) {
    let adaptation = digest.adaptation;
    outcome.memo_shrink_count = outcome
        .memo_shrink_count
        .saturating_add(adaptation.shrink_count);
    // Only a partition that actually shrank contributes to the "deepest
    // cap reached" figure; an unshrunk partition's cap is its initial value
    // and is not a budget-driven result.
    if adaptation.shrink_count > 0 {
        outcome.min_final_memo_cap = Some(match outcome.min_final_memo_cap {
            Some(prev) => prev.min(adaptation.final_memo_cap),
            None => adaptation.final_memo_cap,
        });
    }
    if let (Some(trigger), Some(rss)) = (advisory_trigger_kib, digest.post_build_rss_kib) {
        if rss >= trigger {
            outcome.cluster_size_too_large = true;
        }
    }
}

/// Per-partition emission result: raw 32-byte ccm_hash plus the
/// per-partition var/node counts (folded into the top-level
/// aggregate so the top-level manifest reports totals).
///
/// configflux-9pjy.4: also carries the live memo-cap adaptation for this
/// partition and the post-build RSS sample (in KiB, or `None` when no RSS
/// budget was set / `/proc` is unavailable) used for the cross-partition
/// advisory. Both are metadata — they never affect the emitted bytes.
struct PartitionDigest {
    ccm_hash_raw: [u8; 32],
    var_count: u32,
    node_count: u64,
    adaptation: MemoAdaptation,
    post_build_rss_kib: Option<u64>,
}

/// Per-partition apply-progress threading tuple: `(tracker, base, total)`.
/// `base` is the clauses processed in prior partitions; `total` is the
/// model-wide clause count. `None` keeps the byte-identical default path.
type PartitionProgress<'p, 't, 'a> = Option<(&'p mut ProgressTracker<'a>, usize, usize)>;

#[allow(clippy::too_many_arguments)]
fn emit_cluster_partition(
    model: &ConditionModel,
    all_clauses: &[ConditionExpr],
    cluster: &ClusterSpec,
    sub_dir: &Path,
    heuristic: VarOrderHeuristic,
    construction: Construction,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    cluster_index: Option<usize>,
    progress: PartitionProgress,
    t: Option<&mut StageTimings>,
) -> Result<PartitionDigest> {
    let cluster_clauses: Vec<ConditionExpr> = cluster
        .clause_indices
        .iter()
        .map(|&i| all_clauses[i].clone())
        .collect();
    emit_partition_inner(
        model,
        &cluster_clauses,
        sub_dir,
        heuristic,
        construction,
        memo_cap,
        rss_budget_kib,
        cluster_index,
        progress,
        t,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_bridge_partition(
    model: &ConditionModel,
    all_clauses: &[ConditionExpr],
    bridge: &BridgeSpec,
    sub_dir: &Path,
    heuristic: VarOrderHeuristic,
    construction: Construction,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    progress: PartitionProgress,
    t: Option<&mut StageTimings>,
) -> Result<PartitionDigest> {
    let bridge_clauses: Vec<ConditionExpr> = bridge
        .clause_indices
        .iter()
        .map(|&i| all_clauses[i].clone())
        .collect();
    emit_partition_inner(
        model,
        &bridge_clauses,
        sub_dir,
        heuristic,
        construction,
        memo_cap,
        rss_budget_kib,
        // bridge has no cluster index (None telemetry label)
        None,
        progress,
        t,
    )
}

/// Emit one partition's triple (manifest + symbols + bdd) into
/// `sub_dir`, returning the raw ccm_hash + var/node count for the
/// top-level hash chain.
#[allow(clippy::too_many_arguments)]
fn emit_partition_inner(
    model: &ConditionModel,
    sub_clauses: &[ConditionExpr],
    sub_dir: &Path,
    heuristic: VarOrderHeuristic,
    construction: Construction,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    cluster_index: Option<usize>,
    progress: PartitionProgress,
    mut t: Option<&mut StageTimings>,
) -> Result<PartitionDigest> {
    let symbols = stage(&mut t, |s| &mut s.var_order, || {
        Ok(compute_variable_order(sub_clauses, heuristic))
    })?;
    // configflux-9pjy.3: bind this partition's clause-share base offset to
    // the tracker for the apply loop. `None` leaves the apply loop on its
    // exact pre-progress path.
    let mut apply_progress =
        progress.map(|(tracker, base, total)| ApplyProgress::new(tracker, base, total));
    // configflux-9pjy.4: thread the soft RSS budget into the in-crate apply
    // loop (drives the live memo-cap shrink) and capture the per-partition
    // adaptation. `rss_budget_kib = None` keeps the byte-identical path.
    let (bdd_bin, adaptation) = stage(&mut t, |s| &mut s.bdd_apply_loop, || {
        build_bdd_bin_via_construction(
            sub_clauses,
            &symbols,
            construction,
            memo_cap,
            rss_budget_kib,
            apply_progress.as_mut(),
        )
    })?;
    // configflux-9pjy.4 / ADR-0039 §5: sample this partition's post-build
    // RSS for the cross-partition advisory. Only meaningful when an RSS
    // budget is set; skipped otherwise so the unbudgeted path takes no
    // sample. Metadata only — never affects the emitted bytes.
    let post_build_rss_kib = rss_budget_kib.and_then(|_| crate::proc_rss::read_vm_rss_kib());
    let (symbols_json, manifest_json, ccm_hash_raw, var_count, node_count) =
        stage(&mut t, |s| &mut s.manifest_serialize, || {
            let symbols_json = serialize_symbols(&symbols)?;
            let var_count = symbols.len() as u32;
            let node_count = node_count_of(&bdd_bin)?;
            let (manifest_json, ccm_hash_raw) = serialize_per_partition_manifest(
                &model.bound_model_hash,
                var_count,
                node_count,
                &symbols_json,
                &bdd_bin,
                heuristic,
                construction,
            )?;
            Ok((symbols_json, manifest_json, ccm_hash_raw, var_count, node_count))
        })?;

    stage(&mut t, |s| &mut s.disk_write, || {
        std::fs::create_dir_all(sub_dir)
            .with_context(|| format!("Failed to create partition dir '{}'", sub_dir.display()))?;
        write_file(sub_dir, "ccm.bdd.bin", bdd_bin)?;
        write_file(sub_dir, "ccm.symbols.json", symbols_json)?;
        write_file(sub_dir, "ccm.manifest.json", manifest_json)?;
        Ok(())
    })?;

    emit_partition_telemetry(cluster_index, var_count as usize, node_count);

    Ok(PartitionDigest {
        ccm_hash_raw,
        var_count,
        node_count,
        adaptation,
        post_build_rss_kib,
    })
}

// --- canonical JSON structs ---------------------------------------

/// One entry of the ADR-0054 §5.4 constraint roster.
///
/// `root_index` is the constraint's position in the root AND-fold among the
/// authored conjuncts, so a solver core maps back to a constraint id by index
/// (`cfx explain`, configflux-p571.8). Fields are alphabetical, matching every
/// other canonical-JSON struct in this module.
#[derive(Serialize)]
pub(crate) struct ConstraintRosterEntry<'a> {
    pub(crate) condition: &'a str,
    pub(crate) id: &'a str,
    pub(crate) root_index: u32,
}

/// Pre-image of the top-level manifest. ADR-0005 §5 Step 3: elides
/// `ccm_hash`, `construction_wall_time_us`, and `emitted_at`.
///
/// ADR-0054 §5.4: the roster is INSIDE the pre-image, so declaring a
/// constraint rotates `ccm_hash`. It is skipped when empty, which keeps a
/// model that declares no constraint byte-identical to the pre-ADR-0054
/// artifact — the rotation is scoped to models that actually gained policy.
#[derive(Serialize)]
struct TopLevelPreimage<'a> {
    algorithm: &'a str,
    algorithm_params: &'a BTreeMap<String, String>,
    bound_model_hash: &'a str,
    #[serde(skip_serializing_if = "<[ConstraintRosterEntry]>::is_empty")]
    constraints: &'a [ConstraintRosterEntry<'a>],
    node_count: u64,
    partition_manifest: &'a str,
    schema_version: u32,
    var_count: u32,
}

/// Final top-level `<out>/ccm/ccm.manifest.json` output. Carries the
/// v2 `partition_manifest` field and the ADR-0054 §5.4 constraint
/// roster; otherwise identical shape to the per-partition manifest.
///
/// The roster is model-global and lives ONLY here — per-partition manifests
/// (`serialize_per_partition_manifest`) are unchanged, which keeps the
/// partitioning scheme out of the constraint-identity decision entirely.
/// Synthesized cardinality conjuncts are deliberately absent from the roster.
#[derive(Serialize)]
struct TopLevelManifestOut<'a> {
    algorithm: &'a str,
    algorithm_params: &'a BTreeMap<String, String>,
    bound_model_hash: &'a str,
    ccm_hash: &'a str,
    #[serde(skip_serializing_if = "<[ConstraintRosterEntry]>::is_empty")]
    constraints: &'a [ConstraintRosterEntry<'a>],
    construction_wall_time_us: u64,
    emitted_at: &'a str,
    node_count: u64,
    partition_manifest: &'a str,
    schema_version: u32,
    var_count: u32,
}

/// Pre-image of `partition-manifest.json` (ADR-0012 §5 Step 2).
/// `top_level_ccm_hash` is elided because the value is what's being
/// computed.
#[derive(Serialize)]
struct PartitionManifestPreimage<'a> {
    has_bridge: bool,
    partitions: &'a [String],
    schema_version: u32,
}

/// Final `partition-manifest.json` output (ADR-0005 Amendment 1 §13).
#[derive(Serialize)]
struct PartitionManifestOut<'a> {
    has_bridge: bool,
    partitions: &'a [String],
    schema_version: u32,
    top_level_ccm_hash: &'a str,
}

// --- helpers ------------------------------------------------------

fn write_file(dir: &Path, name: &str, bytes: Vec<u8>) -> Result<()> {
    let path = dir.join(name);
    std::fs::write(&path, bytes)
        .with_context(|| format!("Failed to write '{}'", path.display()))
}

fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// Sibling test module is declared from the parent (`ccm_emitter.rs`):
//   #[cfg(test)] mod multi_part_tests;
// per the codebase convention used by `bdd.rs` + `tests.rs` and
// `partitioner.rs` + `partitioner_tests.rs`.

// configflux-9pjy.4: inline child test module for the soft-budget
// accounting (`fold_partition_outcome`) — it touches `multi_part`'s private
// `PartitionDigest` / `EmitBudgetOutcome` internals, so it lives here rather
// than in the sibling test files (which only see the `ccm_emitter` surface).
#[cfg(test)]
mod budget_tests {
    use super::*;

    fn digest_with(adaptation: MemoAdaptation, post_build_rss_kib: Option<u64>) -> PartitionDigest {
        PartitionDigest {
            ccm_hash_raw: [0u8; 32],
            var_count: 0,
            node_count: 0,
            adaptation,
            post_build_rss_kib,
        }
    }

    #[test]
    fn advisory_fires_only_when_post_build_rss_reaches_threshold() {
        // ADR-0039 §5 (AC2): the cross-partition advisory is raised only
        // when a partition's post-build RSS reaches 0.85 × the budget.
        // budget = 1000 KiB ⇒ trigger = 850 KiB.
        let trigger = Some(1000u64 * ADVISORY_TRIGGER_NUM / ADVISORY_TRIGGER_DEN);

        // Under threshold: no advisory.
        let mut under = EmitBudgetOutcome::default();
        fold_partition_outcome(
            &mut under,
            &digest_with(MemoAdaptation::default(), Some(800)),
            trigger,
        );
        assert!(
            !under.cluster_size_too_large,
            "an under-threshold partition must not raise the advisory"
        );

        // At/over threshold: advisory fires.
        let mut over = EmitBudgetOutcome::default();
        fold_partition_outcome(
            &mut over,
            &digest_with(MemoAdaptation::default(), Some(900)),
            trigger,
        );
        assert!(
            over.cluster_size_too_large,
            "an over-threshold partition must raise the advisory"
        );
    }

    #[test]
    fn no_advisory_without_an_rss_budget() {
        // With no advisory trigger (no RSS budget), no post-build sample is
        // taken and the advisory never fires regardless of partition state.
        let mut outcome = EmitBudgetOutcome::default();
        fold_partition_outcome(
            &mut outcome,
            &digest_with(MemoAdaptation::default(), None),
            None,
        );
        assert!(!outcome.cluster_size_too_large);
    }

    #[test]
    fn shrink_counts_aggregate_and_min_final_cap_tracks_deepest() {
        // The emit-wide outcome sums per-partition shrink counts and tracks
        // the smallest final cap any partition reached; unshrunk partitions
        // do not contribute a final-cap figure.
        let mut outcome = EmitBudgetOutcome::default();
        fold_partition_outcome(
            &mut outcome,
            &digest_with(
                MemoAdaptation {
                    shrink_count: 2,
                    initial_memo_cap: 1 << 16,
                    final_memo_cap: 1 << 14,
                },
                None,
            ),
            None,
        );
        fold_partition_outcome(
            &mut outcome,
            &digest_with(
                MemoAdaptation {
                    shrink_count: 3,
                    initial_memo_cap: 1 << 16,
                    final_memo_cap: 1 << 13,
                },
                None,
            ),
            None,
        );
        // An unshrunk partition: contributes 0 to the count and nothing to
        // the min-final-cap.
        fold_partition_outcome(
            &mut outcome,
            &digest_with(
                MemoAdaptation {
                    shrink_count: 0,
                    initial_memo_cap: 1 << 20,
                    final_memo_cap: 1 << 20,
                },
                None,
            ),
            None,
        );
        assert_eq!(outcome.memo_shrink_count, 5);
        assert_eq!(outcome.min_final_memo_cap, Some(1 << 13));
    }
}
