// SPDX-License-Identifier: BUSL-1.1
//
// CUDD-side BDD construction path for the compiler (configflux-wbzw,
// implementing ADR-0011 + Amendment 1).
//
// # Purpose
//
// This module is the second compiler-side BDD construction path. The
// default path is `compiler::ccm_emitter::bdd::BddBuilder` (hand-rolled,
// algorithm tag `robdd-handrolled-v1`). This module is the opt-in
// `--construction=cudd` path that delegates BDD apply / unique-table /
// variable management to libcudd, emitting the same on-disk byte layout
// per ADR-0005 §4 but tagged `robdd-cudd-v1` per ADR-0005 §2.
//
// Per ADR-0011 Amendment 1 §A1.3, this is exactly **one** file under
// `compiler/` that imports `cudd_sys`. The single-importer review rule
// applies: any future file added here that also imports `cudd_sys` is
// reviewed as a co-change with this one. Currently `compiler/src/`
// contains a single such file (this one); ripgrep `use cudd_sys`
// repo-wide should return three paths after this lands:
//   - solver/src/backend_cudd.rs
//   - solver/src/cudd_translate.rs
//   - compiler/src/cudd_build/mod.rs
//
// # Algorithm
//
// 1. Initialise a fresh CUDD manager (`Cudd_Init` with documented
//    defaults). **Disable dynamic reordering immediately** via
//    `Cudd_AutodynDisable` — sifting OFF is mandatory for byte
//    stability per ADR-0005 §6 G1. A sifted variable order would
//    diverge from the static `compute_variable_order` decision encoded
//    in `ccm.symbols.json` and break the cross-backend semantic-parity
//    contract.
//
// 2. Materialise per-variable literals via `Cudd_bddIthVar(i)` for
//    `i` in `0..var_count`. Each literal is `Cudd_Ref`'d immediately.
//
// 3. Compile each `ConditionExpr` recursively into a CUDD `*mut
//    DdNode`, using `Cudd_bddAnd` / `Cudd_bddOr` / `Cudd_Not` over the
//    pre-allocated literals. Each intermediate result is `Cudd_Ref`'d.
//
// 4. AND-fold the clause results into a single root. The
//    `Cudd_bddAnd` apply cache amortises the per-op cost.
//
// 5. Walk the root via complement-edge-erasing post-order DFS
//    (cribbed from `solver/src/cudd_translate.rs::cudd_to_canonical` —
//    intentional copy per architect critique on configflux-wbzw; do
//    NOT extract a shared crate).
//
// 6. Encode the resulting node table into the ADR-0005 §4 byte layout
//    (cribbed from `compiler/src/ccm_emitter/bdd.rs::{serialize,
//    push_node, ordered_nodes}` — intentional copy of ~50 LOC, also
//    per architect critique).
//
// 7. `Cudd_RecursiveDeref` every intermediate BEFORE drop of the
//    root, then deref the root, then `Cudd_Quit` the manager.
//
// # Memory and thread-safety discipline
//
// - Every materialised `*mut DdNode` is `Cudd_Ref`'d on push and
//   `Cudd_RecursiveDeref`'d before manager teardown. Intermediates are
//   deref'd before drop of the root (debug builds of CUDD assert on
//   unbalanced refcounts at `Cudd_Quit`).
//
// - `BackendError::OutOfCapacity`-equivalent (here surfaced as
//   `anyhow!("CUDD apply returned NULL")`) is the response to any
//   `Cudd_bddAnd` / `Cudd_bddOr` NULL return. The compiler's caller
//   handles this as a build failure.
//
// - The transient `CuddBuilder` struct holds the manager pointer; it
//   is `unsafe impl Send` (not `Sync`) so a future thread-pool caller
//   can hand it across thread boundaries, mirroring
//   `solver/src/backend_cudd.rs:143`. CUDD's own data structures are
//   not internally synchronised; concurrent access would corrupt the
//   unique table.
//
// # LGPL / boundary preservation
//
// This file is BUSL-1.1 (the whole repository is BUSL-1.1 per
// ADR-0046); under `compiler/` it is the single-importer module
// ADR-0011 §4 pinned. It links the BSD-3-Clause `cudd_sys` crate
// (vendored under `third_party/cudd-sys/`) and the vendored
// BSD-3-Clause CUDD 3.0.0 C source. No license change is implied. The
// `ccm_emitter` directory MUST NOT grow a `cudd_sys` import — that is
// the file-level LGPL boundary the architect pinned.

use core::ffi::c_uint;
use std::collections::{BTreeMap, HashMap};
use std::ptr::NonNull;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use cudd_sys::cudd::{
    Cudd_AutodynDisable, Cudd_E, Cudd_Init, Cudd_IsComplement, Cudd_IsConstant, Cudd_NodeReadIndex,
    Cudd_Not, Cudd_Quit, Cudd_ReadDead, Cudd_ReadKeys, Cudd_ReadLogicZero, Cudd_ReadNodeCount,
    Cudd_ReadOne, Cudd_ReadPeakLiveNodeCount, Cudd_ReadPeakNodeCount, Cudd_RecursiveDeref, Cudd_Ref,
    Cudd_Regular, Cudd_SetMaxCacheHard, Cudd_T, Cudd_bddAnd, Cudd_bddIthVar, Cudd_bddOr,
    CUDD_CACHE_SLOTS, CUDD_UNIQUE_SLOTS,
};
// Test-only readback (configflux-xh57): test-gated to avoid an
// unused-import warning in the production build.
#[cfg(test)]
use cudd_sys::cudd::Cudd_ReadMaxCacheHard;
use cudd_sys::DdManager;
use cudd_sys::DdNode;

use crate::conditions::{ConditionExpr, ConditionPredicate, ConditionPredicateOp};
use crate::resource_budget::MEMO_CAP_FLOOR;

