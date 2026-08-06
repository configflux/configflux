// SPDX-License-Identifier: BUSL-1.1

mod bdd;
// configflux-vmlb: the rung-3 scope partitioner is now wired into the
// production v2 multi-part emission pipeline below
// (see `emit_ccm_dir_inner` → `multi_part::emit_multi_part`). The
// transitional `#[allow(dead_code)]` allowance from configflux-0qo3
// is removed because every public(crate) item is now called from
// `multi_part.rs`.
mod multi_part;
#[cfg(test)]
mod multi_part_more_tests;
#[cfg(test)]
mod multi_part_tests;
mod partitioner;
#[cfg(test)]
mod partitioner_tests;
#[cfg(test)]
mod tests;
mod timings;
mod var_order;

// configflux-lz70: re-export ConditionExpr at the ccm_emitter boundary so
// the rung-3 scope partitioner (configflux-0qo3, blocked-by this issue) can
// name the return type of `parse_condition_model` via
// `ccm_emitter::ConditionExpr` without reaching across into the private
// `conditions` module. Crate-internal only; not part of the public API.
pub(crate) use crate::conditions::ConditionExpr;
use crate::conditions::{parse_condition_expr, ConditionPredicate, ConditionPredicateOp};
// configflux-9pjy.3 / ADR-0039 §7: the compile-time progress signal. The
// in-crate apply loop reports against `ApplyProgress` at the existing
// `clear_memos` boundary; `emit_ccm_dir_with_progress` threads the
// `ProgressTracker`. `None`/absent keeps the byte-identical default path.
use crate::progress::{ApplyProgress, ProgressTracker, PROGRESS_CLAUSE_INTERVAL};
use anyhow::{bail, Context, Result};
use bdd::{node_count, sha256, BddBuilder, FALSE_REF, TRUE_REF};
// configflux-9pjy.4 / ADR-0039 §5: the live adaptive memo-cap shrink
// record flows up the emit chain so the compile summary can report it.
pub(crate) use bdd::MemoAdaptation;
// configflux-9pjy.4: the aggregated soft-budget emit outcome (adaptation +
// cross-partition advisory) the dir-emit entry points return to
// `compiler_core` → `product_api` for the compile summary.
pub(crate) use multi_part::EmitBudgetOutcome;
// configflux-9pjy.2 / ADR-0039: the soft-budget derivation
// (`resource_budget::derive_knobs`) anchors its memo-table byte model to
// the same default cap the in-crate `BddBuilder` uses, so re-export it at
// the crate-internal `ccm_emitter` boundary rather than reaching into the
// private `bdd` submodule from `resource_budget.rs`.
pub(crate) use bdd::DEFAULT_MEMO_CAP;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use timings::stage;
pub use timings::StageTimings;
use var_order::{compute_variable_order, VarOrderHeuristic};

// configflux-vmlb / ADR-0012 §4 + ADR-0005 Amendment 1 §15:
// schema version 1 → 2, domain tag v1 → v2. Solver-side bump lands
// in configflux-mwyp; fixture rotation lands in configflux-4fu0.
pub(crate) const CCM_SCHEMA_VERSION: u32 = 2;
pub(crate) const CCM_SYMBOLS_SCHEMA_VERSION: u32 = 2;
pub(crate) const CCM_HASH_DOMAIN_TAG: &[u8] = b"configflux.ccm.v2\n";
/// `partition-manifest.json` schema version. Starts at 2 because the
/// file did not exist at v1 (ADR-0005 Amendment 1 §13).
pub(crate) const CCM_PARTITION_MANIFEST_SCHEMA_VERSION: u32 = 2;

/// The emitter's input: everything that determines the compiled BDD root.
///
/// ADR-0054 §5.1 splits the input by ROLE, and the split is the point. The
/// defect class configflux-9xxq belongs to happened because a *branch selector*
/// reached the BDD root as a *global assertion*. Naming the channels apart is
/// what makes the invariant reviewable: a future change that wants to assert a
/// new policy has to add it to a field whose name says `constraints`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionModel {
    pub bound_model_hash: String,
    /// Symbol-universe contributors.
    ///
    /// The compiler's producer (`compiler_core::collect_ccm_clauses`) emits
    /// **only** symbol-introducing tautologies here (`f == 'v' || f != 'v'`),
    /// which `compile_expr` lowers to the canonical `TRUE` terminal — so the
    /// AND-fold is the identity `and(root, TRUE) = root` and an authored
    /// `condition` has no path to the root. Selector conditions and declared
    /// facet values both arrive through this channel (ADR-0047 §4 Amendment 1,
    /// ADR-0054 §5.1); it exists so their symbols reach the variable order and
    /// `ccm.symbols.json` without constraining anything.
    ///
    /// The channel itself stays general: the FAMA/SPLOT fixture generators feed
    /// externally-authored feature-model clauses through it, where the fold is
    /// their whole semantics. Only the compiler's producer guarantees
    /// tautologies, and `compiler_core_tests` pins that guarantee.
    pub clauses: Vec<String>,
    /// Authored root conjuncts: `(constraint_id, condition_text)`, id-ascending.
    ///
    /// The ONLY channel that carries authored policy into the root, and the
    /// only one that appears in the §5.4 manifest roster. Folded after
    /// [`Self::clauses`], in the order given.
    pub constraints: Vec<(String, String)>,
    /// Synthesized intra-facet cardinality conjuncts (ADR-0054 §5.2).
    ///
    /// Folded AFTER every authored constraint, in facet-name-ascending then
    /// declared-value order. Asserted exactly like a constraint but deliberately
    /// NOT rostered (§5.4): a core that reduces to a cardinality clause means the
    /// *model* is over-constrained, not that a user violated a named policy.
    pub cardinality: Vec<String>,
}

