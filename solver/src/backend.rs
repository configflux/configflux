// SPDX-License-Identifier: BUSL-1.1
//
// Internal boundary between `solver::Session` and the concrete BDD library.
//
// Per ADR-0003 Section 3 and `docs/security/oxidd-audit.md` Section 6, this
// trait isolates the rest of the crate from oxidd's internal node encoding,
// allocation discipline, and DDDMP serialization limitations. In this
// scaffolding task the trait carries only the minimum surface required to
// let `Session` stubs compile and run; the full oxidd-audit Section 6
// surface (formula construction, cofactoring, canonical serialization, etc.)
// lands in the M1 implementation task `configflux-8dm`. This file does not
// import `oxidd`; the one file that will is `solver/src/backend_oxidd.rs`,
// added in M1.

use core::fmt;

/// Opaque handle to a Boolean variable inside a backend session.
///
/// Wraps whatever the backend uses internally (an oxidd `BDDFunction`, a
/// BuDDy `bdd_t`, a batsat `Lit`, etc.). Callers treat it as an opaque
/// token; no field is exposed through the public `Session` API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarHandle(pub(crate) usize);

/// Opaque handle to a Boolean formula (a BDD or equivalent representation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormulaHandle(pub(crate) usize);

/// A canonical variable order, computed by a backend-agnostic orderer and
/// passed into the backend at session-creation time. Never mutated after.
///
/// M1 will populate this from the loaded `.ccm` symbols; in this scaffolding
/// task it is a thin owned wrapper around a `Vec<String>` so callers can
/// construct an empty order for the smoke test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariableOrder(pub(crate) Vec<String>);

impl VariableOrder {
    /// Construct an empty order. Used for sessions bound to the
    /// empty-Ccm stub where no BDD variables exist.
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Construct an order from a concrete list of facet names (as
    /// surfaced by `Ccm::symbols()`). The list is stored in BDD
    /// variable-index order: `names[0]` is the facet bound to BDD
    /// variable 0, and so on.
    pub fn from_names(names: Vec<String>) -> Self {
        Self(names)
    }

    /// Number of variables in the order.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the order is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Stable error enum for backend operations. Maps backend-specific
/// allocation, format, and I/O errors to a solver-facing vocabulary. The
/// full variant list lands with `configflux-8dm`; this scaffolding set is
/// the minimum needed for the `Session` stubs to compile and for
/// `NullBackend::new_session` to be infallible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The backend ran out of its capacity budget (node table, apply cache,
    /// or equivalent). Stable variant across backends.
    OutOfCapacity,
    /// A formula became unsatisfiable under the caller's current
    /// constraints. `Session` maps this into a user-facing rejection.
    Unsat,
    /// An invariant internal to the backend was violated. Carries a
    /// static string for the call site that detected it.
    Invariant(&'static str),
    /// Serialization to or from `ccm.bdd.bin` failed. M1 will refine this
    /// with format-specific detail.
    Serialization(&'static str),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::OutOfCapacity => write!(f, "backend ran out of capacity"),
            BackendError::Unsat => write!(f, "formula is inconsistent (unsat)"),
            BackendError::Invariant(msg) => write!(f, "invariant violated: {msg}"),
            BackendError::Serialization(msg) => write!(f, "serialization failed: {msg}"),
        }
    }
}

impl std::error::Error for BackendError {}

/// Capacity hints passed at backend construction time. Kept intentionally
/// small in the scaffolding task; M1 extends this with apply-cache size and
/// thread count per `dep-audit-v0.3.0.md` Section 5.1 oxidd feature hygiene.
#[derive(Debug, Clone, Copy, Default)]
pub struct CapacityHints {
    /// Suggested upper bound for backend node table entries.
    pub max_nodes: usize,
    /// Requested worker-pool size for the runtime solver session
    /// (configflux-9pjy.4 / ADR-0039 §6). This is the plumb point for the
    /// soft resource budget's `max_threads` knob: a caller that wants to
    /// bound (or widen) the oxidd `multi-threading` worker pool sets it
    /// here. `None` ⇒ the backend's historical `DEFAULT_THREAD_COUNT`, so
    /// the default session is byte- and behaviour-identical to before this
    /// field existed. The value bites only where parallelism actually
    /// exists (the oxidd-backed session); the in-crate compile is
    /// single-threaded recursion and ignores it (ADR-0039 §Context
    /// honesty flag).
    pub threads: Option<u32>,
}

/// The solver-internal boundary trait. `Session` calls through this trait
/// for every BDD operation; no file outside `solver/src/backend_oxidd.rs`
/// (added in M1) may call `oxidd::*` directly.
///
/// # Scaffolding surface
///
/// This trait carries only the methods required for the M0 `Session`
/// stubs and smoke test to compile. The full surface — cofactoring,
/// canonical serialize/deserialize, sat-count, pick-one, introspection —
/// lands in `configflux-8dm` (M1) alongside `backend_oxidd.rs`. Adding a
/// method here is a deliberate trait-growth event and requires the
/// M1 implementation to keep the oxidd wrapper single-file per the
/// `oxidd-audit.md` Section 5 mitigation plan.
pub trait SolverBackend: Send {
    /// Construct a new backend session bound to a canonical variable order
    /// and initial capacity hints. The order is owned by the backend for
    /// the session lifetime and never changes (sifting is deferred; see
    /// ADR-0004 Section 2).
    fn new_session(
        order: VariableOrder,
        hints: CapacityHints,
    ) -> Result<Self, BackendError>
    where
        Self: Sized;