mod checkpoint;
use checkpoint::{emit_if_enabled, CheckpointSnapshot};

// ---------------------------------------------------------------------
// ADR-0005 §4 byte-format constants (intentional duplication of the
// constants in `compiler/src/ccm_emitter/bdd.rs`. Per architect critique
// on configflux-wbzw, the ~50 LOC of post-ADR-0005-stable code is
// accepted as duplication; extracting a shared codec is premature.)
// ---------------------------------------------------------------------

const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

// ---------------------------------------------------------------------
// CUDD manager init defaults — pinned literally so a silent CUDD source
// bump cannot change them. Mirrors solver/src/backend_cudd.rs:73-86.
// ---------------------------------------------------------------------

const DEFAULT_UNIQUE_SLOTS: c_uint = CUDD_UNIQUE_SLOTS;
const DEFAULT_CACHE_SLOTS: c_uint = CUDD_CACHE_SLOTS;
const DEFAULT_MAX_MEMORY: usize = 0;

// Soft resource budget → CUDD apply-cache cap (configflux-xh57,
// ADR-0039 Amendment 1). The in-crate `BddBuilder` bounds its per-table
// `memo_cap`; the CUDD path runs its own unified computed cache, capped
// here via `Cudd_SetMaxCacheHard` (a soft entry cap — not
// `Cudd_SetMaxMemory`, which would return NULL on breach and turn the
// soft budget into a hard build failure). Byte-neutral: the computed
// cache is a pure memo whose canonicity is owned by the unique table, so
// capping it changes only wall-clock, never the emitted bytes, provided
// reordering stays disabled (`Cudd_AutodynDisable`). See ADR-0039
// Amendment 1 §3 for the full source-cited byte-stability argument.

/// In-crate apply-memo table count (`not_memo`/`and_memo`/`or_memo`) the
/// per-table `memo_cap` covers; CUDD's single unified cache gets the
/// equivalent total `memo_cap × 3`.
const IN_CRATE_MEMO_TABLE_COUNT: usize = 3;

/// Floor on the derived CUDD `maxCacheHard` (entries). Non-zero is
/// required: `Cudd_SetMaxCacheHard(dd, 0)` is CUDD's "derive default"
/// sentinel (would discard the budget), and a tiny cache thrashes.
/// Anchored to the in-crate `MEMO_CAP_FLOOR` scaled by the table count.
pub(crate) const CUDD_CACHE_CAP_FLOOR: u32 = (MEMO_CAP_FLOOR * IN_CRATE_MEMO_TABLE_COUNT) as u32;

/// Map the derived per-table `memo_cap` to a CUDD computed-cache hard cap
/// (entries). `None` ⇒ `None` (CUDD default, byte-identical unbudgeted
/// path). `Some(cap)` ⇒ `Some(cap × 3)`, floored at
/// [`CUDD_CACHE_CAP_FLOOR`] and `u32`-saturated (the in-crate cap is
/// bounded by `DEFAULT_MEMO_CAP = 1 << 20`, so `× 3` stays ≪ `u32::MAX`).
/// Pure and deterministic.
pub(crate) fn derive_cudd_cache_cap(memo_cap: Option<usize>) -> Option<u32> {
    memo_cap.map(|cap| {
        let scaled = cap.saturating_mul(IN_CRATE_MEMO_TABLE_COUNT);
        u32::try_from(scaled).unwrap_or(u32::MAX).max(CUDD_CACHE_CAP_FLOOR)
    })
}

/// Per-symbol index for compiling `ConditionPredicate`s into BDD literals.
/// The key is the joined `tag.value` symbol name (matches
/// `compiler::ccm_emitter::symbol_name`); the value is the BDD variable
/// index assigned by the variable-order heuristic.
type SymbolIndex<'a> = &'a BTreeMap<String, u32>;

/// Compile a list of `ConditionExpr` clauses into the on-disk
/// `ccm.bdd.bin` byte stream using the CUDD construction path.
///
/// `symbols` is the output of `compute_variable_order`: a
/// `Vec<(tag, value)>` whose index becomes the BDD variable index.
/// The CUDD path honours this order exactly — no autodyn reordering
/// (see §G1 in ADR-0005 §6 referenced in the call to
/// `Cudd_AutodynDisable` below).
///
/// Returns the same byte layout as
/// `compiler::ccm_emitter::bdd::BddBuilder::serialize`. The two paths
/// are not guaranteed byte-identical (different reduce/canonisation
/// internals produce different node orderings — ADR-0011 §3 and
/// ADR-0004 dwwv §3 addendum); they ARE guaranteed semantically
/// equivalent (same valid_options, same satisfying assignments).
pub(crate) fn build_bdd_bin_via_cudd(
    expressions: &[ConditionExpr],
    symbols: &[(String, String)],
    memo_cap: Option<usize>,
) -> Result<Vec<u8>> {
    // Build the (tag.value -> var_index) lookup the same way
    // `ccm_emitter::build_bdd_bin_with_builder` does.
    let index: BTreeMap<String, u32> = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (format!("{tag}.{value}"), i as u32))
        .collect();

    // configflux-xh57: map the derived per-table `memo_cap` to a CUDD
    // computed-cache hard cap (applied in `CuddBuilder::new`). `None`
    // keeps CUDD's default; byte-neutral either way.
    let cudd_cache_cap = derive_cudd_cache_cap(memo_cap);

    let var_count = symbols.len() as u32;
    let mut builder = CuddBuilder::new(var_count, cudd_cache_cap)
        .context("initialise CUDD manager for CUDD-path BDD construction")?;

    // configflux-vfx4 telemetry: wall-clock the AND-fold loop and the
    // serialize pass when `CONFIGFLUX_CUDD_BUILDER_PROFILE` is set.
    // Unconditional `Instant::now()` reads are sub-microsecond on Linux
    // and add nothing measurable to a 10k × 50/50 build that takes
    // hundreds of seconds. The block is the CUDD-path analogue of
    // `BddBuilder::dump_profile` (configflux-d49v), but the metric is
    // CUDD's native `Cudd_ReadPeakNodeCount` / `Cudd_ReadKeys` rather
    // than the in-crate `unique.len()` — CUDD owns its unique table.
    let apply_start = Instant::now();

    // AND-fold all expressions. CUDD manages its apply cache
    // internally (no `clear_memos`-style reset needed); per-clause
    // `maybe_checkpoint` emits CUDD-CHECKPOINT for the kill-survivor
    // trace (configflux-8l3b).
    let mut root = builder.constant_true();
    for (clause_index, expr) in expressions.iter().enumerate() {
        let clause = builder.compile_expr(expr, &index)?;
        let next = builder.and(root, clause)?;
        // Defer-deref: drop the previous root and the clause node
        // BEFORE overwriting `root` so peak refcount stays bounded.
        builder.deref(root);
        builder.deref(clause);
        root = next;
        builder.maybe_checkpoint(clause_index as u64 + 1, apply_start);
    }

    let apply_wall = apply_start.elapsed();
    let serialize_start = Instant::now();
    let bytes = builder.serialize(root, var_count)?;
    let serialize_wall = serialize_start.elapsed();

    // configflux-vfx4: emit the CUDD-side profile block before
    // teardown (Cudd_Quit invalidates the manager pointer). Gated by
    // the same env-var convention as the in-crate BDD-PROFILE block.
    builder.dump_profile(
        expressions.len() as u64,
        apply_wall,
        serialize_wall,
        bytes.len() as u64,
    );

    // Deref the root before manager teardown (the Drop impl on
    // CuddBuilder dereffs intermediates and the per-variable literals,
    // then calls Cudd_Quit).
    builder.deref(root);
    Ok(bytes)
}

