// SPDX-License-Identifier: BUSL-1.1
//
// `partition_session` — crate-private multi-partition fan-out for
// `Session<B>` per ADR-0012 §1, §6, §7.
//
// Under v2 every `Ccm` is a multi-part artifact (one or more cluster
// partitions plus an optional bridge per ADR-0012 §4). The public
// `Session<B>` holds one `PartitionSession<B>` per partition; each
// owns its own `B` backend instance and its own current handle, so the
// CUDD `DdNode` / `DdManager` isolation rule from `backend_cudd.rs`
// (and from oxidd's manager-index design) is enforced structurally —
// a handle from partition K's backend can only be passed to partition
// K's backend, by virtue of living inside its `PartitionSession<B>`.
//
// `PartitionFormulaHandle` is the crate-private newtype required by
// ADR-0012 §6 (Option B): every handle that survives outside its
// owning `PartitionSession` (specifically: the undo stack) carries its
// partition index alongside the raw `FormulaHandle`. Restoring a
// handle requires routing through `parts[entry.partition_index]`, so
// the type system enforces "no cross-manager handle reuse" at the
// boundary where it actually matters.
//
// Per ADR-0003 §1 every type in this module is `pub(crate)`. The
// public `Session<B>` API in `session.rs` is the only surface
// downstream consumers see; this module is implementation detail.

use crate::backend::{BackendError, CapacityHints, FormulaHandle, SolverBackend, VariableOrder};
use crate::ccm_multi_part::PartitionCcm;
use crate::session::Error;

/// One partition's session-state. Wraps the partition's owned backend
/// instance (`B`), its current active-formula handle, and its symbol
/// metadata for cross-partition lookup.
///
/// Each `PartitionSession` is **physically isolated** from every other:
/// its `backend` is its own `SolverBackend` instance, its `current`
/// handle is meaningful only against that backend, and its
/// `variable_order` / `facet_to_var_local` maps are partition-local.
/// The multi-partition `Session<B>` in `session.rs` enforces the
/// "handle from K never passed to K+1" rule by always indexing into
/// `parts[partition_index]` before any backend call.
#[derive(Debug)]
pub(crate) struct PartitionSession<B: SolverBackend> {
    /// Stable index assigned at construction. Cluster partitions use
    /// `0..N` ascending; the bridge (when present) uses the sentinel
    /// `BRIDGE_PARTITION_INDEX` (`u32::MAX`) per ADR-0012 §8.
    pub(crate) partition_index: u32,
    /// Owning backend instance for this partition. CUDD or oxidd
    /// manager allocations live here; dropped via the backend's `Drop`
    /// impl when the `Session<B>` is dropped.
    pub(crate) backend: B,
    /// Current active-formula handle. Replaced by `apply`, restored by
    /// `retract`. Only meaningful inside `backend`.
    pub(crate) current: FormulaHandle,
    /// Variable order for this partition, in BDD variable-index order.
    /// Element `i` is the symbol name (e.g. `"region.a"`) bound to
    /// variable `i` inside this partition's backend.
    pub(crate) variable_order: Vec<String>,
}

/// Bridge partition's sentinel index per ADR-0012 §8 (the `u32::MAX`
/// reservation used in the `state_hash` pre-image). Cluster partitions
/// use `0..N` where `N` is bounded by `BRIDGE_PARTITION_INDEX - 1` so
/// the sentinel can never collide with a real cluster index.
pub(crate) const BRIDGE_PARTITION_INDEX: u32 = u32::MAX;

/// Crate-private wrapper that ties a `FormulaHandle` to the partition
/// index of its owning backend. The compile-time enforcement from
/// ADR-0012 §6 falls out of the type: any code that wants to restore
/// a `PartitionFormulaHandle` must read its `partition_index` field,
/// route through `parts[partition_index]`, and only then assign the
/// inner `handle`. There is no API on `PartitionSession` that accepts
/// a `FormulaHandle` from a different partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PartitionFormulaHandle {
    pub(crate) partition_index: u32,
    pub(crate) handle: FormulaHandle,
}

/// One entry on the session's shared undo stack. Each successful
/// `apply` (whether single-partition or atomic multi-partition) pushes
/// one entry per *touched* partition; `retract` pops the top entry and
/// restores only the partition it names, leaving every other
/// partition's `current` unchanged (ADR-0012 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UndoEntry {
    pub(crate) prior: PartitionFormulaHandle,
}

impl<B: SolverBackend> PartitionSession<B> {
    /// Construct a `PartitionSession` from a parsed `PartitionCcm`.
    /// Threads the partition's BDD through a fresh backend instance
    /// and seeds `current` to the deserialized root. Cluster partitions
    /// pass `partition_index` in `0..N`; the bridge passes
    /// `BRIDGE_PARTITION_INDEX`.
    pub(crate) fn from_partition(
        partition_index: u32,
        part: &PartitionCcm,
    ) -> Result<Self, Error> {
        let order = VariableOrder::from_names(part.symbols.variable_order.clone());
        let hints = CapacityHints {
            max_nodes: part.bdd.node_count as usize,
            // configflux-9pjy.4: no budget threads source on this path —
            // keep the historical single-threaded default (`None` ⇒
            // `DEFAULT_THREAD_COUNT` in the backend).
            threads: None,
        };
        let mut backend = B::new_session(order, hints)?;
        let current = backend.deserialize_bdd(
            part.bdd.var_count,
            &part.bdd.nodes,
            part.bdd.roots[0],
        )?;
        Ok(Self {
            partition_index,
            backend,
            current,
            variable_order: part.symbols.variable_order.clone(),
        })
    }