impl ConditionModel {
    /// A model whose entire content is symbol-universe clauses: no authored
    /// constraints, no synthesized cardinality.
    ///
    /// This is the shape the FAMA/SPLOT/synthetic fixture generators build —
    /// externally authored feature models that have no ConfigFlux
    /// `constraints` namespace, and whose clauses are their whole semantics.
    /// A real product compile goes through `compiler_core`, which populates
    /// every field explicitly.
    pub fn from_clauses(bound_model_hash: String, clauses: Vec<String>) -> Self {
        Self {
            bound_model_hash,
            clauses,
            constraints: Vec::new(),
            cardinality: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcmEmission {
    pub manifest_json: Vec<u8>,
    pub symbols_json: Vec<u8>,
    pub bdd_bin: Vec<u8>,
}

/// BDD construction strategy selector — configflux-wbzw / ADR-0011.
///
/// Default is [`Construction::InCrate`] (the hand-rolled
/// `compiler::ccm_emitter::bdd::BddBuilder`, tag
/// `robdd-handrolled-v1`). [`Construction::Cudd`] is the opt-in
/// CUDD-backed compiler build path defined by ADR-0011 §1 and
/// Amendment 1 §A1.4, tag `robdd-cudd-v1`. The two paths produce the
/// same Boolean function (same valid_options, same satisfying
/// assignments) but emit different `ccm.bdd.bin` bytes — per-backend
/// byte stability is the contract; cross-backend byte equality is
/// not provided (ADR-0005 §6 + ADR-0011 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construction {
    /// Hand-rolled in-crate `BddBuilder` (default).
    InCrate,
    /// CUDD-backed compiler build path (configflux-wbzw).
    Cudd,
}

impl Construction {
    /// Wire tag string for the manifest's `algorithm` field per
    /// ADR-0005 §2 + ADR-0011 Amendment 1 §A1.1.
    fn algorithm_tag(self) -> &'static str {
        match self {
            Construction::InCrate => "robdd-handrolled-v1",
            Construction::Cudd => "robdd-cudd-v1",
        }
    }
}

fn parse_construction_tag(tag: &str) -> Result<Construction> {
    match tag {
        "in-crate" => Ok(Construction::InCrate),
        "cudd" => Ok(Construction::Cudd),
        other => bail!("unknown --construction value '{other}'; expected 'in-crate' or 'cudd'"),
    }
}

// configflux-4jmc: fail-closed message for the `Construction::Cudd` arm in the
// lean (no-`cudd`-feature) build. The CUDD construction path is gated out, so
// both dispatch sites bail with this single shared message.
#[cfg(not(feature = "cudd"))]
const CUDD_PATH_UNAVAILABLE: &str = "CUDD construction path is unavailable in \
    this build: the compiler crate was built without the `cudd` feature \
    (configflux-4jmc). Rebuild against //compiler:compiler_lib_cudd \
    (crate_features=[\"cudd\"]) to use --construction=cudd";

pub fn build_ccm_artifact(model: &ConditionModel) -> Result<CcmEmission> {
    build_ccm_artifact_inner(
        model,
        VarOrderHeuristic::FacetNameAscending,
        Construction::InCrate,
        None,
    )
}

/// Profiled variant of [`build_ccm_artifact`]. Returns the same
/// `CcmEmission` plus a per-stage wall-clock breakdown. Byte-identity
/// against the legacy entry point is pinned by
/// `tests::with_timings_byte_identical_to_legacy`.
pub fn build_ccm_artifact_with_timings(
    model: &ConditionModel,
) -> Result<(CcmEmission, StageTimings)> {
    let mut t = StageTimings::default();
    let e = build_ccm_artifact_inner(
        model,
        VarOrderHeuristic::FacetNameAscending,
        Construction::InCrate,
        Some(&mut t),
    )?;
    Ok((e, t))
}

/// Build a CCM artifact under the named variable-order heuristic.
/// `heuristic_tag` accepts the same string values that appear in
/// `ccm.manifest.json.algorithm_params.var_order_heuristic` per
/// ADR-0005 §2 (`"facet-name-ascending"` or `"clause-grouped-dfs"`);
/// any other value is rejected. Existing callers stay on
/// [`build_ccm_artifact`], which always uses
/// `"facet-name-ascending"`, and observe no behaviour change.
pub fn build_ccm_artifact_with_heuristic(
    model: &ConditionModel,
    heuristic_tag: &str,
) -> Result<CcmEmission> {
    build_ccm_artifact_inner(
        model,
        parse_heuristic_tag(heuristic_tag)?,
        Construction::InCrate,
        None,
    )
}

/// Build a CCM artifact under the named variable-order heuristic AND
/// the named construction strategy — configflux-wbzw / ADR-0011.
/// `construction_tag` accepts `"in-crate"` or `"cudd"`. Existing
/// callers stay on [`build_ccm_artifact_with_heuristic`] (which
/// always uses in-crate) and observe no behaviour change.
pub fn build_ccm_artifact_with_construction(
    model: &ConditionModel,
    heuristic_tag: &str,
    construction_tag: &str,
) -> Result<CcmEmission> {
    build_ccm_artifact_inner(
        model,
        parse_heuristic_tag(heuristic_tag)?,
        parse_construction_tag(construction_tag)?,
        None,
    )
}

fn build_ccm_artifact_inner(
    model: &ConditionModel,
    heuristic: VarOrderHeuristic,
    construction: Construction,
    mut t: Option<&mut StageTimings>,
) -> Result<CcmEmission> {
    let parsed = stage(&mut t, |s| &mut s.clause_parse, || {
        parse_condition_model(model)
    })?;
    let symbols = stage(&mut t, |s| &mut s.var_order, || {
        Ok(compute_variable_order(&parsed, heuristic))
    })?;
    let bdd_bin = stage(&mut t, |s| &mut s.bdd_apply_loop, || match construction {
        Construction::InCrate => build_bdd_bin(&parsed, &symbols),
        // Delegated to the single-importer module per ADR-0011 §2.
        // Note `ccm_emitter` itself never references `cudd_sys` —
        // the LGPL boundary is preserved at the file level.
        // configflux-4jmc: gated behind the `cudd` feature; the lean default
        // build fails closed (CUDD_PATH_UNAVAILABLE) instead of linking CUDD.
        // configflux-xh57: no budget in scope here, so the CUDD cache stays
        // uncapped (`None`) — byte-identical; bd follow-up tracks budgeting it.
        #[cfg(feature = "cudd")]
        Construction::Cudd => crate::cudd_build::build_bdd_bin_via_cudd(&parsed, &symbols, None),
        #[cfg(not(feature = "cudd"))]
        Construction::Cudd => bail!(CUDD_PATH_UNAVAILABLE),
    })?;
    let (symbols_json, manifest_json) = stage(&mut t, |s| &mut s.manifest_serialize, || {
        let symbols_json = serialize_symbols(&symbols)?;
        let manifest_json = serialize_manifest(
            &model.bound_model_hash,
            symbols.len() as u32,
            node_count(&bdd_bin)? as u64,
            &symbols_json,
            &bdd_bin,
            heuristic,
            construction,
        )?;
        Ok((symbols_json, manifest_json))
    })?;
    Ok(CcmEmission {
        manifest_json,
        symbols_json,
        bdd_bin,
    })
}

pub fn emit_ccm(model: &ConditionModel, output: &mut impl Write) -> Result<()> {
    output.write_all(&build_ccm_artifact(model)?.bdd_bin)?;
    Ok(())
}

pub fn emit_ccm_dir(model: &ConditionModel, dir: impl AsRef<Path>) -> Result<()> {
    emit_ccm_dir_inner(
        model,
        dir,
        VarOrderHeuristic::FacetNameAscending,
        Construction::InCrate,
        usize::MAX,
        None,
        None,
        None,
    )
    .map(|_| ())
}

/// Like [`emit_ccm_dir`], but writes the artifact under the named
/// variable-order heuristic. See
/// [`build_ccm_artifact_with_heuristic`] for the accepted strings.
pub fn emit_ccm_dir_with_heuristic(
    model: &ConditionModel,
    dir: impl AsRef<Path>,
    heuristic_tag: &str,
) -> Result<()> {
    emit_ccm_dir_inner(
        model,
        dir,
        parse_heuristic_tag(heuristic_tag)?,
        Construction::InCrate,
        usize::MAX,
        None,
        None,
        None,
    )
    .map(|_| ())
}

/// Like [`emit_ccm_dir_with_heuristic`], but also selects the BDD
/// construction strategy — configflux-wbzw / ADR-0011.
/// `construction_tag` accepts `"in-crate"` or `"cudd"`. Existing
/// callers stay on [`emit_ccm_dir_with_heuristic`] (in-crate) and
/// observe no behaviour change.
pub fn emit_ccm_dir_with_construction(
    model: &ConditionModel,
    dir: impl AsRef<Path>,
    heuristic_tag: &str,
    construction_tag: &str,
) -> Result<()> {
    emit_ccm_dir_inner(
        model,
        dir,
        parse_heuristic_tag(heuristic_tag)?,
        parse_construction_tag(construction_tag)?,
        usize::MAX,
        None,
        None,
        None,
    )
    .map(|_| ())
}

/// Like [`emit_ccm_dir_with_construction`], plus an explicit
/// `cluster_size` upper bound that feeds the rung-3 scope partitioner
/// (configflux-vmlb / ADR-0012 §2). `cluster_size = usize::MAX`
/// collapses to a single partition (single-partition v2 layout, no
/// bridge) — every FAMA fixture trivially takes this branch.
pub fn emit_ccm_dir_with_cluster_size(
    model: &ConditionModel,
    dir: impl AsRef<Path>,
    heuristic_tag: &str,
    construction_tag: &str,
    cluster_size: usize,
) -> Result<()> {
    emit_ccm_dir_inner(
        model,
        dir,
        parse_heuristic_tag(heuristic_tag)?,
        parse_construction_tag(construction_tag)?,
        cluster_size,
        None,
        None,
        None,
    )
    .map(|_| ())
}

/// Like [`emit_ccm_dir_with_cluster_size`], but also threads a soft
/// resource budget (configflux-9pjy.2 / ADR-0039). The budget derives
/// an apply-memo cap (a byte-neutral cache lever) and, when the
/// projected unique table would exceed the budget, a `cluster_size`.
/// Per ADR-0012 Amendment 1 the explicit `cluster_size` argument here
/// **always wins**: callers compute `request.cluster_size.or(derived
/// .cluster_size)` before calling, so this function receives the already-
/// resolved effective value. `memo_cap = None` keeps the byte-identical
/// default apply-memo path; `Some(cap)` bounds the in-crate memos.
///
/// configflux-9pjy.4 / ADR-0039 §5: `rss_budget_kib` (the soft RSS budget
/// in KiB) drives the live adaptive memo-cap shrink, and the returned
/// [`EmitBudgetOutcome`] surfaces the adaptation + the cross-partition
/// advisory for the compile summary. `rss_budget_kib = None` keeps the
/// byte-identical path and returns the zeroed outcome.
// configflux-9pjy.4: `pub(crate)` because it returns the crate-internal
// `EmitBudgetOutcome` and is only consumed by `compiler_core`; the
// externally-public emit surface stays `emit_ccm_dir*` (which return `()`).
pub(crate) fn emit_ccm_dir_with_budget(
    model: &ConditionModel,
    dir: impl AsRef<Path>,
    heuristic_tag: &str,
    construction_tag: &str,
    cluster_size: usize,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
) -> Result<EmitBudgetOutcome> {
    emit_ccm_dir_inner(
        model,
        dir,
        parse_heuristic_tag(heuristic_tag)?,
        parse_construction_tag(construction_tag)?,
        cluster_size,
        memo_cap,
        rss_budget_kib,
        None,
    )
}

/// Like [`emit_ccm_dir_with_budget`], but also threads a compile-time
/// progress tracker (configflux-9pjy.3 / ADR-0039 §7). The tracker emits
/// the VarOrder → BddApplyLoop band to its sink as the per-partition apply
/// loops run; the terminal Serialize completion (pct 1.0) is the caller's
/// responsibility via [`crate::progress::ProgressTracker::finish`].
/// Progress is a SEPARATE stream and never enters the artifact bytes
/// (ADR-0005 Amendment 2); passing a [`crate::progress::NullSink`]-backed
/// tracker is byte-identical to today.
///
/// configflux-9pjy.4: `rss_budget_kib` drives the live memo-cap shrink and
/// the returned [`EmitBudgetOutcome`] carries the adaptation + advisory.
/// `pub(crate)` for the same reason as [`emit_ccm_dir_with_budget`] — it
/// returns the crate-internal outcome and is only consumed by
/// `compiler_core`.
pub(crate) fn emit_ccm_dir_with_progress(
    model: &ConditionModel,
    dir: impl AsRef<Path>,
    heuristic_tag: &str,
    construction_tag: &str,
    cluster_size: usize,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    progress: &mut ProgressTracker,
) -> Result<EmitBudgetOutcome> {
    let dir = dir.as_ref();
    multi_part::emit_multi_part_with_progress(
        model,
        dir,
        parse_heuristic_tag(heuristic_tag)?,
        parse_construction_tag(construction_tag)?,
        cluster_size,
        memo_cap,
        rss_budget_kib,
        Some(progress),
        None,
    )
}

/// Profiled variant of [`emit_ccm_dir_with_heuristic`]. Returns the
/// per-stage wall-clock breakdown including disk-write time. The
/// breakdown is stderr/log-only data — it is NEVER written into the
/// emitted ccm.* files (ADR-0005 §6 byte-stability).
pub fn emit_ccm_dir_with_timings(
    model: &ConditionModel,
    dir: impl AsRef<Path>,
    heuristic_tag: &str,
) -> Result<StageTimings> {
    let mut t = StageTimings::default();
    emit_ccm_dir_inner(
        model,
        dir,
        parse_heuristic_tag(heuristic_tag)?,
        Construction::InCrate,
        usize::MAX,
        None,
        None,
        Some(&mut t),
    )?;
    Ok(t)
}

#[allow(clippy::too_many_arguments)]
fn emit_ccm_dir_inner(
    model: &ConditionModel,
    dir: impl AsRef<Path>,
    heuristic: VarOrderHeuristic,
    construction: Construction,
    cluster_size: usize,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    t: Option<&mut StageTimings>,
) -> Result<EmitBudgetOutcome> {
    let dir = dir.as_ref();
    // configflux-vmlb / ADR-0012 §4 + ADR-0005 Amendment 1 §11:
    // v2 wire format is ALWAYS multi-part. Single-partition models
    // (cluster_size = usize::MAX or var_count <= cluster_size) still
    // produce one `partition-0000/` subdirectory plus a top-level
    // `partition-manifest.json`. configflux-mwyp: stage timings now
    // thread end-to-end through `multi_part::emit_multi_part` so the
    // per-stage breakdown matches the in-memory `build_ccm_artifact`
    // shape — `clause_parse`, `var_order`, `bdd_apply_loop`,
    // `manifest_serialize`, `disk_write` all carry nonzero
    // durations on the dir-emit path, restoring the contract the
    // `tools/gen_synthetic --profile` consumer relies on.
    // configflux-9pjy.4: `rss_budget_kib` threads the soft RSS budget into
    // the apply loop (live memo-cap shrink) and the returned outcome carries
    // the adaptation + advisory; `None` keeps the byte-identical path.
    multi_part::emit_multi_part(
        model,
        dir,
        heuristic,
        construction,
        cluster_size,
        memo_cap,
        rss_budget_kib,
        t,
    )
}

fn parse_heuristic_tag(tag: &str) -> Result<VarOrderHeuristic> {
    match tag {
        "facet-name-ascending" => Ok(VarOrderHeuristic::FacetNameAscending),
        "clause-grouped-dfs" => Ok(VarOrderHeuristic::ClauseGroupedDfs),
        // configflux-pqi4 (rung 2.5a): FORCE static variable ordering
        // per ADR-0005 §2 value-namespace extension. Opt-in only;
        // default remains 'facet-name-ascending'.
        "force" => Ok(VarOrderHeuristic::Force),
        other => bail!(
            "unknown var_order_heuristic '{other}'; expected 'facet-name-ascending', 'clause-grouped-dfs', or 'force'"
        ),
    }
}

/// Parse a [`ConditionModel`] into its in-memory clause tree
/// representation, performing the same `bound_model_hash` validation
/// the full build path performs. Returns the same
/// `Vec<ConditionExpr>` that [`build_ccm_artifact_inner`] would feed
/// into `compute_variable_order` and the BDD builders.
///
/// configflux-lz70: lifted out of [`build_ccm_artifact_inner`] so the
/// rung-3 scope partitioner (configflux-0qo3, the next sub-issue in
/// the keg4 chain) can walk the parsed `ConditionExpr` trees and
/// build the variable-connectivity bipartite graph WITHOUT paying for
/// a full BDD construction. Behaviour is byte-identical to the
/// previous inline path: the full build still calls this function and
/// then proceeds with variable ordering + BDD construction unchanged.
///
/// Crate-internal: callers outside the `compiler` crate continue to
/// go through the `build_ccm_artifact*` / `emit_ccm_dir*` public
/// surface.
pub(crate) fn parse_condition_model(model: &ConditionModel) -> Result<Vec<ConditionExpr>> {
    validate_hash(&model.bound_model_hash)?;
    // ADR-0054 §5.1/§5.2 fold order, and it is load-bearing for the §5.4
    // roster: symbol-universe clauses first (each lowers to `TRUE`, so the
    // fold is the identity), then the authored constraints in the
    // id-ascending order the caller supplied, then the synthesized
    // cardinality conjuncts. A constraint's `root_index` in the manifest
    // roster is its position in `model.constraints`, so the constraint block
    // must stay contiguous and in caller order.
    let mut exprs = parse_clauses(&model.clauses)?;
    exprs.reserve(model.constraints.len() + model.cardinality.len());
    for (id, condition) in &model.constraints {
        exprs.push(
            parse_condition_expr(condition)
                .with_context(|| format!("constraint '{id}': {condition}"))?,
        );
    }
    for clause in &model.cardinality {
        exprs.push(
            parse_condition_expr(clause)
                .with_context(|| format!("synthesized cardinality clause: {clause}"))?,
        );
    }
    Ok(exprs)
}

pub(crate) fn parse_clauses(clauses: &[String]) -> Result<Vec<ConditionExpr>> {
    clauses
        .iter()
        .map(|clause| parse_condition_expr(clause).with_context(|| format!("clause: {clause}")))
        .collect()
}

fn build_bdd_bin(expressions: &[ConditionExpr], symbols: &[(String, String)]) -> Result<Vec<u8>> {
    build_bdd_bin_with_memo_cap(expressions, symbols, None)
}

/// Build the in-crate BDD with an optional apply-memo cap
/// (configflux-9pjy.2 / ADR-0039). `None` constructs
/// `BddBuilder::default()` exactly as before — byte-identical to the
/// unbudgeted path — and `Some(cap)` constructs
/// `BddBuilder::with_memo_cap(cap)`. The memo cap is a pure cache lever
/// (the memos are caches, not the canonical `unique` table), so it
/// changes only time and cache churn, never the emitted bytes
/// (ADR-0039 §8; pinned by the byte-stability tests in `tests.rs`).
fn build_bdd_bin_with_memo_cap(
    expressions: &[ConditionExpr],
    symbols: &[(String, String)],
    memo_cap: Option<usize>,
) -> Result<Vec<u8>> {
    let mut builder = match memo_cap {
        Some(cap) => BddBuilder::with_memo_cap(cap),
        None => BddBuilder::default(),
    };
    build_bdd_bin_with_builder_progress(expressions, symbols, &mut builder, None)
}

/// As [`build_bdd_bin_with_memo_cap`], plus the optional apply-loop
/// progress handle (configflux-9pjy.3) and the optional soft RSS budget
/// that drives the live adaptive memo-cap shrink (configflux-9pjy.4 /
/// ADR-0039 §5). Returns the emitted bytes plus the post-build
/// [`MemoAdaptation`] (zeroed when no shrink fired). `rss_budget_kib`
/// is the soft target peak in **KiB** (the `proc_rss` unit); `None`
/// disables sampling and keeps the byte-identical default path.
fn build_bdd_bin_with_memo_cap_rss_progress(
    expressions: &[ConditionExpr],
    symbols: &[(String, String)],
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    progress: Option<&mut ApplyProgress>,
) -> Result<(Vec<u8>, MemoAdaptation)> {
    // When neither a cap nor an RSS budget is set, this is exactly the
    // historical `BddBuilder::default()` build (byte-identical). The RSS
    // budget alone (cap `None`) starts from `DEFAULT_MEMO_CAP` and lets
    // the live shrink take it down.
    let mut builder = match (memo_cap, rss_budget_kib) {
        (None, None) => BddBuilder::default(),
        (Some(cap), None) => BddBuilder::with_memo_cap(cap),
        (cap, budget) => {
            BddBuilder::with_memo_cap_and_rss_budget(cap.unwrap_or(DEFAULT_MEMO_CAP), budget)
        }
    };
    let bytes = build_bdd_bin_with_builder_progress(expressions, symbols, &mut builder, progress)?;
    Ok((bytes, builder.adaptation()))
}

/// Run the in-crate apply loop on a borrowed builder and return the
/// serialized BDD bytes. Borrowing (rather than consuming) the builder
/// lets callers read its post-build [`BddBuilder::adaptation`] — the live
/// memo-cap shrink record (configflux-9pjy.4) — after this returns.
///
/// configflux-9pjy.3 / ADR-0039 §7: the optional progress handle is
/// invoked at the **existing** `clear_memos` clause boundary — the single
/// apply-loop sampling site (no second sampling site is added) — at the
/// bounded [`PROGRESS_CLAUSE_INTERVAL`] cadence, and again once at the end
/// so the partition always reports its final clause.
///
/// Byte-stability (ADR-0005 Amendment 2 / ADR-0039 §8): when neither
/// progress nor an RSS budget is active, the loop is the exact
/// pre-adaptive code path — the same `builder.and` / `clear_memos`
/// sequence in the same order. The progress `on_clear` call and the
/// in-`clear_memos` RSS sample/shrink are observational with respect to
/// the emitted bytes: they only resize caches, never `unique`/`nodes`.
fn build_bdd_bin_with_builder_progress(
    expressions: &[ConditionExpr],
    symbols: &[(String, String)],
    builder: &mut BddBuilder,
    mut progress: Option<&mut ApplyProgress>,
) -> Result<Vec<u8>> {
    let index: BTreeMap<String, u32> = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (symbol_name(tag, value), i as u32))
        .collect();
    let mut root = TRUE_REF;
    let total = expressions.len();
    for (processed, expr) in expressions.iter().enumerate() {
        let clause = compile_expr(builder, expr, &index)?;
        root = builder.and(root, clause);
        // bd-93oj: drop the apply memos at each clause boundary so the
        // 10k+cross-tree workload stays under the 4 GB envelope. Per
        // ADR-0005 §6 byte-stability, the memos are caches (a miss
        // recomputes the same answer through `apply`), so this clear
        // is semantically a no-op on output bytes. The canonical
        // `unique` table is left intact — it MUST persist for ROBDD
        // canonicity. configflux-9pjy.4: `clear_memos` also samples RSS
        // and shrinks the live cap when over the soft budget (no-op when
        // unbudgeted) — likewise byte-neutral.
        builder.clear_memos();
        // configflux-9pjy.3: report progress at this same boundary on the
        // bounded cadence, and always on the final clause. Observational
        // only — does not perturb the builder or output bytes.
        if let Some(ap) = progress.as_deref_mut() {
            let done = processed + 1;
            if done % PROGRESS_CLAUSE_INTERVAL == 0 || done == total {
                ap.on_clear(done);
            }
        }
    }
    // configflux-d49v: emit a single BDD-PROFILE block on stderr if
    // CONFIGFLUX_BDD_BUILDER_PROFILE=1. Always called; no-op when
    // the env var is unset, so byte-stable behaviour is unchanged.
    builder.dump_profile();
    Ok(builder.serialize(root, symbols.len() as u32))
}