    /// Construct the constant Boolean formula for `value` (`true` or
    /// `false`). Used by the `NullBackend` smoke test and by M1's
    /// `Session::new` to seed `current` before loading a CCM.
    fn mk_const(&mut self, value: bool) -> FormulaHandle;

    /// True if the given handle represents the logical `false` formula.
    fn is_false(&self, f: FormulaHandle) -> bool;

    /// True if the given handle represents the logical `true` formula.
    fn is_true(&self, f: FormulaHandle) -> bool;

    /// Human-readable backend identifier, surfaced in
    /// `ccm.manifest.json.algorithm`. Stable across the backend's
    /// lifetime of a given solver version.
    fn backend_id(&self) -> &'static str;

    /// Rebuild a BDD from its `ccm.bdd.bin` canonical form per ADR-0005
    /// §4 and return a `FormulaHandle` pointing at the primary root.
    ///
    /// Contract (configflux-8dm.2):
    ///
    ///   - `var_count` is the Boolean variable count the backend must
    ///     materialize before any non-terminal node is reconstructed.
    ///     Backends that carry a lazy variable table (oxidd) allocate
    ///     them here; backends that do not (NullBackend) may treat the
    ///     count as a no-op.
    ///   - `nodes` is the fully-parsed node table from the on-disk BDD.
    ///     Terminal records appear at indices 0 (FALSE) and 1 (TRUE)
    ///     per §4; non-terminal records follow in post-order.
    ///   - `root` is the primary root sentinel or node index from
    ///     `root_table[0]`; sentinel values `0xFFFF_FFFE` and
    ///     `0xFFFF_FFFF` denote the terminal TRUE and FALSE respectively.
    ///
    /// The default impl on this trait rebuilds as a pure constant
    /// (TRUE if `root == TERMINAL_TRUE`, FALSE otherwise) and ignores
    /// the node table. This keeps `NullBackend` and other stub backends
    /// working without per-node walks; real backends (OxiddBackend)
    /// override with a node-by-node `ite` walk.
    /// Return true iff `current ∧ x_var_idx` is satisfiable. A pure
    /// query: computes the conjunction, checks for the FALSE terminal,
    /// and drops the intermediate BDD without mutating any caller-
    /// visible backend state. Used by `Session::valid_options`
    /// (configflux-8dm.3) to run the per-option cofactor walk described
    /// in ADR-0004 §1: for every candidate `(facet, value)` variable
    /// `i`, `is_var_sat_under(current, i)` tells the session whether
    /// `value` is still a valid option under the current formula.
    ///
    /// The default impl returns `BackendError::Invariant` because the
    /// scaffolding backends (`NullBackend`) do not carry a real BDD.
    /// The oxidd wrapper overrides this with a direct call into the
    /// manager's apply cache.
    fn is_var_sat_under(
        &self,
        current: FormulaHandle,
        var_idx: u32,
    ) -> Result<bool, BackendError> {
        let _ = (current, var_idx);
        Err(BackendError::Invariant(
            "backend does not implement is_var_sat_under(); cannot cofactor",
        ))
    }

    /// Conjoin `current` with the unit clause `x_var_idx = 1` and
    /// return a new `FormulaHandle` pointing at the resulting BDD.
    ///
    /// Contract (configflux-8dm.4):
    ///
    ///   - `current` is the session's active-formula handle.
    ///   - `var_idx` is the Boolean variable for the `(facet, value)`
    ///     pair the caller is applying, as resolved by the
    ///     `Symbols::var_for_facet("{facet}.{value}")` lookup.
    ///   - Returns a fresh `FormulaHandle` owned by the backend —
    ///     structural equality with the FALSE terminal (checked via
    ///     `is_false` on the returned handle) tells the caller that
    ///     the conjunction is unsat. The caller is responsible for
    ///     translating that into a typed `Error::Conflict`.
    ///   - The method does not mutate `current`; the handle stays
    ///     valid so `Session::apply` can leave the old root on its
    ///     undo stack and `Session::retract` can restore it.
    ///
    /// The default impl on this trait returns
    /// `BackendError::Invariant` because the scaffolding backends
    /// (`NullBackend`) do not carry a real BDD. The oxidd wrapper
    /// overrides this with a direct call into the manager's apply
    /// cache.
    fn apply_and(
        &mut self,
        current: FormulaHandle,
        var_idx: u32,
    ) -> Result<FormulaHandle, BackendError> {
        let _ = (current, var_idx);
        Err(BackendError::Invariant(
            "backend does not implement apply_and(); cannot conjoin",
        ))
    }

