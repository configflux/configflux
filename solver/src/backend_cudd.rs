// SPDX-License-Identifier: BUSL-1.1
//
// CUDD-backed implementation of `SolverBackend`.
//
// Per ADR-0003 §3 and the ADR-0004 second-pass amendment 2026-05-09
// (configflux-dwwv), this is the ONLY file in the workspace allowed to
// `use cudd_sys::*`. Every other file in `solver/` — including
// `solver::Session`, `NullBackend`, and the parallel `OxiddBackend` —
// calls through the `SolverBackend` trait defined in `backend.rs`.
//
// That isolation is the same AC-5 leg the oxidd audit pinned for
// `backend_oxidd.rs`: callers must not see CUDD types crossing the
// `Session` API. Reviewers should reject any diff that surfaces a
// `*mut DdManager`, `*mut DdNode`, `cudd_sys::*` type, or other CUDD
// internal across the `pub` surface.
//
// Scope: configflux-jxk6 wires the same M0 + M1 surface that
// `backend_oxidd.rs` already exposes:
//   - `new_session`, `mk_const`, `is_false`, `is_true`, `backend_id`
//     (M0 round-trip-empty contract);
//   - `deserialize_bdd` (rebuilds an in-memory BDD from the ADR-0005 §4
//     canonical node table);
//   - `is_var_sat_under` (per-option cofactor query for
//     `Session::valid_options`);
//   - `apply_and` (unit-clause conjunction for `Session::apply`).
//
// Memory discipline: CUDD nodes are reference-counted through
// `Cudd_Ref` / `Cudd_RecursiveDeref`; every `*mut DdNode` we keep
// alive across calls (constants in `formulas[]`, per-variable
// literals in `var_functions[]`, intermediates returned to the
// session) is `Cudd_Ref`'d once on entry into the backend table and
// `Cudd_RecursiveDeref`'d in `Drop`. Borrowed pointers (the `low_id`
// / `high_id` children resolved during a `deserialize_bdd` walk) are
// not ref'd because they are reachable through their parents until
// the parent itself is ref'd. The pattern matches the CUDD manual's
// "owner refs the root only" idiom.
//
// Reordering: dynamic reordering is left at CUDD's default-off state.
// `Cudd_Init` does not enable autodyn; we do not call
// `Cudd_AutodynEnable`. ADR-0004 §5 mitigation rung 2 names sifting
// as a follow-up; turning it on is a deliberate later event with its
// own configflux issue.

use core::ffi::c_uint;
use std::ptr::NonNull;

use cudd_sys::cudd::{
    Cudd_AutodynDisable, Cudd_DagSize, Cudd_Init, Cudd_IsConstant, Cudd_NodeReadIndex, Cudd_Not,
    Cudd_Quit, Cudd_ReadLogicZero, Cudd_ReadOne, Cudd_RecursiveDeref, Cudd_Ref, Cudd_bddAnd,
    CUDD_CACHE_SLOTS, CUDD_UNIQUE_SLOTS,
};
use cudd_sys::DdNode;
use cudd_sys::DdManager;

use crate::backend::{
    BackendError, CapacityHints, FormulaHandle, SolverBackend, VariableOrder,
};
use crate::ccm_format::BddNode;
use crate::cudd_translate::canonical_to_cudd;

/// Stable identifier for the CUDD backend, surfaced through
/// `SolverBackend::backend_id` and eventually written into
/// `ccm.manifest.json.algorithm` per ADR-0005 §2. Pinned as a literal so
/// a silent CUDD source bump cannot silently change the identifier;
/// changing this string is a deliberate review event linked to the
/// vendored `third_party/cudd/` source version.
const CUDD_BACKEND_ID: &str = "cudd-v3.0";

/// Default initial unique-table slot count. CUDD's `Cudd_Init` defaults
/// to `CUDD_UNIQUE_SLOTS` (256) when the caller passes 0, but we pin the
/// value explicitly here so that the M0 empty-CCM session has a
/// deterministic, audit-traceable starting size — independent of any
/// future CUDD upstream tweak to the constant.
const DEFAULT_UNIQUE_SLOTS: c_uint = CUDD_UNIQUE_SLOTS;