fn compile_expr(
    builder: &mut BddBuilder,
    expr: &ConditionExpr,
    index: &BTreeMap<String, u32>,
) -> Result<u32> {
    Ok(match expr {
        ConditionExpr::Bool(value) => {
            if *value {
                TRUE_REF
            } else {
                FALSE_REF
            }
        }
        ConditionExpr::Predicate(predicate) => compile_predicate(builder, predicate, index)?,
        ConditionExpr::Not(inner) => {
            let compiled = compile_expr(builder, inner, index)?;
            builder.not(compiled)
        }
        ConditionExpr::And(left, right) => {
            let l = compile_expr(builder, left, index)?;
            let r = compile_expr(builder, right, index)?;
            builder.and(l, r)
        }
        ConditionExpr::Or(left, right) => {
            let l = compile_expr(builder, left, index)?;
            let r = compile_expr(builder, right, index)?;
            builder.or(l, r)
        }
        // `any_of(e_1, …, e_N)` — OR-reduction (ADR-0006 §4). Lower each
        // child in ascending `Vec` index order, then fold with `builder.or`
        // strictly left-associatively starting from `c_1` (ADR-0006 §5,
        // the pinned fold direction). For `N = 1` this is `c_1` unchanged.
        // Empty lists are rejected by the parser (configflux-ccs.3, ADR-0006
        // §2 "N ≥ 1"); the fold below also degrades to terminal-false for an
        // empty `Vec` rather than panicking, but that path is unreachable
        // through the parser.
        ConditionExpr::AnyOf(args) => {
            let mut acc: Option<u32> = None;
            for child in args {
                let c = compile_expr(builder, child, index)?;
                acc = Some(match acc {
                    None => c,
                    Some(prev) => builder.or(prev, c),
                });
            }
            acc.unwrap_or(FALSE_REF)
        }
        // `all_of(e_1, …, e_N)` — AND-reduction (ADR-0006 §4). Same
        // accumulator shape as `AnyOf` above but folding with `builder.and`:
        // lower each child in ascending `Vec` index order, strictly
        // left-associatively from `c_1` (ADR-0006 §5, pinned fold direction).
        // `N = 1` yields `c_1`. Empty lists are parser-rejected
        // (configflux-ccs.3); the `None` fallback degrades to terminal-true
        // (AND identity) and is unreachable through the parser.
        ConditionExpr::AllOf(args) => {
            let mut acc: Option<u32> = None;
            for child in args {
                let c = compile_expr(builder, child, index)?;
                acc = Some(match acc {
                    None => c,
                    Some(prev) => builder.and(prev, c),
                });
            }
            acc.unwrap_or(TRUE_REF)
        }
        // `exactly_one_of(e_1, …, e_N)` — at-least-one (OR) ∧ pairwise
        // at-most-one (AMO) (ADR-0006 §4, configflux-ccs.6). Logical meaning:
        // exactly one child holds. Lowered as the conjunction of two parts,
        // built in the ADR-0006 §5 pinned order so the emitted `.ccm` bytes
        // stay reproducible across rebuilds and across the two backends:
        //
        //   1. Lower each child `e_i` to `c_i` ONCE, in ascending `Vec` index
        //      order. Reusing the same node refs in both parts below is sound
        //      because the builder memoizes (re-lowering returns the same ref)
        //      and matches ADR-0006 §4 ("let c_1 … c_N be the references
        //      obtained by lowering each child expression").
        //   2. At-least-one (ALO): `c_1 ∨ … ∨ c_N`, an OR-fold strictly
        //      left-associative from `c_1` in ascending index order — identical
        //      to the `AnyOf` reduction above (ADR-0006 §5).
        //   3. At-most-one (AMO), pairwise: for every unordered pair `i < j`
        //      enumerated in ascending `(i, j)` lexicographic order, the mutex
        //      clause `¬(c_i ∧ c_j)`, AND-folded left-associatively over the
        //      `C(N, 2)` pairs in that same enumeration order (ADR-0006 §5).
        //      AMO is the AND-identity `TRUE_REF` when there are no pairs.
        //   4. Result is `ALO ∧ AMO`. For `N = 1` AMO is vacuously true and
        //      `and(c_1, TRUE_REF)` reduces to `c_1` (matches ADR-0006 §4);
        //      for `N = 2` this is `(c_1 ∨ c_2) ∧ ¬(c_1 ∧ c_2)`, i.e. XOR.
        //
        // Pairwise AMO is O(N²) and uses NO auxiliary variables (every BDD
        // variable here is a real `(tag, value)` facet — there is no aux-var
        // space; ADR-0006 §4). The practical author-facing bound is N ≤ 16,
        // which caps AMO at C(16, 2) = 120 mutex clauses — trivial for the
        // apply engine. N > 16 emits a compile-time WARNING (not an error):
        // the encoding still works and stays byte-stable; the warning flags
        // that a larger list almost certainly wants a different modeling
        // primitive (e.g. an enumerated facet). Empty lists are rejected by
        // the parser (configflux-ccs.3); the `None`/`TRUE_REF` fallbacks below
        // are unreachable through the parser. This is the last cardinality
        // stub — after this no `ConditionExpr` arm errors in `compile_expr`.
        ConditionExpr::ExactlyOneOf(args) => {
            warn_if_exceeds_pairwise_amo_bound(args.len());
            let children: Vec<u32> = args
                .iter()
                .map(|child| compile_expr(builder, child, index))
                .collect::<Result<_>>()?;
            // ALO: OR-fold ascending, left-associative from c_1.
            let mut alo: Option<u32> = None;
            for &c in &children {
                alo = Some(match alo {
                    None => c,
                    Some(prev) => builder.or(prev, c),
                });
            }
            let alo = alo.unwrap_or(FALSE_REF);
            // AMO: pairwise mutex, ascending (i, j), AND-folded left-assoc.
            let mut amo: Option<u32> = None;
            for i in 0..children.len() {
                for j in (i + 1)..children.len() {
                    let pair_and = builder.and(children[i], children[j]);
                    let mutex = builder.not(pair_and);
                    amo = Some(match amo {
                        None => mutex,
                        Some(prev) => builder.and(prev, mutex),
                    });
                }
            }
            let amo = amo.unwrap_or(TRUE_REF);
            builder.and(alo, amo)
        }
    })
}

