// SPDX-License-Identifier: BUSL-1.1
//
// oxidd-backed implementation of `SolverBackend`.
//
// Per ADR-0003 §3 and `docs/security/oxidd-audit.md` §5–§6, this is the
// ONLY file in the workspace allowed to `use oxidd::*`. Every other
// file in `solver/` — including `solver::Session` and `NullBackend` —
// calls through the `SolverBackend` trait defined in `backend.rs`. That
// isolation is the AC-5 leg of the oxidd audit's acceptance bar
// (oxidd-audit.md §7.5); violating it is a license- and audit-relevant
// change, and reviewers should reject such diffs on sight.
//
// Scope: configflux-8dm.1 wired the M0 scaffolding surface
// (`new_session`, `mk_const`, `is_false`, `is_true`, `backend_id`);
// 8dm.2 added `deserialize_bdd`; 8dm.3 added `is_var_sat_under` for the
// `valid_options` cofactor walk. The full oxidd-audit §6 surface
// (`and`/`or`/`not`/`xor`/`implies`, `restrict`, `sat_count`, etc.)
// lands in subsequent M1 tasks as the trait grows. Each addition is a
// deliberate trait-growth event that moves the trait, `NullBackend`,
// and this file together.
//
// Feature hygiene: compiles against `oxidd` with `default-features = false`
// and only the four features enumerated in `dep-audit-v0.3.0.md` §5.1:
// `bdd`, `manager-index`, `apply-cache-direct-mapped`, `multi-threading`.
// `dot-export`, `dddmp`, `visualize`, `mtbdd`, `zbdd`, `tdd`, `bcdd` are
// deliberately disabled. Switching to `manager-pointer` is a re-audit
// event per oxidd-audit §3.1.

use oxidd::bdd::{BDDFunction, BDDManagerRef};
use oxidd::{BooleanFunction, Manager, ManagerRef};

use crate::backend::{
    BackendError, CapacityHints, FormulaHandle, SolverBackend, VariableOrder,
};
use crate::ccm_format::{BddNode, TERMINAL_FALSE, TERMINAL_TRUE, TERMINAL_VAR_INDEX};

/// Default apply-cache capacity used when `CapacityHints::max_nodes` is
/// zero (which is the case for the M0 smoke test and for every empty-CCM
/// load in the scaffolding crate). oxidd's index-based manager panics if
/// the cache capacity is zero (the `apply-cache-direct-mapped` variant
/// round-trips the capacity through a bit-width calculation), so we
/// always hand it a non-zero floor. The value is small because the M0
/// backend only constructs two constants (`⊥`, `⊤`) — it does not run
/// any apply operators and therefore does not need a large cache.
const DEFAULT_APPLY_CACHE_CAPACITY: usize = 1024;

/// Default inner-node capacity used when `CapacityHints::max_nodes` is
/// zero. Mirrors `DEFAULT_APPLY_CACHE_CAPACITY` above; the empty-CCM
/// session never allocates more than the two terminal nodes for `⊥` and
/// `⊤`, but oxidd's `manager-index` variant sizes its internal hash
/// tables up front and rejects a zero here.
const DEFAULT_INNER_NODE_CAPACITY: usize = 1024;

/// Headroom multiplier on `CapacityHints::max_nodes` so the
/// `manager-index` table has room for `is_var_sat_under` cofactor
/// nodes. 8x covers the M1 query surface (configflux-8dm.7 gate).
const INNER_NODE_HEADROOM_MULTIPLIER: usize = 8;

/// Default worker-pool thread count. oxidd's `multi-threading` feature
/// (enabled in `solver/Cargo.toml`) spins a rayon worker pool at manager
/// construction time; one worker is enough for the M0 constant-only
/// workload and keeps the dev-container footprint low. Downstream M1
/// tasks that run real apply sequences will plumb this through
/// `CapacityHints` once that struct carries a `threads` field.
const DEFAULT_THREAD_COUNT: u32 = 1;