/// Transient holder for the CUDD manager and the per-build refcounted
/// node set. Lifetime equals one `build_bdd_bin_via_cudd` call.
///
/// `Cudd_Ref` discipline: every `*mut DdNode` materialised inside
/// `compile_*` and `and`/`or`/`not` is reffed on creation. The caller
/// (the `build_bdd_bin_via_cudd` driver) is responsible for
/// `Cudd_RecursiveDeref`'ing intermediates BEFORE the final root is
/// passed to `serialize`. The `Drop` impl handles the per-variable
/// literals + any straggler refs by walking `extra_refs` then calling
/// `Cudd_Quit`.
struct CuddBuilder {
    manager: NonNull<DdManager>,
    /// Per-variable literal nodes, indexed by BDD variable index.
    /// Each entry is `Cudd_Ref`'d on push and `Cudd_RecursiveDeref`'d
    /// in `Drop`. Matches the discipline in
    /// `solver/src/backend_cudd.rs::var_functions`.
    var_functions: Vec<NonNull<DdNode>>,
    /// Stragglers — nodes whose ref must be released at manager
    /// teardown because the caller forgot to deref them. Currently
    /// empty in the happy path (the driver dereffes its own
    /// intermediates), but populated by error-path `compile_expr`
    /// helpers so a build that bails mid-clause does not leak.
    extra_refs: Vec<NonNull<DdNode>>,
}

// SAFETY: CuddBuilder owns its `*mut DdManager` exclusively. CUDD's
// own data structures are not internally synchronised; the caller MUST
// NOT pass a CuddBuilder across thread boundaries while holding handles
// to its nodes. Sync is intentionally NOT implemented (matches
// solver/src/backend_cudd.rs:143).
unsafe impl Send for CuddBuilder {}

impl CuddBuilder {
    /// Initialise a fresh CUDD manager with `var_count` variables
    /// pre-allocated. Dynamic reordering is explicitly disabled.
    ///
    /// `cudd_cache_cap` (configflux-xh57 / ADR-0039 Amendment 1) is the
    /// soft budget's derived computed-cache hard cap, in entries. `None`
    /// leaves CUDD's default (byte-identical unbudgeted build); a `Some`
    /// value is applied via `Cudd_SetMaxCacheHard` — byte-neutral, see
    /// the module-level `derive_cudd_cache_cap` doc.
    fn new(var_count: u32, cudd_cache_cap: Option<u32>) -> Result<Self> {
        // SAFETY: `Cudd_Init` is a documented FFI entry point that
        // either returns a valid pointer or NULL on allocation
        // failure. We materialise the NonNull invariant immediately.
        let manager_ptr = unsafe {
            Cudd_Init(
                0,
                0,
                DEFAULT_UNIQUE_SLOTS,
                DEFAULT_CACHE_SLOTS,
                DEFAULT_MAX_MEMORY,
            )
        };
        let manager = NonNull::new(manager_ptr)
            .ok_or_else(|| anyhow!("Cudd_Init returned NULL (allocation failure)"))?;

        // ADR-0005 §6 G1 (byte-stability): disable dynamic reordering
        // unconditionally. Sifting would permute the variable order
        // away from the static `compute_variable_order` decision
        // encoded in `ccm.symbols.json` and break the per-backend
        // byte-stability contract. CUDD's default is off, but a
        // future linked-against build that ships with autodyn-on as
        // a compile-time toggle would silently change semantics;
        // this call pins the contract regardless of the upstream
        // default. Mirrors solver/src/backend_cudd.rs:249.
        //
        // SAFETY: `manager` is freshly-initialised and non-null.
        unsafe { Cudd_AutodynDisable(manager.as_ptr()) };

        // configflux-xh57: apply the soft budget's derived cache cap, when
        // set. AFTER `Cudd_AutodynDisable` (load-bearing for byte-stability)
        // and BEFORE any apply. `None` skips the call (CUDD default). The
        // cap only bounds cache growth; the unique table is untouched.
        //
        // SAFETY: `manager` is live and non-null; `Cudd_SetMaxCacheHard`
        // mutates a single scalar bound field on the manager.
        if let Some(cap) = cudd_cache_cap {
            unsafe { Cudd_SetMaxCacheHard(manager.as_ptr(), cap) };
        }

        // Pre-allocate the per-variable literals. `Cudd_bddIthVar`
        // is the standard CUDD entry point for materialising a
        // positive literal `x_i = 1`; the returned pointer is owned
        // by the manager but we take an explicit `Cudd_Ref` so the
        // GC inside CUDD's unique table cannot reclaim it under us.
        let mut var_functions: Vec<NonNull<DdNode>> = Vec::with_capacity(var_count as usize);
        for i in 0..var_count {
            // SAFETY: manager is live; `Cudd_bddIthVar` returns NULL
            // only on allocation failure.
            let var_ptr = unsafe { Cudd_bddIthVar(manager.as_ptr(), i as core::ffi::c_int) };
            let var = NonNull::new(var_ptr).ok_or_else(|| {
                // SAFETY: we own the refs taken so far in this loop.
                unsafe {
                    for v in &var_functions {
                        Cudd_RecursiveDeref(manager.as_ptr(), v.as_ptr());
                    }
                    Cudd_Quit(manager.as_ptr());
                }
                anyhow!("Cudd_bddIthVar returned NULL at index {i}")
            })?;
            // SAFETY: `var` is a valid CUDD-managed node.
            unsafe { Cudd_Ref(var.as_ptr()) };
            var_functions.push(var);
        }

        Ok(Self {
            manager,
            var_functions,
            extra_refs: Vec::new(),
        })
    }