fn compile_predicate(
    builder: &mut BddBuilder,
    predicate: &ConditionPredicate,
    index: &BTreeMap<String, u32>,
) -> Result<u32> {
    let name = symbol_name(&predicate.tag, &predicate.value);
    let var = *index
        .get(&name)
        .with_context(|| format!("missing symbol index for {name}"))?;
    let node = builder.var(var);
    Ok(match predicate.op {
        ConditionPredicateOp::Eq => node,
        ConditionPredicateOp::NotEq => builder.not(node),
    })
}

pub(crate) fn serialize_symbols(symbols: &[(String, String)]) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct SymbolsOut {
        facet_to_var: BTreeMap<String, u32>,
        schema_version: u32,
        var_to_label: Vec<String>,
        variable_order: Vec<String>,
    }
    let variable_order: Vec<String> = symbols.iter().map(|(t, v)| symbol_name(t, v)).collect();
    let facet_to_var = variable_order
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), i as u32))
        .collect();
    json_line(&SymbolsOut {
        facet_to_var,
        schema_version: CCM_SYMBOLS_SCHEMA_VERSION,
        var_to_label: symbols.iter().map(|(t, v)| format!("{t}={v}")).collect(),
        variable_order,
    })
}

/// Serialise a per-partition `ccm.manifest.json` (the byte-stable v1
/// recipe, re-tagged under the v2 `CCM_HASH_DOMAIN_TAG` /
/// `CCM_SCHEMA_VERSION`). Returns canonical JSON bytes plus the raw
/// 32-byte ccm_hash so the top-level chain in `multi_part.rs` can
/// fold it in (ADR-0012 §5 Step 1).
///
/// The top-level `<out>/ccm/ccm.manifest.json` has its own serializer
/// in [`multi_part`] — it carries the v2 `partition_manifest` field
/// that the per-partition shape does not.
pub(crate) fn serialize_per_partition_manifest(
    bound_model_hash: &str,
    var_count: u32,
    node_count: u64,
    symbols_json: &[u8],
    bdd_bin: &[u8],
    heuristic: VarOrderHeuristic,
    construction: Construction,
) -> Result<(Vec<u8>, [u8; 32])> {
    let algorithm = construction.algorithm_tag();
    let params = algorithm_params(heuristic, construction);
    let pre = ManifestPreimage {
        algorithm,
        algorithm_params: &params,
        bound_model_hash,
        node_count,
        schema_version: CCM_SCHEMA_VERSION,
        var_count,
    };
    let manifest_canon = serde_json::to_vec(&pre)?;
    let mut hasher = Sha256::new();
    hasher.update(CCM_HASH_DOMAIN_TAG);
    hasher.update(&manifest_canon);
    hasher.update(b"\n");
    hasher.update(sha256(bdd_bin));
    hasher.update(sha256(symbols_json));
    let raw: [u8; 32] = hasher.finalize().into();
    let ccm_hash_hex = hex32(&raw);
    let bytes = json_line(&ManifestOut {
        algorithm,
        algorithm_params: &params,
        bound_model_hash,
        ccm_hash: &ccm_hash_hex,
        construction_wall_time_us: 0,
        emitted_at: "1970-01-01T00:00:00Z",
        node_count,
        schema_version: CCM_SCHEMA_VERSION,
        var_count,
    })?;
    Ok((bytes, raw))
}