    fn deserialize_bdd(
        &mut self,
        _var_count: u32,
        _nodes: &[crate::ccm_format::BddNode],
        root: u32,
    ) -> Result<FormulaHandle, BackendError> {
        // Default: interpret only the root sentinel. This is correct for
        // the empty-CCM path (where the full formula is trivially TRUE
        // or FALSE) and for any backend that cannot faithfully
        // reconstruct a variable-valued BDD. Concrete backends that do
        // know how to rebuild (OxiddBackend) override this method with
        // a full node-table walk.
        let value = match root {
            crate::ccm_format::TERMINAL_TRUE => true,
            crate::ccm_format::TERMINAL_FALSE => false,
            _ => {
                return Err(BackendError::Serialization(
                    "backend lacks deserialize_bdd override; cannot rebuild variable-valued BDD",
                ));
            }
        };
        Ok(self.mk_const(value))
    }
}

/// A no-op `SolverBackend` used for compile-link smoke tests and by the
/// scaffolding `Session` stubs before M1 wires the oxidd backend.
///
/// The implementation is deliberately minimal: it holds no BDD state, it
/// serves constants as integer tokens, and it never reports a formula as
/// `false`. This is the test backend referenced by ADR-0003 Section
/// "Alternatives D"; it is also what lets downstream consumers sanity-check
/// the `Session` API shape without pulling in `oxidd`.
#[derive(Debug, Default)]
pub struct NullBackend {
    order: VariableOrder,
    hints: CapacityHints,
}

impl NullBackend {
    /// Number of variables the null backend was constructed with. Used by
    /// the smoke test to assert the constructor round-trips its inputs.
    pub fn var_count(&self) -> usize {
        self.order.len()
    }

    /// Capacity hints the backend was constructed with.
    pub fn hints(&self) -> CapacityHints {
        self.hints
    }
}

impl SolverBackend for NullBackend {
    fn new_session(order: VariableOrder, hints: CapacityHints) -> Result<Self, BackendError> {
        Ok(Self { order, hints })
    }

    fn mk_const(&mut self, value: bool) -> FormulaHandle {
        // Encode `true` as 1 and `false` as 0; this matches the convention
        // in the oxidd-audit sketch and keeps `is_true` / `is_false` O(1).
        FormulaHandle(value as usize)
    }

    fn is_false(&self, f: FormulaHandle) -> bool {
        f.0 == 0
    }

    fn is_true(&self, f: FormulaHandle) -> bool {
        f.0 == 1
    }

    fn backend_id(&self) -> &'static str {
        "null-v0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_backend_round_trip_is_empty() {
        let order = VariableOrder::empty();
        let hints = CapacityHints::default();
        let mut backend = NullBackend::new_session(order, hints).expect("null backend is infallible");
        assert_eq!(backend.var_count(), 0);
        assert_eq!(backend.backend_id(), "null-v0");
        let t = backend.mk_const(true);
        let f = backend.mk_const(false);
        assert!(backend.is_true(t));
        assert!(backend.is_false(f));
        assert!(!backend.is_false(t));
        assert!(!backend.is_true(f));
    }

    #[test]
    fn capacity_hints_threads_defaults_to_none() {
        // configflux-9pjy.4 / ADR-0039 §6: `threads` is the plumb point
        // for the runtime solver session's worker count. The default
        // (`CapacityHints::default()`) MUST leave it `None` so existing
        // callers — every M0/M1 session that does not set a budget — keep
        // the historical single-threaded `DEFAULT_THREAD_COUNT` behaviour
        // unchanged.
        let hints = CapacityHints::default();
        assert_eq!(hints.threads, None);
    }

    #[test]
    fn capacity_hints_carries_explicit_threads() {
        // A caller that derives a thread count from `ResourceBudget
        // .max_threads` sets it here; the field round-trips so the oxidd
        // backend can read it at `new_session`.
        let hints = CapacityHints {
            max_nodes: 4096,
            threads: Some(4),
        };
        assert_eq!(hints.threads, Some(4));
        assert_eq!(hints.max_nodes, 4096);
    }

    #[test]
    fn backend_error_display_is_stable() {
        // The wire-facing variants above each map to a fixed string so
        // that daemon transport error envelopes in v0.4.0 have a stable
        // `message` field. Pinning them here catches silent drift.
        assert_eq!(
            format!("{}", BackendError::OutOfCapacity),
            "backend ran out of capacity"
        );
        assert_eq!(
            format!("{}", BackendError::Unsat),
            "formula is inconsistent (unsat)"
        );
        assert_eq!(
            format!("{}", BackendError::Invariant("oops")),
            "invariant violated: oops"
        );
        assert_eq!(
            format!("{}", BackendError::Serialization("bad")),
            "serialization failed: bad"
        );
    }
}