    /// Emit a structured `CUDD-PROFILE:` block on stderr if the env
    /// var `CONFIGFLUX_CUDD_BUILDER_PROFILE` is set
    /// (configflux-vfx4). Called once at end of build, before
    /// `Cudd_Quit` — the manager pointer is required to be live.
    /// Mirrors `BddBuilder::dump_profile` (configflux-d49v) in shape;
    /// the "peak unique.len()" analogue for the CUDD path is
    /// `Cudd_ReadPeakNodeCount`, since CUDD owns its unique table.
    ///
    /// Output is for human + grep'able log inspection only and is
    /// never parsed into byte-stable artifacts (ADR-0005 §6).
    fn dump_profile(
        &self,
        clause_count: u64,
        apply_wall: std::time::Duration,
        serialize_wall: std::time::Duration,
        bdd_bin_bytes: u64,
    ) {
        if std::env::var_os("CONFIGFLUX_CUDD_BUILDER_PROFILE").is_none() {
            return;
        }
        // SAFETY: manager is live (called before `Cudd_Quit`).
        // `Cudd_ReadPeakNodeCount` returns a `c_long`; `Cudd_ReadKeys`
        // / `Cudd_ReadNodeCount` / `Cudd_ReadPeakLiveNodeCount` are
        // documented read-only inspectors that never NULL on a live
        // manager.
        let (peak_nodes, current_nodes, current_keys, peak_live_nodes) = unsafe {
            (
                Cudd_ReadPeakNodeCount(self.manager.as_ptr()),
                Cudd_ReadNodeCount(self.manager.as_ptr()),
                Cudd_ReadKeys(self.manager.as_ptr()),
                Cudd_ReadPeakLiveNodeCount(self.manager.as_ptr()),
            )
        };
        eprintln!(
            "CUDD-PROFILE:\n  \
             peak_nodes            = {peak_nodes} (CUDD unique-table peak; analogue of in-crate unique.len())\n  \
             peak_live_nodes       = {peak_live_nodes} (live, reffed nodes at peak)\n  \
             current_nodes         = {current_nodes} (at end-of-build, before serialize)\n  \
             current_keys          = {current_keys} (active CUDD unique-table keys)\n  \
             clause_count          = {clause_count}\n  \
             apply_wall_s          = {:.3}\n  \
             serialize_wall_s      = {:.3}\n  \
             bdd_bin_bytes         = {bdd_bin_bytes}",
            apply_wall.as_secs_f64(),
            serialize_wall.as_secs_f64(),
        );
    }

    /// Periodic CUDD-CHECKPOINT (configflux-8l3b). FFI reads stay here
    /// (single-importer rule, ADR-0011 + Amendment 1 §A1.3); the
    /// env-gate / RSS / format logic is in sibling `checkpoint.rs`.
    fn maybe_checkpoint(&self, clause_count: u64, build_start: Instant) {
        // SAFETY: manager is live (mid-build, before Cudd_Quit in Drop).
        let m = self.manager.as_ptr();
        let snapshot = unsafe {
            CheckpointSnapshot {
                clause_count,
                peak_node_count: Cudd_ReadPeakNodeCount(m),
                live_keys: Cudd_ReadKeys(m),
                dead: Cudd_ReadDead(m),
            }
        };
        emit_if_enabled(snapshot, build_start);
    }

    /// Return the CUDD constant TRUE node (no ref taken — the caller
    /// must `Cudd_Ref` if they intend to keep it). Used as the seed
    /// for the AND-fold over clauses.
    fn constant_true(&mut self) -> NonNull<DdNode> {
        // SAFETY: manager is live.
        let one_ptr = unsafe { Cudd_ReadOne(self.manager.as_ptr()) };
        let one = NonNull::new(one_ptr)
            .expect("Cudd_ReadOne returned NULL on a live manager");
        // Take a ref so the discipline matches `and`/`or`/`not`
        // (every returned node from a builder method is reffed).
        // SAFETY: `one` is a valid CUDD-managed node.
        unsafe { Cudd_Ref(one.as_ptr()) };
        one
    }