/// Default apply-cache slot count. As above, `CUDD_CACHE_SLOTS`
/// (262144) is CUDD's documented default; pinning it here keeps the
/// dev-container footprint predictable and matches the
/// `OxiddBackend::DEFAULT_APPLY_CACHE_CAPACITY` rationale.
const DEFAULT_CACHE_SLOTS: c_uint = CUDD_CACHE_SLOTS;

/// Default `maxMemory` argument to `Cudd_Init`. The CUDD manual lets a
/// caller pass 0 to mean "use the heuristic default"; we forward 0
/// because the backend is bounded externally by `tools/resource_guard.sh`
/// (4 GiB cap on the gate envelope), not by an in-process malloc cap.
const DEFAULT_MAX_MEMORY: usize = 0;

/// Pre-allocated slot holding the constant `⊥` (`false`) BDD node.
///
/// `new_session` pushes `⊥` at index 0 and `⊤` at index 1 of the
/// internal `formulas` table so that `mk_const(value)` becomes a pure
/// lookup — no FFI roundtrip per call. The slots are never reused;
/// callers holding `FormulaHandle(0)` can rely on it pointing at `⊥`
/// for the lifetime of the backend instance.
const FALSE_SLOT: usize = 0;

/// Pre-allocated slot holding the constant `⊤` (`true`) BDD node.
/// See `FALSE_SLOT` for the rationale; this slot is never reused either.
const TRUE_SLOT: usize = 1;

/// Concrete `SolverBackend` wrapping the CUDD ROBDD library.
///
/// State: an owned `*mut DdManager` (every `Cudd_*` call routes through
/// it) plus a `Vec<NonNull<DdNode>>` handle table indexed by
/// `FormulaHandle.0` (never shrinks, so caller-held handles stay valid
/// for the session). `*mut DdNode` is held internally as `NonNull` to
/// document the "never NULL after Cudd_Ref" invariant; raw pointers
/// never leave this module, preserving the ADR-0003 §2 "no CUDD types
/// leak" invariant by construction.
///
/// `Debug` is hand-implemented because raw FFI pointers do not carry a
/// useful `Debug` impl and the crate-wide `deny(missing_debug_implementations)`
/// requires one. The impl emits only the handle-table lengths — no
/// manager introspection, which would require an FFI call inside a
/// formatter.
pub struct CuddBackend {
    /// Owning pointer to the CUDD manager. Freed by `Drop` via
    /// `Cudd_Quit`. Held as `NonNull` because `Cudd_Init` returning
    /// NULL is mapped to `BackendError::OutOfCapacity` at construction
    /// time, so by the time we have a `CuddBackend` value the manager
    /// is guaranteed non-null.
    manager: NonNull<DdManager>,
    /// Handle table mapping `FormulaHandle.0` to the CUDD-side `DdNode`
    /// pointer. Entries 0 (`⊥`) / 1 (`⊤`) are pre-populated by
    /// `new_session`; `deserialize_bdd` and `apply_and` push rebuilt or
    /// conjoined nodes. Every entry is `Cudd_Ref`'d on push and
    /// `Cudd_RecursiveDeref`'d in `Drop`.
    formulas: Vec<NonNull<DdNode>>,
    /// Per-variable literal nodes materialized by `deserialize_bdd`.
    /// `var_functions[i]` is `x_i = 1`; read by `is_var_sat_under` and
    /// `apply_and` for per-option cofactor checks without re-entering
    /// the manager. Every entry is `Cudd_Ref`'d on push and
    /// `Cudd_RecursiveDeref`'d in `Drop`.
    var_functions: Vec<NonNull<DdNode>>,
}

// SAFETY: `CuddBackend` owns its `*mut DdManager` exclusively and the
// session is single-threaded by `SolverBackend`'s `Send` bound. CUDD's
// own data structures are not internally synchronized, so we MUST NOT
// also implement `Sync`; `Send` is sufficient and matches the trait.
// The pointers themselves are valid for the lifetime of the struct
// (constructed by `Cudd_Init`, freed by `Cudd_Quit` in `Drop`).
unsafe impl Send for CuddBackend {}

impl CuddBackend {
    /// Resolve a `FormulaHandle` back to the stored `*mut DdNode`.
    /// Returns `None` only for a forged/out-of-range index; handles
    /// issued by trait methods are always in-range.
    #[inline]
    fn lookup(&self, handle: FormulaHandle) -> Option<NonNull<DdNode>> {
        self.formulas.get(handle.0).copied()
    }