fn serialize_manifest(
    bound_model_hash: &str,
    var_count: u32,
    node_count: u64,
    symbols_json: &[u8],
    bdd_bin: &[u8],
    heuristic: VarOrderHeuristic,
    construction: Construction,
) -> Result<Vec<u8>> {
    let (bytes, _hash_raw) = serialize_per_partition_manifest(
        bound_model_hash,
        var_count,
        node_count,
        symbols_json,
        bdd_bin,
        heuristic,
        construction,
    )?;
    Ok(bytes)
}

#[derive(Serialize)]
struct ManifestPreimage<'a> {
    algorithm: &'a str,
    algorithm_params: &'a BTreeMap<String, String>,
    bound_model_hash: &'a str,
    node_count: u64,
    schema_version: u32,
    var_count: u32,
}

#[derive(Serialize)]
struct ManifestOut<'a> {
    algorithm: &'a str,
    algorithm_params: &'a BTreeMap<String, String>,
    bound_model_hash: &'a str,
    ccm_hash: &'a str,
    construction_wall_time_us: u64,
    emitted_at: &'a str,
    node_count: u64,
    schema_version: u32,
    var_count: u32,
}

/// Expose `algorithm_params` to the sibling `multi_part` module so
/// the top-level manifest's serialization matches the per-partition
/// shape exactly (the partition_manifest field is the ONLY
/// difference per ADR-0005 Amendment 1 §12).
pub(crate) fn algorithm_params_pub(
    heuristic: VarOrderHeuristic,
    construction: Construction,
) -> BTreeMap<String, String> {
    algorithm_params(heuristic, construction)
}