    /// Return the CUDD constant FALSE node. Mirrors `constant_true`.
    fn constant_false(&mut self) -> NonNull<DdNode> {
        // SAFETY: manager is live.
        let zero_ptr = unsafe { Cudd_ReadLogicZero(self.manager.as_ptr()) };
        let zero = NonNull::new(zero_ptr)
            .expect("Cudd_ReadLogicZero returned NULL on a live manager");
        // SAFETY: `zero` is a valid CUDD-managed node.
        unsafe { Cudd_Ref(zero.as_ptr()) };
        zero
    }

    /// Read back the manager's `maxCacheHard` (entries). configflux-xh57:
    /// lets the test suite assert the derived cap reached the manager (the
    /// budget is not a silent no-op on the cudd path).
    #[cfg(test)]
    fn read_max_cache_hard(&self) -> u32 {
        // SAFETY: manager is live; `Cudd_ReadMaxCacheHard` is a read-only
        // inspector that never NULLs on a live manager.
        unsafe { Cudd_ReadMaxCacheHard(self.manager.as_ptr()) }
    }

    /// Release a node ref. Mirrors `Cudd_RecursiveDeref`.
    fn deref(&mut self, node: NonNull<DdNode>) {
        // SAFETY: caller must have held a ref on `node`. Manager is live.
        unsafe { Cudd_RecursiveDeref(self.manager.as_ptr(), node.as_ptr()) };
    }

    /// Take an additional ref on a node the caller already holds, so the
    /// node can be handed to a fold accumulator (and later deref'd via
    /// that accumulator) without disturbing the caller's own ref. Used by
    /// the `ExactlyOneOf` lowering, where each lowered child is read
    /// multiple times (once in the ALO fold, `N−1` times across the AMO
    /// pairs) and must outlive all of them.
    fn clone_ref(&mut self, node: NonNull<DdNode>) -> NonNull<DdNode> {
        // SAFETY: `node` is a live CUDD node the caller holds a ref on;
        // `Cudd_Ref` only increments the count. Manager is live.
        unsafe { Cudd_Ref(node.as_ptr()) };
        node
    }

    /// CUDD-side conjunction. Returns a reffed result.
    fn and(&mut self, a: NonNull<DdNode>, b: NonNull<DdNode>) -> Result<NonNull<DdNode>> {
        // SAFETY: manager is live; `a` and `b` are caller-held refs,
        // so the apply cache cannot recycle them under us.
        let out_ptr = unsafe { Cudd_bddAnd(self.manager.as_ptr(), a.as_ptr(), b.as_ptr()) };
        let out = NonNull::new(out_ptr)
            .ok_or_else(|| anyhow!("Cudd_bddAnd returned NULL (apply cache OutOfCapacity)"))?;
        // SAFETY: `out` is a valid CUDD-managed node returned by
        // `Cudd_bddAnd`; taking a ref keeps it live across subsequent
        // apply calls.
        unsafe { Cudd_Ref(out.as_ptr()) };
        Ok(out)
    }

    /// CUDD-side disjunction. Returns a reffed result.
    fn or(&mut self, a: NonNull<DdNode>, b: NonNull<DdNode>) -> Result<NonNull<DdNode>> {
        // SAFETY: manager is live; `a` and `b` are caller-held refs.
        let out_ptr = unsafe { Cudd_bddOr(self.manager.as_ptr(), a.as_ptr(), b.as_ptr()) };
        let out = NonNull::new(out_ptr)
            .ok_or_else(|| anyhow!("Cudd_bddOr returned NULL (apply cache OutOfCapacity)"))?;
        // SAFETY: `out` is a valid CUDD-managed node.
        unsafe { Cudd_Ref(out.as_ptr()) };
        Ok(out)
    }

    /// CUDD-side negation. `Cudd_Not` flips the complement bit in
    /// place; the returned pointer shares the underlying node and
    /// MUST be reffed independently so an apply on a child does not
    /// invalidate this handle.
    fn not(&mut self, a: NonNull<DdNode>) -> NonNull<DdNode> {
        // SAFETY: `a` is a valid CUDD node ptr held by the caller.
        // `Cudd_Not` is pure pointer arithmetic on the complement bit;
        // it never returns NULL on a valid input.
        let out_ptr = unsafe { Cudd_Not(a.as_ptr()) };
        let out = NonNull::new(out_ptr)
            .expect("Cudd_Not returned NULL on a non-null input");
        // SAFETY: `out` is a valid CUDD-managed node (same underlying
        // node as `a`, complement bit flipped).
        unsafe { Cudd_Ref(out.as_ptr()) };
        out
    }

    /// Look up the BDD variable index for a `tag.value` symbol name.
    fn lookup_literal(&self, name: &str, index: SymbolIndex) -> Result<NonNull<DdNode>> {
        let var_idx = *index
            .get(name)
            .ok_or_else(|| anyhow!("missing symbol index for {name}"))?;
        let var = *self.var_functions.get(var_idx as usize).ok_or_else(|| {
            anyhow!("var_index {var_idx} out of range (var_count={})", self.var_functions.len())
        })?;
        // Take a fresh ref so the caller owns it (mirrors the
        // and/or/not return convention).
        // SAFETY: `var` is a valid CUDD-managed node held in
        // `var_functions[]`.
        unsafe { Cudd_Ref(var.as_ptr()) };
        Ok(var)
    }