/// Pre-allocated slot holding the constant `⊥` (`false`) BDD function.
///
/// `new_session` pushes `⊥` at index 0 and `⊤` at index 1 of the
/// internal `formulas` table so that `mk_const(value)` becomes a pure
/// lookup — no closure entry into the oxidd manager per call. Downstream
/// callers holding a `FormulaHandle(0)` can rely on it pointing at `⊥`
/// for the lifetime of the backend instance; the slot is never reused.
const FALSE_SLOT: usize = 0;

/// Pre-allocated slot holding the constant `⊤` (`true`) BDD function.
/// See `FALSE_SLOT` for the rationale; this slot is never reused either.
const TRUE_SLOT: usize = 1;

/// Stable identifier for the oxidd backend, surfaced through
/// `SolverBackend::backend_id` and eventually written into
/// `ccm.manifest.json.algorithm` per ADR-0005 §2. The value is pinned
/// here as a literal so that a silent bump of the oxidd version would
/// not silently change the identifier — changing this string is a
/// deliberate review event linked to re-auditing the oxidd dep.
const OXIDD_BACKEND_ID: &str = "oxidd-v0.11";

/// Concrete `SolverBackend` wrapping oxidd's ROBDD library.
///
/// State: the `Arc`-backed `BDDManagerRef` (all ops route through it)
/// plus a `Vec<BDDFunction>` handle table indexed by `FormulaHandle.0`
/// (never shrinks, so caller-held handles stay valid for the session).
/// `BDDFunction` never leaves this module, preserving the ADR-0003 §2
/// "no oxidd types leak" invariant by construction.
///
/// `Debug` is hand-implemented because neither `BDDManagerRef` nor
/// `BDDFunction` in oxidd 0.11.0 derives `Debug` and the crate-wide
/// `deny(missing_debug_implementations)` requires one. The impl emits
/// only the handle-table length — no manager introspection, which
/// would require holding the manager lock unsafely from a formatter.
pub struct OxiddBackend {
    /// Owning reference to the oxidd BDD manager. Kept live so every
    /// `BDDFunction` in `formulas` remains valid through the `Arc`
    /// inside `BDDManagerRef`.
    manager: BDDManagerRef,
    /// Handle table mapping `FormulaHandle.0` to the oxidd-side
    /// `BDDFunction`. Entries 0 (`⊥`) / 1 (`⊤`) are pre-populated by
    /// `new_session`; `deserialize_bdd` pushes the rebuilt root.
    formulas: Vec<BDDFunction>,
    /// Per-variable literal functions materialized by `deserialize_bdd`.
    /// `var_functions[i]` is `x_i = 1`; read by `is_var_sat_under` for
    /// per-option cofactor checks without re-entering the manager.
    var_functions: Vec<BDDFunction>,
}

impl OxiddBackend {
    /// Resolve a `FormulaHandle` back to the stored `BDDFunction`.
    /// Returns `None` only for a forged/out-of-range index; handles
    /// issued by trait methods are always in-range.
    #[inline]
    fn lookup(&self, handle: FormulaHandle) -> Option<&BDDFunction> {
        self.formulas.get(handle.0)
    }
}

// See struct-level doc comment on `OxiddBackend` for the rationale
// behind the hand-rolled `Debug` impl.
impl core::fmt::Debug for OxiddBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OxiddBackend")
            .field("backend_id", &OXIDD_BACKEND_ID)
            .field("formula_count", &self.formulas.len())
            .finish_non_exhaustive()
    }
}