    /// Push a freshly-`Cudd_Ref`'d node into the handle table and
    /// return the new `FormulaHandle`. The caller must have already
    /// `Cudd_Ref`'d the node; this method does NOT re-ref. Used by
    /// `deserialize_bdd` (root push) and `apply_and` (conjunction
    /// result push).
    #[inline]
    fn push_referenced(&mut self, node: NonNull<DdNode>) -> FormulaHandle {
        let idx = self.formulas.len();
        self.formulas.push(node);
        FormulaHandle(idx)
    }

    /// Internal helper: are two `*mut DdNode` pointers the same node?
    /// CUDD nodes are canonical, so pointer equality is the correct
    /// check — two BDDs represent the same Boolean function iff their
    /// node pointers (with complement bit) are equal. Used only by
    /// the in-module test of `formulas[]` slot identity; gated to
    /// `cfg(test)` so the production build does not carry the helper.
    #[cfg(test)]
    #[inline]
    fn ptr_eq(a: NonNull<DdNode>, b: NonNull<DdNode>) -> bool {
        a.as_ptr() == b.as_ptr()
    }
}

// See struct-level doc comment on `CuddBackend` for the rationale
// behind the hand-rolled `Debug` impl.
impl core::fmt::Debug for CuddBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CuddBackend")
            .field("backend_id", &CUDD_BACKEND_ID)
            .field("formula_count", &self.formulas.len())
            .field("var_count", &self.var_functions.len())
            .finish_non_exhaustive()
    }
}

impl Drop for CuddBackend {
    fn drop(&mut self) {
        // Order matters: deref every node we own before tearing down
        // the manager. `Cudd_Quit` itself does not deref individual
        // nodes — it frees the entire arena — but calling
        // `Cudd_RecursiveDeref` first keeps the unique-table refcount
        // invariant satisfied so debug builds of CUDD do not assert.
        //
        // SAFETY: every pointer in `formulas` and `var_functions` was
        // produced by a CUDD function (`Cudd_ReadOne`, `Cudd_bddIthVar`,
        // `Cudd_bddIte`, `Cudd_bddAnd`) and `Cudd_Ref`'d on push. The
        // manager pointer is still live because `Cudd_Quit` has not
        // been called yet.
        unsafe {
            for node in self.formulas.drain(..) {
                Cudd_RecursiveDeref(self.manager.as_ptr(), node.as_ptr());
            }
            for node in self.var_functions.drain(..) {
                Cudd_RecursiveDeref(self.manager.as_ptr(), node.as_ptr());
            }
            Cudd_Quit(self.manager.as_ptr());
        }
    }
}

impl SolverBackend for CuddBackend {
    fn new_session(
        order: VariableOrder,
        _hints: CapacityHints,
    ) -> Result<Self, BackendError> {
        // CUDD's `Cudd_Init` allocates a fresh manager. We pass the
        // documented defaults for unique-table and apply-cache sizes;
        // `CapacityHints::max_nodes` is intentionally unused on this
        // backend because CUDD grows its tables dynamically (the
        // M1 acceptance bar tracks RSS via `tools/resource_guard.sh`,
        // not in-process limits).
        //
        // SAFETY: `Cudd_Init` is a documented FFI entry point that
        // either returns a valid pointer or NULL on allocation failure.
        // We materialize the NonNull invariant immediately after.
        let manager_ptr = unsafe {
            Cudd_Init(
                0, // numVars: variables are added on demand by Cudd_bddIthVar
                0, // numVarsZ: ZDD variables not used
                DEFAULT_UNIQUE_SLOTS,
                DEFAULT_CACHE_SLOTS,
                DEFAULT_MAX_MEMORY,
            )
        };
        let manager = NonNull::new(manager_ptr).ok_or(BackendError::OutOfCapacity)?;

        // Defensive: explicitly disable dynamic reordering. CUDD's
        // default is off, but a future linked-against build that
        // ships with autodyn-on as a compile-time toggle would
        // silently change semantics — this call pins the contract
        // regardless of the upstream default.
        //
        // SAFETY: `manager` is freshly-initialized and non-null.
        unsafe { Cudd_AutodynDisable(manager.as_ptr()) };

        // Pre-populate the constant slots. We build `⊥` (logic zero)
        // and `⊤` (logic one) once, ref each, and stash them in the
        // handle table. Subsequent `mk_const` calls become pure O(1)
        // table lookups — no FFI per call.
        //
        // SAFETY: `Cudd_ReadOne` / `Cudd_ReadLogicZero` always return
        // valid CUDD-managed nodes for a live manager; both are also
        // documented to never need a `Cudd_Ref` for the constants
        // themselves, but reffing is harmless and keeps the
        // bookkeeping uniform with the variable / intermediate path.
        let (false_ptr, true_ptr) = unsafe {
            let zero = Cudd_ReadLogicZero(manager.as_ptr());
            let one = Cudd_ReadOne(manager.as_ptr());
            Cudd_Ref(zero);
            Cudd_Ref(one);
            (zero, one)
        };
        let false_node = NonNull::new(false_ptr).ok_or(BackendError::Invariant(
            "Cudd_ReadLogicZero returned NULL on a fresh manager",
        ))?;
        let true_node = NonNull::new(true_ptr).ok_or(BackendError::Invariant(
            "Cudd_ReadOne returned NULL on a fresh manager",
        ))?;

        let mut formulas = Vec::with_capacity(2);
        formulas.push(false_node);
        formulas.push(true_node);

        // `order` is accepted but not consumed in the M0 surface —
        // variable introduction (`mk_var`) lands with `deserialize_bdd`,
        // which is the only entry point that materializes variables on
        // this backend. Acknowledging it explicitly keeps the signature
        // forward-compatible and silences the unused-binding lint.
        let _ = order;

        Ok(Self {
            manager,
            formulas,
            var_functions: Vec::new(),
        })
    }