    /// Compile a `ConditionExpr` recursively into a reffed CUDD node.
    /// Mirrors `compiler::ccm_emitter::compile_expr`.
    fn compile_expr(
        &mut self,
        expr: &ConditionExpr,
        index: SymbolIndex,
    ) -> Result<NonNull<DdNode>> {
        match expr {
            ConditionExpr::Bool(value) => Ok(if *value {
                self.constant_true()
            } else {
                self.constant_false()
            }),
            ConditionExpr::Predicate(predicate) => self.compile_predicate(predicate, index),
            ConditionExpr::Not(inner) => {
                let inner_node = self.compile_expr(inner, index)?;
                let out = self.not(inner_node);
                // Drop the ref on `inner_node`; `out` is independent
                // (CUDD's complement edges share the underlying node,
                // but the explicit ref/deref pair makes the discipline
                // uniform across operators).
                self.deref(inner_node);
                Ok(out)
            }
            ConditionExpr::And(left, right) => {
                let l = self.compile_expr(left, index)?;
                let r = self.compile_expr(right, index)?;
                let out = self.and(l, r)?;
                self.deref(l);
                self.deref(r);
                Ok(out)
            }
            ConditionExpr::Or(left, right) => {
                let l = self.compile_expr(left, index)?;
                let r = self.compile_expr(right, index)?;
                let out = self.or(l, r)?;
                self.deref(l);
                self.deref(r);
                Ok(out)
            }
            // `any_of(e_1, …, e_N)` — OR-reduction (ADR-0006 §4), mirroring
            // the native backend in lock-step: lower each child in ascending
            // `Vec` index order and fold with `self.or` strictly
            // left-associatively from `c_1` (ADR-0006 §5, pinned fold
            // direction). `self.or` returns a freshly reffed node, so after
            // each apply we drop the refs on both operands — exactly the
            // ref/deref discipline the binary `Or` arm above uses. For
            // `N = 1` the loop yields `c_1` unchanged. Empty lists are
            // rejected by the parser (configflux-ccs.3); the `None`-accumulator
            // fallback returns constant-false and is unreachable through it.
            ConditionExpr::AnyOf(args) => {
                let mut acc: Option<NonNull<DdNode>> = None;
                for child in args {
                    let c = self.compile_expr(child, index)?;
                    acc = Some(match acc {
                        None => c,
                        Some(prev) => {
                            let out = self.or(prev, c)?;
                            self.deref(prev);
                            self.deref(c);
                            out
                        }
                    });
                }
                match acc {
                    Some(node) => Ok(node),
                    None => Ok(self.constant_false()),
                }
            }
            // `all_of(e_1, …, e_N)` — AND-reduction (ADR-0006 §4), mirroring
            // the native backend in lock-step: same accumulator shape as
            // `AnyOf` above but folding with `self.and`, ascending `Vec` index,
            // left-associative from `c_1` (ADR-0006 §5, pinned fold direction).
            // `self.and` returns a freshly reffed node, so each apply drops the
            // refs on both operands — the ref/deref discipline of the binary
            // `And` arm. `N = 1` yields `c_1`; empty lists are parser-rejected
            // (configflux-ccs.3), the `None` fallback returns constant-true.
            ConditionExpr::AllOf(args) => {
                let mut acc: Option<NonNull<DdNode>> = None;
                for child in args {
                    let c = self.compile_expr(child, index)?;
                    acc = Some(match acc {
                        None => c,
                        Some(prev) => {
                            let out = self.and(prev, c)?;
                            self.deref(prev);
                            self.deref(c);
                            out
                        }
                    });
                }
                match acc {
                    Some(node) => Ok(node),
                    None => Ok(self.constant_true()),
                }
            }
            // `exactly_one_of(e_1, …, e_N)` — at-least-one (OR) ∧ pairwise
            // at-most-one (AMO) (ADR-0006 §4, configflux-ccs.6), mirroring the
            // native backend in lock-step. Logical meaning: exactly one child
            // holds. Built in the ADR-0006 §5 pinned order:
            //   1. lower each child `e_i` to `c_i` ONCE, ascending `Vec` index;
            //   2. ALO = `c_1 ∨ … ∨ c_N`, OR-fold left-assoc from `c_1`
            //      (identical to the `AnyOf` arm above);
            //   3. AMO = AND-fold of `¬(c_i ∧ c_j)` over every pair `i < j` in
            //      ascending `(i, j)` order (AND-identity constant-true when
            //      there are no pairs);
            //   4. result = `ALO ∧ AMO`. `N = 1` ⇒ AMO is constant-true and the
            //      result reduces to `c_1`; `N = 2` ⇒ XOR.
            // The `N ≤ 16` pairwise bound and the `N > 16` warning live on the
            // native path (`ccm_emitter::warn_if_exceeds_pairwise_amo_bound`),
            // which runs for every build regardless of `--construction`; the
            // CUDD path is reached only after that check, so it is not repeated
            // here (it would double-emit the warning).
            //
            // Ref discipline (the delicate part for CUDD): each lowered child
            // is read N + (N−1) times total but holds exactly one ref. CUDD
            // applies READ their operands without consuming refs, so the
            // children stay valid across every read; they are deref'd exactly
            // once each at the very end. Every intermediate produced by
            // `self.or`/`self.and`/`self.not` is reffed on return and deref'd
            // as soon as it is consumed — never deref a child here. The ALO
            // accumulator is seeded with a fresh `clone_ref` of `c_1` so that
            // deref'ing the accumulator never touches the child's own ref.
            ConditionExpr::ExactlyOneOf(args) => {
                let children: Vec<NonNull<DdNode>> = args
                    .iter()
                    .map(|child| self.compile_expr(child, index))
                    .collect::<Result<_>>()?;

                // ALO: OR-fold ascending, left-associative from c_1. Deref the
                // accumulator intermediates only; children are kept alive.
                let mut alo: Option<NonNull<DdNode>> = None;
                for &c in &children {
                    alo = Some(match alo {
                        None => self.clone_ref(c),
                        Some(prev) => {
                            let next = self.or(prev, c)?;
                            self.deref(prev);
                            next
                        }
                    });
                }
                // Empty `args` is parser-rejected (configflux-ccs.3); the
                // constant-false fallback keeps the function total.
                let alo = match alo {
                    Some(node) => node,
                    None => self.constant_false(),
                };

                // AMO: pairwise mutex `¬(c_i ∧ c_j)`, ascending (i, j),
                // AND-folded left-associatively. constant-true when no pairs.
                let mut amo: Option<NonNull<DdNode>> = None;
                for i in 0..children.len() {
                    for j in (i + 1)..children.len() {
                        let pair_and = self.and(children[i], children[j])?;
                        let mutex = self.not(pair_and);
                        self.deref(pair_and);
                        amo = Some(match amo {
                            None => mutex,
                            Some(prev) => {
                                let next = self.and(prev, mutex)?;
                                self.deref(prev);
                                self.deref(mutex);
                                next
                            }
                        });
                    }
                }
                let amo = match amo {
                    Some(node) => node,
                    None => self.constant_true(),
                };

                let result = self.and(alo, amo)?;
                self.deref(alo);
                self.deref(amo);
                for c in children {
                    self.deref(c);
                }
                Ok(result)
            }
            // configflux-secb.2 / ADR-0057 §D5: expanded into the
            // `And`/`Or`/`Not`/`Predicate` fragment by
            // `ccm_emitter::parse_condition_model` before either backend sees
            // it, because the pairwise equivalence it denotes ranges over the
            // union of the two DECLARED domains and `index` carries no domain
            // information. Expanding once, upstream, is also what makes this
            // backend and the in-crate one produce the same canonical BDD:
            // they fold an identical tree with the arms above. Reaching here
            // means the expansion was skipped.
            ConditionExpr::FacetCompare { left, op, right } => bail!(
                "internal: facet comparison '{} {} {}' reached CUDD lowering unexpanded; \
                 `parse_condition_model` must expand it against the declared facet domains first",
                left,
                match op {
                    ConditionPredicateOp::Eq => "==",
                    ConditionPredicateOp::NotEq => "!=",
                },
                right
            ),
        }
    }