impl SolverBackend for OxiddBackend {
    fn new_session(
        order: VariableOrder,
        hints: CapacityHints,
    ) -> Result<Self, BackendError> {
        // `hints.max_nodes` is a lower-bound floor (M0 fallback +
        // configflux-8dm.3 cofactor headroom); the headroom
        // multiplier above leaves room for the intermediate nodes a
        // 10k cofactor walk produces (configflux-8dm.7 gate).
        let inner_nodes = hints
            .max_nodes
            .max(DEFAULT_INNER_NODE_CAPACITY)
            .checked_mul(INNER_NODE_HEADROOM_MULTIPLIER)
            .unwrap_or(usize::MAX);
        let apply_cache = DEFAULT_APPLY_CACHE_CAPACITY;

        // configflux-9pjy.4 / ADR-0039 §6: honour the soft budget's
        // requested worker-pool size when set, falling back to
        // `DEFAULT_THREAD_COUNT` when unset. oxidd's `multi-threading`
        // feature spins a rayon pool at manager construction time, so this
        // is the one place `max_threads` genuinely bites. A `Some(0)`
        // request is nonsensical (oxidd would reject a zero-worker pool),
        // so it collapses to the default — the budget is a soft hint, not
        // a hard contract, and a degenerate value must never abort a
        // session.
        let threads = match hints.threads {
            Some(n) if n > 0 => n,
            _ => DEFAULT_THREAD_COUNT,
        };

        // `oxidd::bdd::new_manager` panics on zero capacity; the max()
        // above guards against that.
        let manager = oxidd::bdd::new_manager(inner_nodes, apply_cache, threads);

        // Pre-populate the constant slots. We build `⊥` and `⊤` once,
        // up front, under a single exclusive manager borrow so that
        // subsequent `mk_const` calls become pure O(1) table lookups
        // with no re-entry into oxidd.
        let (false_fn, true_fn) = manager.with_manager_exclusive(|mgr| {
            (BDDFunction::f(mgr), BDDFunction::t(mgr))
        });

        // The handle table is seeded with exactly these two entries.
        // `FALSE_SLOT` and `TRUE_SLOT` above document the invariant.
        let mut formulas = Vec::with_capacity(2);
        formulas.push(false_fn);
        formulas.push(true_fn);

        // `order` is accepted but not consumed in the M0 surface —
        // variable introduction (`mk_var`) lands with the M1 apply
        // surface. Acknowledging it explicitly keeps the signature
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
        // This matches the NullBackend convention where `FormulaHandle(0)
        // == ⊥` and `FormulaHandle(1) == ⊤`, so code that was written
        // against NullBackend continues to read the same way under the
        // oxidd backend.
        if value {
            FormulaHandle(TRUE_SLOT)
        } else {
            FormulaHandle(FALSE_SLOT)
        }
    }

    fn is_false(&self, f: FormulaHandle) -> bool {
        // A handle represents `⊥` iff its stored BDDFunction is equal
        // (via oxidd's derived `PartialEq`) to a freshly constructed
        // `BDDFunction::f(manager)`. oxidd's BDDs are canonical, so
        // structural equality is the correct check — two BDDs are equal
        // iff they represent the same Boolean function, and the
        // constant-false function has a unique representation in a
        // ROBDD.
        let Some(stored) = self.lookup(f) else {
            return false;
        };
        self.manager
            .with_manager_shared(|mgr| *stored == BDDFunction::f(mgr))
    }

    fn is_true(&self, f: FormulaHandle) -> bool {
        // Mirror of `is_false`; see comment there for the canonicity
        // argument.
        let Some(stored) = self.lookup(f) else {
            return false;
        };
        self.manager
            .with_manager_shared(|mgr| *stored == BDDFunction::t(mgr))
    }