    fn mk_const(&mut self, value: bool) -> FormulaHandle {
        // Pure lookup — the constants were built once in `new_session`.
        // This matches the NullBackend / OxiddBackend convention where
        // `FormulaHandle(0) == ⊥` and `FormulaHandle(1) == ⊤`, so code
        // written against either of those backends continues to read
        // the same way under the CUDD backend.
        if value {
            FormulaHandle(TRUE_SLOT)
        } else {
            FormulaHandle(FALSE_SLOT)
        }
    }

    fn is_false(&self, f: FormulaHandle) -> bool {
        // A handle represents `⊥` iff its stored pointer is structurally
        // equal to the canonical `Cudd_ReadLogicZero(manager)` pointer.
        // CUDD BDDs are canonical, so pointer equality is the correct
        // check — two BDDs are equal iff they represent the same
        // Boolean function, and the constant-false function has a
        // unique representation in a ROBDD.
        let Some(stored) = self.lookup(f) else {
            return false;
        };
        // SAFETY: the manager is live for the lifetime of `self`.
        let zero_ptr = unsafe { Cudd_ReadLogicZero(self.manager.as_ptr()) };
        stored.as_ptr() == zero_ptr
    }

    fn is_true(&self, f: FormulaHandle) -> bool {
        // Mirror of `is_false`; see comment there for the canonicity
        // argument.
        let Some(stored) = self.lookup(f) else {
            return false;
        };
        // SAFETY: the manager is live for the lifetime of `self`.
        let one_ptr = unsafe { Cudd_ReadOne(self.manager.as_ptr()) };
        stored.as_ptr() == one_ptr
    }