fn algorithm_params(
    heuristic: VarOrderHeuristic,
    construction: Construction,
) -> BTreeMap<String, String> {
    // ADR-0011 Amendment 1 §A1.4 (configflux-ew24, configflux-3t3z):
    // - In-crate path (`robdd-handrolled-v1`) emits
    //   `apply_cache=hashmap-default`, `manager=in-crate-v1`,
    //   `reorder=static`, `threads=1`. `hashmap-default` names the
    //   `std::collections::HashMap`-backed apply memos
    //   (`not_memo`/`and_memo`/`or_memo`) in
    //   `ccm_emitter/bdd.rs::BddBuilder`, with the "clear-when-full"
    //   eviction policy bounded by `DEFAULT_MEMO_CAP`. `in-crate-v1`
    //   names the in-crate `BddBuilder` manager-equivalent (a single
    //   `Vec<RawNode>` + `HashMap` unique table) and is versioned in
    //   the configflux schema rather than against an external library.
    //   Rotated accurate-values under configflux-3t3z (separate
    //   audit-clean event after the configflux-ew24 algorithm-tag
    //   rotation).
    // - CUDD path (`robdd-cudd-v1`) emits
    //   `apply_cache=cudd-default`, `manager=cudd-3.0`, `reorder=static`,
    //   `threads=1` (configflux-wbzw).
    let (apply_cache, manager) = match construction {
        Construction::InCrate => ("hashmap-default", "in-crate-v1"),
        Construction::Cudd => ("cudd-default", "cudd-3.0"),
    };
    BTreeMap::from([
        ("apply_cache".to_string(), apply_cache.to_string()),
        ("manager".to_string(), manager.to_string()),
        ("reorder".to_string(), "static".to_string()),
        ("threads".to_string(), "1".to_string()),
        (
            "var_order_heuristic".to_string(),
            heuristic.manifest_tag().to_string(),
        ),
    ])
}