    fn backend_id(&self) -> &'static str {
        OXIDD_BACKEND_ID
    }

    /// Pure-query cofactor check: does `current ∧ x_var_idx` have a
    /// satisfying assignment? Computes the conjunction via oxidd's apply
    /// cache and compares it to `BDDFunction::f`; ROBDDs are canonical,
    /// so structural equality with the FALSE terminal is exactly a SAT
    /// check. The intermediate `BDDFunction` is dropped before return,
    /// which is what makes this `&self` and therefore compatible with
    /// ADR-0003 §4's pure-query pin on `Session::valid_options`.
    fn is_var_sat_under(
        &self,
        current: FormulaHandle,
        var_idx: u32,
    ) -> Result<bool, BackendError> {
        let current_fn = self.lookup(current).ok_or(BackendError::Invariant(
            "is_var_sat_under(): current handle out of range",
        ))?;
        let var_fn = self.var_functions.get(var_idx as usize).ok_or(
            BackendError::Invariant("is_var_sat_under(): var_idx out of range"),
        )?;
        let conj = current_fn.and(var_fn).map_err(|_| BackendError::OutOfCapacity)?;
        let is_false = self
            .manager
            .with_manager_shared(|mgr| conj == BDDFunction::f(mgr));
        Ok(!is_false)
    }

    /// Conjoin the formula behind `current` with the unit clause
    /// `x_var_idx = 1` and push the result into the handle table.
    ///
    /// Unlike `is_var_sat_under` — which drops the intermediate
    /// `BDDFunction` after a SAT check — `apply_and` keeps the
    /// resulting BDD alive on `self.formulas` so `Session::apply`
    /// can swap it in as the new active-formula handle and keep the
    /// old one on its undo stack. The returned handle indexes into
    /// the same table as handles issued by `deserialize_bdd`; handle
    /// ids never wrap because `self.formulas` never shrinks.
    ///
    /// On an unsat conjunction (the oxidd call returns structural
    /// equality with `BDDFunction::f(mgr)`) we still push the FALSE
    /// function into the handle table and return a handle to it —
    /// the caller uses `is_false` on that handle to detect the
    /// conflict. This keeps the API symmetric across sat/unsat
    /// results and avoids a dedicated `Err` variant here (the error
    /// translation into `Session::Error::Conflict` lives in
    /// `Session::apply`).
    fn apply_and(
        &mut self,
        current: FormulaHandle,
        var_idx: u32,
    ) -> Result<FormulaHandle, BackendError> {
        let current_fn = self
            .lookup(current)
            .ok_or(BackendError::Invariant(
                "apply_and(): current handle out of range",
            ))?
            .clone();
        let var_fn = self
            .var_functions
            .get(var_idx as usize)
            .ok_or(BackendError::Invariant("apply_and(): var_idx out of range"))?
            .clone();
        // oxidd's `and` returns an `AllocResult`; the only documented
        // Err variant is node-table capacity exhaustion. Map it onto
        // the same `OutOfCapacity` variant `is_var_sat_under` uses so
        // the solver surface stays uniform.
        let conj = current_fn
            .and(&var_fn)
            .map_err(|_| BackendError::OutOfCapacity)?;
        let idx = self.formulas.len();
        self.formulas.push(conj);
        Ok(FormulaHandle(idx))
    }

    /// Rebuild an oxidd BDD from the ADR-0005 §4 canonical node table.
    ///
    /// Algorithm (bottom-up walk in post-order):
    ///
    ///   1. Allocate `var_count` new oxidd variables in the manager via
    ///      `Manager::add_vars`. This is a single exclusive-locked call
    ///      — variables added here stay alive for the backend's
    ///      lifetime. Variable `i` in the on-disk `var_index` is the
    ///      oxidd variable with `VarNo = i` under the natural mapping.
    ///   2. Pre-allocate `BDDFunction::t(mgr)` and `BDDFunction::f(mgr)`
    ///      to resolve the TERMINAL_TRUE / TERMINAL_FALSE sentinels
    ///      without re-entering the manager per lookup.
    ///   3. Walk `nodes` in increasing index order. For every terminal
    ///      record (index 0 = FALSE, index 1 = TRUE per §4) push the
    ///      corresponding pre-allocated function into a parallel
    ///      `rebuilt` table. For every non-terminal record resolve
    ///      `low_id` and `high_id` (via sentinels or into `rebuilt`)
    ///      and construct `var.ite(high, low)`. This follows the
    ///      standard shannon-expansion identity:
    ///        f(x_0, …, x_n) = ite(x_top, f|_{x_top=1}, f|_{x_top=0})
    ///      = (x_top ∧ high) ∨ (¬x_top ∧ low)
    ///      which is exactly what a BDD node encodes.
    ///   4. Resolve the `root` parameter (sentinel or index) against
    ///      `rebuilt`, push the resulting `BDDFunction` into
    ///      `self.formulas`, and return the new `FormulaHandle`.
    ///
    /// Error handling: any oxidd allocation failure (`AllocResult::Err`)
    /// surfaces as `BackendError::OutOfCapacity` because oxidd's only
    /// documented Err variant from `ite` / `var` is capacity exhaustion.
    /// An out-of-range `var_index`, an out-of-range child id, or a
    /// malformed root is reported as `BackendError::Serialization` with
    /// a static message — those cases should have been caught by
    /// `ccm_format::parse_bdd_bin`; reaching them here means a caller
    /// bypassed the format reader with a hand-built node table, which
    /// is OK in tests but not in production.
    fn deserialize_bdd(
        &mut self,
        var_count: u32,
        nodes: &[BddNode],
        root: u32,
    ) -> Result<FormulaHandle, BackendError> {
        // Step 1: allocate `var_count` fresh variables under a single
        // exclusive manager borrow. The returned range is the VarNo block we
        // index with the on-disk `var_index`. The vector is cached on
        // `self.var_functions` so per-variable cofactor queries in
        // `valid_options` (configflux-8dm.3) can look up the literal function
        // without re-entering the manager.
        //
        // This runs BEFORE the trivial-terminal fast path below. A root that
        // reduces to ⊤ or ⊥ still declares `var_count` variables in the symbol
        // table, and `is_var_sat_under` / `apply_and` index `self.var_functions`
        // by that symbol-table var index. ADR-0047 Amendment 1 makes this
        // reachable: a declaration-only model's synthesized clauses are all
        // symbol-introduction tautologies, so the root is ⊤ while `var_count`
        // (the declared symbols) is non-zero. Registering the variables here
        // keeps per-arm `valid_options` queries in range — every declared arm is
        // satisfiable under ⊤. An empty CCM (`var_count == 0`) allocates
        // nothing, exactly as before.
        let var_functions: Vec<BDDFunction> = if var_count == 0 {
            Vec::new()
        } else {
            self.manager.with_manager_exclusive(
                |mgr| -> Result<Vec<BDDFunction>, BackendError> {
                    let range = mgr.add_vars(var_count);
                    let mut vars = Vec::with_capacity(var_count as usize);
                    for v in range {
                        let var_fn = BDDFunction::var(mgr, v)
                            .map_err(|_| BackendError::OutOfCapacity)?;
                        vars.push(var_fn);
                    }
                    Ok(vars)
                },
            )?
        };
        self.var_functions = var_functions;

        // Fast path for trivial-terminal roots. An empty CCM (var_count == 0)
        // hits this branch; so does any model whose constraints reduce to ⊥ or
        // ⊤ (including a declaration-only facet model under ADR-0047 Amendment
        // 1). The variables are already registered above, so a subsequent
        // `valid_options` cofactor query stays in range.
        if root == TERMINAL_TRUE {
            return Ok(self.mk_const(true));
        }
        if root == TERMINAL_FALSE {
            return Ok(self.mk_const(false));
        }

        // Non-trivial root. We must materialize every node in the table into an
        // oxidd function, then pick the index named by `root`.

        // Step 2: snapshot ⊥ / ⊤ for sentinel resolution.
        let (false_fn, true_fn) = self
            .manager
            .with_manager_shared(|mgr| (BDDFunction::f(mgr), BDDFunction::t(mgr)));

        // Step 3: walk the node table in post-order (index-ascending).
        // `rebuilt[i]` holds the oxidd function for the node at on-disk
        // index `i`; it is indexed only after a node has been processed,
        // so post-order is what makes the forward references valid.
        let mut rebuilt: Vec<Option<BDDFunction>> = vec![None; nodes.len()];
        for (i, node) in nodes.iter().enumerate() {
            if node.var_index == TERMINAL_VAR_INDEX {
                // §4: index 0 = FALSE, index 1 = TRUE. Anything else
                // tagged as terminal is a parser bug.
                if i == 0 {
                    rebuilt[i] = Some(false_fn.clone());
                } else if i == 1 {
                    rebuilt[i] = Some(true_fn.clone());
                } else {
                    return Err(BackendError::Serialization(
                        "deserialize_bdd: terminal-tagged node at unexpected index",
                    ));
                }
                continue;
            }
            if node.var_index as usize >= self.var_functions.len() {
                return Err(BackendError::Serialization(
                    "deserialize_bdd: node var_index out of range",
                ));
            }
            let low = lookup_child(node.low_id, &rebuilt, &false_fn, &true_fn)?;
            let high = lookup_child(node.high_id, &rebuilt, &false_fn, &true_fn)?;
            // Shannon expansion: node = (var ∧ high) ∨ (¬var ∧ low) = ite(var, high, low).
            let var_fn = &self.var_functions[node.var_index as usize];
            let built = var_fn
                .ite(&high, &low)
                .map_err(|_| BackendError::OutOfCapacity)?;
            rebuilt[i] = Some(built);
        }

        // Step 4: resolve root.
        let root_idx = root as usize;
        if root_idx >= rebuilt.len() {
            return Err(BackendError::Serialization(
                "deserialize_bdd: root index out of range",
            ));
        }
        let root_fn = rebuilt[root_idx]
            .take()
            .ok_or(BackendError::Serialization(
                "deserialize_bdd: root references an unprocessed node slot",
            ))?;

        let idx = self.formulas.len();
        self.formulas.push(root_fn);
        Ok(FormulaHandle(idx))
    }
}