    fn compile_predicate(
        &mut self,
        predicate: &ConditionPredicate,
        index: SymbolIndex,
    ) -> Result<NonNull<DdNode>> {
        let name = format!("{}.{}", predicate.tag, predicate.value);
        let node = self.lookup_literal(&name, index)?;
        match predicate.op {
            ConditionPredicateOp::Eq => Ok(node),
            ConditionPredicateOp::NotEq => {
                let neg = self.not(node);
                self.deref(node);
                Ok(neg)
            }
        }
    }

    /// Walk the CUDD-rooted DAG and emit the ADR-0005 §4 byte stream.
    ///
    /// The walk is a complement-edge-erasing post-order DFS that mirrors
    /// `solver/src/cudd_translate.rs::cudd_to_canonical`. Per ADR-0011
    /// Amendment 1 §A1.2, post-order DFS recursion order IS the
    /// cross-backend wire contract — we do not apply the in-crate
    /// `ordered_nodes` band sort (which is a strict refinement, not
    /// part of the wire contract).
    ///
    /// The output format is identical in shape to
    /// `compiler::ccm_emitter::bdd::BddBuilder::serialize`: header +
    /// `root_table` + terminal records at indices 0/1 + non-terminals
    /// in post-order DFS order.
    fn serialize(&mut self, root: NonNull<DdNode>, var_count: u32) -> Result<Vec<u8>> {
        // Build the post-order canonical table via the same algorithm
        // as `cudd_translate::cudd_to_canonical`. We inline it here
        // instead of calling the solver-side function for two reasons:
        // (1) ADR-0003 §2 forbids `compiler` depending on `solver`;
        // (2) per architect critique on configflux-wbzw, the
        // duplication is acceptable — extracting a shared crate is
        // premature for ~50 LOC of post-ADR-0005-stable code.
        //
        // SAFETY: `root` is a valid CUDD node held by the caller;
        // manager is live for the lifetime of `self`.
        let (nodes, root_id) = unsafe { cudd_to_canonical_walk(root) };

        // Sanity: nodes[0] must be FALSE-terminal, nodes[1] must be
        // TRUE-terminal (the walk inserts these as the first two
        // records; if it produced something else, the walk is broken).
        if nodes.len() < 2
            || nodes[0].var_index != TERMINAL_VAR_INDEX
            || nodes[1].var_index != TERMINAL_VAR_INDEX
        {
            bail!(
                "internal: cudd_to_canonical_walk did not seed terminal records (len={}, n0.var={:#x}, n1.var={:#x})",
                nodes.len(),
                nodes.first().map(|n| n.var_index).unwrap_or(0),
                nodes.get(1).map(|n| n.var_index).unwrap_or(0)
            );
        }

        // Emit bytes. The layout mirrors
        // `compiler::ccm_emitter::bdd::BddBuilder::serialize`:
        //   magic[4] | version[1] | reserved[3] | var_count[4] |
        //   node_count[4] | root_count[4] | root_table[root_count*4] |
        //   node_record[node_count * 16]
        //
        // node_count in the in-crate path is `nodes.len() + 2` because
        // its `nodes` excludes terminals; here `nodes` already includes
        // the two terminals, so node_count is `nodes.len()` directly.
        let mut bytes = Vec::with_capacity(20 + 4 + nodes.len() * 16);
        bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
        bytes.push(CCM_BDD_BIN_VERSION);
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes.extend_from_slice(&var_count.to_le_bytes());
        bytes.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count
        bytes.extend_from_slice(&root_id.to_le_bytes());
        for node in &nodes {
            push_node(&mut bytes, node.var_index, node.low_id, node.high_id);
        }
        Ok(bytes)
    }
}