    fn backend_id(&self) -> &'static str {
        CUDD_BACKEND_ID
    }

    /// Pure-query cofactor check: does `current ∧ x_var_idx` have a
    /// satisfying assignment? Computes the conjunction via CUDD's apply
    /// cache, compares the result to `Cudd_ReadLogicZero`, and drops
    /// the intermediate node. CUDD BDDs are canonical, so structural
    /// equality with the FALSE terminal is exactly an UNSAT check.
    /// The intermediate `*mut DdNode` is `Cudd_Ref`'d before the equality
    /// check (so the apply cache cannot recycle it under us) and
    /// `Cudd_RecursiveDeref`'d before return — which is what makes this
    /// `&self`-friendly and therefore compatible with ADR-0003 §4's
    /// pure-query pin on `Session::valid_options`.
    fn is_var_sat_under(
        &self,
        current: FormulaHandle,
        var_idx: u32,
    ) -> Result<bool, BackendError> {
        let current_node = self.lookup(current).ok_or(BackendError::Invariant(
            "is_var_sat_under(): current handle out of range",
        ))?;
        let var_node = self
            .var_functions
            .get(var_idx as usize)
            .copied()
            .ok_or(BackendError::Invariant(
                "is_var_sat_under(): var_idx out of range",
            ))?;
        // SAFETY: both pointers are live (kept alive by their respective
        // `formulas[]` / `var_functions[]` ref); the manager is live.
        // `Cudd_bddAnd` returns NULL on apply-cache exhaustion, which
        // we surface as `OutOfCapacity`.
        let conj_ptr = unsafe {
            Cudd_bddAnd(
                self.manager.as_ptr(),
                current_node.as_ptr(),
                var_node.as_ptr(),
            )
        };
        let conj = NonNull::new(conj_ptr).ok_or(BackendError::OutOfCapacity)?;
        // Ref the intermediate so the cache cannot recycle it before
        // we read it back; deref before return so the node count stays
        // bounded across many `valid_options` calls.
        // SAFETY: manager + node pointers are live.
        unsafe { Cudd_Ref(conj.as_ptr()) };
        let zero_ptr = unsafe { Cudd_ReadLogicZero(self.manager.as_ptr()) };
        let is_false = conj.as_ptr() == zero_ptr;
        // SAFETY: we own the ref we just took.
        unsafe { Cudd_RecursiveDeref(self.manager.as_ptr(), conj.as_ptr()) };
        Ok(!is_false)
    }

    /// Conjoin the formula behind `current` with the unit clause
    /// `x_var_idx = 1` and push the result into the handle table.
    ///
    /// Unlike `is_var_sat_under` — which derefs the intermediate
    /// after a SAT check — `apply_and` keeps the resulting BDD alive
    /// on `self.formulas` so `Session::apply` can swap it in as the
    /// new active-formula handle and keep the old one on its undo
    /// stack. The returned handle indexes into the same table as
    /// handles issued by `deserialize_bdd`; handle ids never wrap
    /// because `self.formulas` never shrinks.
    ///
    /// On an unsat conjunction (CUDD returns the structural FALSE
    /// pointer) we still push the FALSE node into the handle table
    /// and return a handle to it — the caller uses `is_false` on that
    /// handle to detect the conflict. This keeps the API symmetric
    /// across sat/unsat results and avoids a dedicated `Err` variant
    /// here (the error translation into `Session::Error::Conflict`
    /// lives in `Session::apply`).
    fn apply_and(
        &mut self,
        current: FormulaHandle,
        var_idx: u32,
    ) -> Result<FormulaHandle, BackendError> {
        let current_node = self.lookup(current).ok_or(BackendError::Invariant(
            "apply_and(): current handle out of range",
        ))?;
        let var_node = self
            .var_functions
            .get(var_idx as usize)
            .copied()
            .ok_or(BackendError::Invariant("apply_and(): var_idx out of range"))?;
        // SAFETY: both pointers are live; manager is live.
        let conj_ptr = unsafe {
            Cudd_bddAnd(
                self.manager.as_ptr(),
                current_node.as_ptr(),
                var_node.as_ptr(),
            )
        };
        let conj = NonNull::new(conj_ptr).ok_or(BackendError::OutOfCapacity)?;
        // Ref before push so the handle table holds the only owning
        // reference outside of CUDD's internal DAG.
        // SAFETY: manager + node pointers are live.
        unsafe { Cudd_Ref(conj.as_ptr()) };
        Ok(self.push_referenced(conj))
    }

    /// Rebuild a CUDD BDD from the ADR-0005 §4 canonical node table.
    ///
    /// The translation walk lives in `crate::cudd_translate` so the
    /// round-trip integration test (configflux-fvew) can exercise it
    /// without going through the `SolverBackend` trait. This method is
    /// the trait-side wrapper: it delegates the table walk and then
    /// integrates the result into the backend's handle table
    /// (`var_functions[]` for per-variable cofactor lookups,
    /// `formulas[]` for the rebuilt root).
    ///
    /// Error handling: any CUDD allocation failure (NULL return)
    /// surfaces as `BackendError::OutOfCapacity`. An out-of-range
    /// `var_index`, an out-of-range child id, or a malformed root is
    /// reported as `BackendError::Serialization` with a static
    /// message — those cases should have been caught by
    /// `ccm_format::parse_bdd_bin`; reaching them here means a caller
    /// bypassed the format reader with a hand-built node table, which
    /// is OK in tests but not in production.
    fn deserialize_bdd(
        &mut self,
        var_count: u32,
        nodes: &[BddNode],
        root: u32,
    ) -> Result<FormulaHandle, BackendError> {
        // SAFETY: `self.manager` is live for the lifetime of `self`;
        // `canonical_to_cudd` documents that it takes a single
        // `Cudd_Ref` on each returned `var_functions[]` entry and on
        // the returned root, and makes no other mutation to the manager
        // beyond standard CUDD bookkeeping. The caller owns the
        // refcount discipline from here on; `Drop` on `CuddBackend`
        // dereferences both vectors before `Cudd_Quit`.
        let translated =
            unsafe { canonical_to_cudd(self.manager, var_count, nodes, root)? };
        self.var_functions = translated.var_functions;
        Ok(self.push_referenced(translated.root))
    }
}