/// Resolve a `(low_id, high_id)`-style child reference from a node
/// record: a sentinel points at ⊥ / ⊤ directly; anything else indexes
/// into the in-progress `rebuilt` vector. The return is a cheap clone
/// of an `Arc`-backed `BDDFunction` — oxidd's `BDDFunction` is `Clone`
/// and derives its equality from the underlying edge, so cloning is
/// cheap and does not allocate BDD nodes.
fn lookup_child(
    id: u32,
    rebuilt: &[Option<BDDFunction>],
    false_fn: &BDDFunction,
    true_fn: &BDDFunction,
) -> Result<BDDFunction, BackendError> {
    if id == TERMINAL_FALSE {
        return Ok(false_fn.clone());
    }
    if id == TERMINAL_TRUE {
        return Ok(true_fn.clone());
    }
    let idx = id as usize;
    if idx >= rebuilt.len() {
        return Err(BackendError::Serialization(
            "deserialize_bdd: child index out of range",
        ));
    }
    rebuilt[idx]
        .as_ref()
        .cloned()
        .ok_or(BackendError::Serialization(
            "deserialize_bdd: child references an unprocessed node slot",
        ))
}

#[cfg(test)]
mod tests {
    //! Unit tests for the oxidd-backed `SolverBackend` implementation.
    //!
    //! These tests live inside the backend module so they can touch
    //! internal fields (`formulas`, `manager`) that the public
    //! `SolverBackend` surface does not expose. The external integration
    //! coverage — which proves that no oxidd type leaks across the crate
    //! boundary — lives in `solver/tests/round_trip_empty.rs`.
    use super::*;