impl Drop for CuddBuilder {
    fn drop(&mut self) {
        // Order matters: deref every node we own before tearing down
        // the manager. Mirrors solver/src/backend_cudd.rs:191-214.
        //
        // SAFETY: every pointer in `var_functions` / `extra_refs` was
        // produced by a CUDD function and `Cudd_Ref`'d at the
        // production site; the manager is still live.
        unsafe {
            for node in self.extra_refs.drain(..) {
                Cudd_RecursiveDeref(self.manager.as_ptr(), node.as_ptr());
            }
            for node in self.var_functions.drain(..) {
                Cudd_RecursiveDeref(self.manager.as_ptr(), node.as_ptr());
            }
            Cudd_Quit(self.manager.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------
// CUDD-rooted DAG walk producing the ADR-0005 §4 canonical node table.
// Cribbed (intentional copy, NOT extracted) from
// `solver/src/cudd_translate.rs::cudd_to_canonical` per architect
// critique on configflux-wbzw. Sized at ~50 LOC of post-ADR-0005-stable
// code; the architect's verdict is that extracting a shared crate is
// premature for that span. ADR-0003 §2 also forbids compiler->solver
// imports, which would block any shared-crate option anyway.
// ---------------------------------------------------------------------

/// Lightweight canonical node record used by the serialize pass. Matches
/// the shape of `solver::ccm_format::BddNode` but we cannot share that
/// type because the solver crate is not a dependency of the compiler
/// (ADR-0003 §2).
#[derive(Debug, Clone, Copy)]
struct CanonicalNode {
    var_index: u32,
    low_id: u32,
    high_id: u32,
}

/// Index of the FALSE terminal record in the node table per ADR-0005
/// §4. The walk emits two leading terminal records (indices 0 and 1)
/// so callers can treat the returned `(nodes, root_id)` pair as a
/// drop-in for `push_node` byte emission.
const FALSE_NODE_INDEX: u32 = 0;

/// Index of the TRUE terminal record in the node table per ADR-0005 §4.
const TRUE_NODE_INDEX: u32 = 1;

/// Walk a CUDD-rooted sub-DAG and emit the ADR-0005 §4 canonical node
/// table for the same Boolean function. Complement edges are erased
/// (see module-level doc); every emitted record carries `flags = 0`.
///
/// Returns `(nodes, root_id)` where `nodes[0]` is the FALSE terminal,
/// `nodes[1]` is the TRUE terminal, and `nodes[2..]` are non-terminals
/// in post-order DFS order (the canonical cross-backend wire ordering
/// per ADR-0011 Amendment 1 §A1.2). `root_id` is either a sentinel
/// (`TERMINAL_FALSE` / `TERMINAL_TRUE`) for a constant formula or a
/// zero-based index into `nodes`.
///
/// # Safety
///
/// `root` must be a valid CUDD node pointer in a live manager. The walk
/// is read-only; this function does NOT take or release any CUDD
/// references.
unsafe fn cudd_to_canonical_walk(root: NonNull<DdNode>) -> (Vec<CanonicalNode>, u32) {
    let mut out: Vec<CanonicalNode> = vec![
        CanonicalNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_FALSE,
            high_id: TERMINAL_FALSE,
        },
        CanonicalNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_TRUE,
            high_id: TERMINAL_TRUE,
        },
    ];

    let mut memo: HashMap<(usize, u8), u32> = HashMap::new();
    let root_index = visit(root, &mut out, &mut memo);
    let root_id = match root_index {
        FALSE_NODE_INDEX => TERMINAL_FALSE,
        TRUE_NODE_INDEX => TERMINAL_TRUE,
        idx => idx,
    };
    (out, root_id)
}

unsafe fn visit(
    node: NonNull<DdNode>,
    out: &mut Vec<CanonicalNode>,
    memo: &mut HashMap<(usize, u8), u32>,
) -> u32 {
    // SAFETY: caller guarantees `node` is a live CUDD pointer.
    let regular_ptr = Cudd_Regular(node.as_ptr());
    let phase: u8 = if Cudd_IsComplement(node.as_ptr()) != 0 {
        1
    } else {
        0
    };

    if Cudd_IsConstant(regular_ptr) != 0 {
        if phase == 0 {
            return TRUE_NODE_INDEX;
        } else {
            return FALSE_NODE_INDEX;
        }
    }

    let key = (regular_ptr as usize, phase);
    if let Some(&id) = memo.get(&key) {
        return id;
    }

    let then_child_raw = NonNull::new(Cudd_T(regular_ptr))
        .expect("Cudd_T returned NULL on non-constant node");
    let else_child_raw = NonNull::new(Cudd_E(regular_ptr))
        .expect("Cudd_E returned NULL on non-constant node");
    let then_child = if phase == 1 {
        NonNull::new(((then_child_raw.as_ptr() as usize) ^ 1) as *mut DdNode)
            .expect("phase-flipped pointer is non-null")
    } else {
        then_child_raw
    };
    let else_child = if phase == 1 {
        NonNull::new(((else_child_raw.as_ptr() as usize) ^ 1) as *mut DdNode)
            .expect("phase-flipped pointer is non-null")
    } else {
        else_child_raw
    };

    let high_id = visit(then_child, out, memo);
    let low_id = visit(else_child, out, memo);

    // ROBDD reduction rule 1: collapse identical-children nodes. CUDD
    // enforces this internally for its own representation, but after
    // phase erasure the two children may have collapsed to the same
    // canonical id; re-check here.
    if low_id == high_id {
        memo.insert(key, low_id);
        return low_id;
    }

    let var_index = Cudd_NodeReadIndex(regular_ptr);
    let id = out.len() as u32;
    out.push(CanonicalNode {
        var_index,
        low_id,
        high_id,
    });
    memo.insert(key, id);
    id
}

// ---------------------------------------------------------------------
// Byte emission helper — copy of
// `compiler::ccm_emitter::bdd::push_node`. Sized at ~5 lines; per
// architect critique on configflux-wbzw, this duplication is accepted
// alongside the serialize-shape duplication above.
// ---------------------------------------------------------------------

fn push_node(bytes: &mut Vec<u8>, var_index: u32, low: u32, high: u32) {
    bytes.extend_from_slice(&var_index.to_le_bytes());
    bytes.extend_from_slice(&low.to_le_bytes());
    bytes.extend_from_slice(&high.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&[0, 0, 0]);
}

#[cfg(test)]
mod tests;