    /// Construct a trivial `PartitionSession` whose backend holds the
    /// constant ⊤ formula. Used for the empty-Ccm session path
    /// (`Ccm::empty()`) so that the multi-partition `Session<B>` always
    /// has at least one partition to fan out to — the M0
    /// round-trip-empty smoke test pins this behavior.
    pub(crate) fn empty_constant_true() -> Result<Self, Error> {
        let mut backend =
            B::new_session(VariableOrder::empty(), CapacityHints::default())?;
        let current = backend.mk_const(true);
        Ok(Self {
            partition_index: 0,
            backend,
            current,
            variable_order: Vec::new(),
        })
    }

    /// Local lookup: if this partition's symbol table contains the
    /// compound symbol `{facet}.{value}`, return its variable index.
    /// Used by `Session::apply` to identify which partitions own the
    /// applied symbol (the bd-0r62 cross-partition apply contract).
    pub(crate) fn var_for_symbol(&self, compound: &str) -> Option<u32> {
        self.variable_order
            .iter()
            .position(|s| s == compound)
            .map(|i| i as u32)
    }

    /// Per-partition `valid_options(facet)` query.
    ///
    /// Walks this partition's variable order, picks every symbol of
    /// the form `{facet}.{value}`, and asks the backend whether
    /// `current ∧ x_i` is satisfiable. Returns the surviving values in
    /// symbol-table order (the same order the legacy single-partition
    /// path used). If no symbol in this partition is prefixed by
    /// `{facet}.`, returns `None` so the caller can distinguish
    /// "facet not in this partition" from "all options pruned".
    pub(crate) fn valid_options_for(
        &self,
        facet_prefix: &str,
    ) -> Result<Option<Vec<String>>, BackendError> {
        let mut saw_any = false;
        let mut options: Vec<String> = Vec::new();
        for (var_idx, sym) in self.variable_order.iter().enumerate() {
            let Some(suffix) = sym.strip_prefix(facet_prefix) else {
                continue;
            };
            saw_any = true;
            let sat = self
                .backend
                .is_var_sat_under(self.current, var_idx as u32)?;
            if sat {
                options.push(suffix.to_string());
            }
        }
        if saw_any {
            Ok(Some(options))
        } else {
            Ok(None)
        }
    }

    /// Conjoin `current` with `x_var_idx = 1` and return the new
    /// handle alongside the prior one wrapped in a
    /// `PartitionFormulaHandle`. Does NOT mutate `self.current` — the
    /// caller is responsible for the swap so that the multi-partition
    /// atomic apply can stage every touched partition's transition
    /// before committing.
    pub(crate) fn stage_apply(
        &mut self,
        var_idx: u32,
    ) -> Result<StagedApply, BackendError> {
        let prior_handle = self.current;
        let new_handle = self.backend.apply_and(prior_handle, var_idx)?;
        let is_false = self.backend.is_false(new_handle);
        Ok(StagedApply {
            prior: PartitionFormulaHandle {
                partition_index: self.partition_index,
                handle: prior_handle,
            },
            new_handle,
            is_false,
        })
    }

    /// Commit a `StagedApply` — installs `staged.new_handle` as
    /// `self.current`. The caller has already verified that no other
    /// touched partition's `stage_apply` produced a ⊥ handle; if any
    /// did, the caller must instead drop every staged transition and
    /// not call `commit_apply` on any of them.
    pub(crate) fn commit_apply(&mut self, staged: &StagedApply) {
        debug_assert_eq!(
            staged.prior.partition_index, self.partition_index,
            "commit_apply must route through the originating partition",
        );
        self.current = staged.new_handle;
    }

    /// Restore `current` from a `PartitionFormulaHandle`. Asserts that
    /// the handle's `partition_index` matches `self.partition_index`
    /// — the type system already routed it correctly via the
    /// `parts[handle.partition_index]` index in `Session::retract`, so
    /// this is a debug-only invariant check that catches refactor
    /// mistakes.
    pub(crate) fn restore(&mut self, handle: PartitionFormulaHandle) {
        debug_assert_eq!(
            handle.partition_index, self.partition_index,
            "restore must route through the originating partition",
        );
        self.current = handle.handle;
    }
}

/// One partition's staged transition mid-atomic-apply. Carries the
/// prior handle (for rollback) and the new handle (for commit). The
/// `is_false` flag is the per-partition outcome of `apply_and` — if
/// any partition's staged apply is `is_false: true`, the multi-
/// partition apply must roll back instead of committing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StagedApply {
    pub(crate) prior: PartitionFormulaHandle,
    pub(crate) new_handle: FormulaHandle,
    pub(crate) is_false: bool,
}