    #[test]
    fn new_session_seeds_constant_slots() {
        // new_session must pre-populate FALSE_SLOT and TRUE_SLOT so
        // that `mk_const` becomes a pure lookup. This test pins that
        // invariant: if a refactor dropped the preload, the length
        // assertion would fail.
        let backend = OxiddBackend::new_session(
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
        let mut backend = OxiddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        assert_eq!(backend.mk_const(false), FormulaHandle(FALSE_SLOT));
        assert_eq!(backend.mk_const(true), FormulaHandle(TRUE_SLOT));
        // Repeated calls must not grow the handle table — `mk_const` on
        // a constant is idempotent at the wrapper level.
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
        let mut backend = OxiddBackend::new_session(
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
        // A handle whose index is out of range must not panic — the
        // trait documents handles as opaque, and the wrapper must
        // degrade gracefully on forgery rather than risk undefined
        // behavior by indexing past the vector.
        let backend = OxiddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        let forged = FormulaHandle(9_999);
        assert!(!backend.is_true(forged));
        assert!(!backend.is_false(forged));
    }

    #[test]
    fn new_session_honors_capacity_hints_threads() {
        // configflux-9pjy.4 / ADR-0039 §6: the runtime solver session
        // worker-pool size must come from `CapacityHints::threads` when
        // set, not the hardcoded `DEFAULT_THREAD_COUNT`. We cannot read
        // oxidd's worker count back through the public API, so this test
        // pins the *observable* contract: a session constructed with an
        // explicit thread count builds successfully and runs a real apply
        // (rebuilding a 1-variable BDD and cofactoring it) on the
        // multi-worker manager. A regression that ignored `threads` and
        // hardcoded 1 would still pass; a regression that mis-wired the
        // value (e.g. passed 0, which oxidd rejects) would panic or fail
        // here. The default-None fallback is covered by the other tests,
        // which all construct with `DEFAULT_THREAD_COUNT` implicitly.
        let hints = CapacityHints {
            max_nodes: 1024,
            threads: Some(2),
        };
        let mut backend = OxiddBackend::new_session(VariableOrder::empty(), hints)
            .expect("new_session must succeed with an explicit thread count");

        // Rebuild a trivial single-variable BDD (x0) so the multi-worker
        // manager actually performs an allocation + a cofactor query.
        let nodes = vec![
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_FALSE,
                high_id: TERMINAL_FALSE,
                flags: 0,
            },
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_TRUE,
                high_id: TERMINAL_TRUE,
                flags: 0,
            },
            BddNode {
                var_index: 0,
                low_id: 0,
                high_id: 1,
                flags: 0,
            },
        ];
        let root = backend
            .deserialize_bdd(1, &nodes, 2)
            .expect("deserialize x0 on a multi-worker manager");
        assert!(
            backend
                .is_var_sat_under(root, 0)
                .expect("cofactor query on the multi-worker manager"),
            "x0 ∧ x0 must be satisfiable"
        );
    }