pub(crate) fn json_line<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Construction-dispatching BDD builder: invokes the in-crate
/// `BddBuilder` or the CUDD-side builder (per `Construction`) on the
/// supplied clauses and variable order. Used by
/// `multi_part::emit_multi_part` to build each per-partition BDD.
///
/// Knobs (configflux-9pjy.2 added `memo_cap`; configflux-9pjy.3 added the
/// optional `progress` handle; configflux-9pjy.4 added `rss_budget_kib`,
/// the soft RSS budget in **KiB** that drives the live adaptive memo-cap
/// shrink, ADR-0039 §5). All three bite only on the in-crate apply loop
/// (the single-threaded recursion whose `clear_memos` boundary is the
/// sampling site, ADR-0039 §7); the CUDD path manages its own apply cache
/// and emits its own env-gated `CUDD-CHECKPOINT:` telemetry, so it ignores
/// them. `memo_cap`/`rss_budget_kib`/`progress` all `None` keeps the
/// byte-identical default path on both arms.
///
/// Returns the emitted bytes plus the post-build [`MemoAdaptation`] — the
/// live shrink record (zeroed on the CUDD arm and whenever no shrink
/// fired), surfaced up the chain for the compile summary.
pub(crate) fn build_bdd_bin_via_construction(
    expressions: &[ConditionExpr],
    symbols: &[(String, String)],
    construction: Construction,
    memo_cap: Option<usize>,
    rss_budget_kib: Option<u64>,
    progress: Option<&mut ApplyProgress>,
) -> Result<(Vec<u8>, MemoAdaptation)> {
    match construction {
        // configflux-9pjy.2 / ADR-0039: the soft-budget memo cap is an
        // in-crate-only lever — it bounds the hand-rolled `BddBuilder`'s
        // apply memos. `None` keeps the byte-identical default path.
        // configflux-9pjy.3: the apply-loop progress handle threads in on
        // the same in-crate arm. configflux-9pjy.4: so does the RSS budget
        // that drives the live shrink.
        Construction::InCrate => build_bdd_bin_with_memo_cap_rss_progress(
            expressions,
            symbols,
            memo_cap,
            rss_budget_kib,
            progress,
        ),
        // configflux-4jmc: gated behind the `cudd` feature; lean build fails
        // closed (CUDD_PATH_UNAVAILABLE). configflux-xh57 / ADR-0039
        // Amendment 1: the CUDD path runs its OWN computed cache, so
        // `memo_cap` is forwarded to `build_bdd_bin_via_cudd` → a byte-neutral
        // `Cudd_SetMaxCacheHard` bound (`None` = byte-identical default). The
        // RSS shrink (`rss_budget_kib`) and `progress` stay in-crate-only
        // (ADR-0039 §5/§7); the adaptation record here is the zeroed default.
        #[cfg(feature = "cudd")]
        Construction::Cudd => {
            let _ = (progress, rss_budget_kib);
            let bytes = crate::cudd_build::build_bdd_bin_via_cudd(expressions, symbols, memo_cap)?;
            Ok((bytes, MemoAdaptation::default()))
        }
        #[cfg(not(feature = "cudd"))]
        Construction::Cudd => {
            let _ = (progress, rss_budget_kib);
            bail!(CUDD_PATH_UNAVAILABLE)
        }
    }
}