// Suppress unused-import warnings for the FFI helpers we keep imported
// for future M1 surface growth (DAG-size introspection, sat-count, etc).
// Once the trait grows those methods these imports will be exercised.
#[allow(dead_code)]
fn _keep_ffi_imports_warm(mgr: *mut DdManager, n: *mut DdNode) {
    // SAFETY: never called; this fn exists so unused-import lints stay
    // green for symbols we want compile-time visible for M1 growth.
    unsafe {
        let _ = Cudd_DagSize(n);
        let _ = Cudd_IsConstant(n);
        let _ = Cudd_NodeReadIndex(n);
        let _ = Cudd_Not(n);
        let _ = mgr;
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the CUDD-backed `SolverBackend` implementation.
    //!
    //! These tests live inside the backend module so they can touch
    //! internal fields (`formulas`, `manager`) that the public
    //! `SolverBackend` surface does not expose. The external integration
    //! coverage — which proves no CUDD type leaks across the crate
    //! boundary AND proves semantic parity vs OxiddBackend — lives in
    //! `solver/tests/cudd_backend_parity.rs`.
    use super::*;

    #[test]
    fn new_session_seeds_constant_slots() {
        let backend = CuddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed on empty inputs");
        assert_eq!(
            backend.formulas.len(),
            2,
            "new_session must seed exactly two constant slots"
        );
    }

    #[test]
    fn mk_const_returns_stable_slots() {
        let mut backend = CuddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        assert_eq!(backend.mk_const(false), FormulaHandle(FALSE_SLOT));
        assert_eq!(backend.mk_const(true), FormulaHandle(TRUE_SLOT));
        // Repeated calls must not grow the handle table — `mk_const`
        // on a constant is idempotent at the wrapper level.
        let _ = backend.mk_const(false);
        let _ = backend.mk_const(true);
        assert_eq!(
            backend.formulas.len(),
            2,
            "mk_const on constants must not extend the handle table"
        );
    }

    #[test]
    fn is_true_and_is_false_distinguish_the_constants() {
        let mut backend = CuddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        let t = backend.mk_const(true);
        let f = backend.mk_const(false);
        assert!(backend.is_true(t), "mk_const(true) must report is_true");
        assert!(!backend.is_false(t), "mk_const(true) must NOT report is_false");
        assert!(backend.is_false(f), "mk_const(false) must report is_false");
        assert!(!backend.is_true(f), "mk_const(false) must NOT report is_true");
    }

    #[test]
    fn is_true_returns_false_for_forged_handle() {
        let backend = CuddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        let forged = FormulaHandle(9_999);
        assert!(!backend.is_true(forged));
        assert!(!backend.is_false(forged));
    }

    #[test]
    fn backend_id_is_stable() {
        let backend = CuddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        assert_eq!(backend.backend_id(), CUDD_BACKEND_ID);
        assert_eq!(backend.backend_id(), "cudd-v3.0");
    }

    #[test]
    fn ptr_eq_helper_matches_pointer_equality() {
        // Sanity check for the internal helper. Two NonNull values
        // built from the same constant pointer must compare equal;
        // values built from distinct constants must not.
        let backend = CuddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        let f = backend.formulas[FALSE_SLOT];
        let t = backend.formulas[TRUE_SLOT];
        assert!(CuddBackend::ptr_eq(f, f));
        assert!(CuddBackend::ptr_eq(t, t));
        assert!(!CuddBackend::ptr_eq(f, t));
    }
}