    #[test]
    fn deserialize_trivial_true_root_still_registers_declared_variables() {
        // configflux-5zqr (ADR-0047 Amendment 1): a declaration-only declared
        // facet's synthesized clauses are all symbol-introduction tautologies, so
        // the emitted BDD root reduces to ⊤ while the symbol table still names
        // `var_count` variables. `deserialize_bdd` must register those variables
        // BEFORE its trivial-terminal fast path, or a per-arm `valid_options`
        // cofactor query (`is_var_sat_under`) indexes out of range — the exact
        // pre-fix Invariant("is_var_sat_under(): var_idx out of range") this
        // asserts against. Mirrors CuddBackend, which already allocates before
        // its terminal fast path (see cudd_translate::canonical_to_cudd).
        let mut backend = OxiddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");

        // A ⊤ root over a two-variable symbol table. The nodes table carries the
        // two terminal sentinels (index 0 = ⊥, index 1 = ⊤) the serializer always
        // emits; the root names the ⊤ terminal directly, so the fast path is
        // taken and no interior node references either variable.
        let nodes = vec![
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_FALSE,
                high_id: TERMINAL_FALSE,
                flags: 0,
            },
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_TRUE,
                high_id: TERMINAL_TRUE,
                flags: 0,
            },
        ];
        let root = backend
            .deserialize_bdd(2, &nodes, TERMINAL_TRUE)
            .expect("deserialize a ⊤ root over a 2-variable table");
        assert!(backend.is_true(root), "a TERMINAL_TRUE root loads as ⊤");
        // Both declared variables must be in range and satisfiable under ⊤
        // (⊤ ∧ x_i reduces to x_i, which is sat), so `valid_options` would
        // enumerate every declared arm rather than faulting.
        for var_idx in 0u32..2 {
            assert!(
                backend
                    .is_var_sat_under(root, var_idx)
                    .expect("cofactor query must stay in range for a declared var"),
                "x{var_idx} must be satisfiable under a ⊤ root"
            );
        }
    }

    #[test]
    fn backend_id_is_stable() {
        let backend = OxiddBackend::new_session(
            VariableOrder::empty(),
            CapacityHints::default(),
        )
        .expect("new_session must succeed");
        assert_eq!(backend.backend_id(), OXIDD_BACKEND_ID);
        assert_eq!(backend.backend_id(), "oxidd-v0.11");
    }
}