/// Public(crate) accessor for the BDD's serialized node count. The
/// `bdd::node_count` function is module-private; this wrapper exposes
/// it under a `pub(crate)` name used by `multi_part.rs`.
pub(crate) fn node_count_of(bdd_bin: &[u8]) -> Result<u64> {
    Ok(node_count(bdd_bin)? as u64)
}

/// Count the model's distinct `(tag, value)` BDD variables — the
/// soft-budget `total_vars_hint` (configflux-9pjy.2 / ADR-0039 §3).
///
/// Parses the model's clauses (same path the emit chain uses) and counts
/// the distinct variables the partitioner would see, so the budget's
/// projected unique-table footprint and the eventual partition layout
/// agree. A model whose clauses fail to parse yields `0` (no projection
/// possible); the caller treats that as "no hint" and preserves
/// single-partition collapse. Pure: no env, no clock.
pub(crate) fn count_model_variables(model: &ConditionModel) -> usize {
    match parse_condition_model(model) {
        Ok(parsed) => partitioner::count_distinct_variables(&parsed),
        Err(_) => 0,
    }
}


fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("bound_model_hash must be 64-character lowercase SHA-256 hex");
    }
    Ok(())
}

fn symbol_name(tag: &str, value: &str) -> String {
    format!("{tag}.{value}")
}

/// Practical upper bound on the number of children in an
/// `exactly_one_of(...)` list before the pairwise at-most-one (AMO)
/// encoding is flagged (configflux-ccs.6, ADR-0006 §4).
///
/// Pairwise AMO emits `C(N, 2) = N(N−1)/2` mutex clauses with NO
/// auxiliary variables — the `.ccm` variable space models only real
/// `(tag, value)` facets, so a log/commander encoding (which needs aux
/// vars) is not available without changing the byte-layout contract.
/// At `N = 16` that is 120 mutex clauses, trivial for the apply engine
/// and a generous ceiling for author-facing cardinality lists (expected
/// to be a handful of mutually-exclusive variants).
///
/// configflux-ccs.6 surfaces `N > 16` as a compile-time **warning**, not
/// an error: the pairwise encoding still produces a correct, byte-stable
/// BDD past 16 children; the warning exists to nudge authors toward a
/// better-suited primitive (e.g. an enumerated facet) before the O(N²)
/// clause count grows large.
pub(crate) const EXACTLY_ONE_OF_PAIRWISE_AMO_BOUND: usize = 16;

/// Emit a single stderr warning when an `exactly_one_of(...)` list has
/// more than [`EXACTLY_ONE_OF_PAIRWISE_AMO_BOUND`] children
/// (configflux-ccs.6, ADR-0006 §4). The warning is advisory only — it
/// does NOT fail the build and does NOT touch the emitted artifact, so
/// byte-stability is unaffected (the message goes to stderr, never into
/// any `ccm.*` file; cf. the ADR-0005 §6 stderr-only profile blocks).
fn warn_if_exceeds_pairwise_amo_bound(child_count: usize) {
    if child_count > EXACTLY_ONE_OF_PAIRWISE_AMO_BOUND {
        eprintln!(
            "warning: exactly_one_of over {child_count} children exceeds the \
             pairwise-AMO bound of {EXACTLY_ONE_OF_PAIRWISE_AMO_BOUND}; the \
             encoding emits C(N, 2) = {} mutex clauses (O(N²)). This still \
             compiles to a byte-stable BDD, but a list this large almost \
             certainly wants a different modeling primitive (e.g. an \
             enumerated facet). See ADR-0006 §4.",
            child_count * (child_count - 1) / 2
        );
    }
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

