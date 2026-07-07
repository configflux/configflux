// SPDX-License-Identifier: BUSL-1.1
//
// Internal boundary between `solver::Session` and the concrete CDCL SAT
// library used for deletion-based minimal-unsatisfiable-subset (MUS)
// extraction.
//
// Per ADR-0004 §4, the CDCL SAT solver is embedded **only** for MUS
// extraction inside `Session::explain_rejection`. It is not a primary
// solving backend: `valid_options`, `apply`, `retract`, and `resolve`
// all stay on the `SolverBackend` (ROBDD) side. This trait is the
// narrower analogue of `SolverBackend` (`solver/src/backend.rs`): it
// isolates the rest of the crate from the concrete SAT library's
// internal literal encoding and clause-database discipline.
//
// ADR-0004 §4 (AC-5 mitigation leg): wrapping `batsat` behind this trait
// is exactly what makes a future swap to `cadical`, `kissat` (via FFI),
// or another CDCL library mechanical. No file in `solver/` outside
// `solver/src/sat_backend_batsat.rs` may `use batsat::*`; every other
// file — including `solver::Session` — calls through `SatBackend`. The
// confinement is enforced by a grep check in CI.
//
// # Surface
//
// The trait carries only the methods deletion-based MUS extraction
// (Marques-Silva & Lynce 2011) actually needs:
//
//   - variable allocation (`new_var`);
//   - clause addition (`add_clause`);
//   - incremental solving under a set of assumption literals
//     (`solve_with_assumptions`);
//   - conflict/reason extraction, i.e. the unsat core as a subset of the
//     supplied assumptions (`unsat_core`, `core_contains`).
//
// `Session::explain_rejection` (a separate issue, `configflux-kv5d`)
// serializes the BDD-committed feasible space into CNF, adds the
// candidate `(facet, option)` unit clause as an assumption, confirms
// UNSAT, and then runs deletion-based shrinking over the clause set using
// exactly this surface. Model-value introspection (`get_model`,
// `value_lit`) is intentionally absent: MUS extraction never reads a
// satisfying assignment, so adding it here would be unused surface and a
// re-audit liability on every SAT-library bump.

use core::fmt;

/// Opaque handle to a Boolean variable inside a SAT solver session.
///
/// Wraps whatever the backend uses internally (a `batsat::Var`, a
/// MiniSat literal index, etc.). Callers treat it as an opaque token; no
/// concrete SAT-library type is exposed through this trait or through the
/// `Session` API. The wrapped `u32` is the backend-agnostic variable
/// index in allocation order: the first `new_var` returns `SatVar(0)`,
/// the second `SatVar(1)`, and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SatVar(pub(crate) u32);

impl SatVar {
    /// The backend-agnostic variable index, in allocation order. Stable
    /// for the lifetime of the solver session that issued it.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// Opaque handle to a literal: a variable together with a polarity.
///
/// `polarity == true` is the positive literal (`x`); `polarity == false`
/// is the negated literal (`¬x`). Like `SatVar`, no concrete SAT-library
/// type leaks: the backend maps `SatLit` to its own literal
/// representation behind the trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SatLit {
    var: SatVar,
    polarity: bool,
}

impl SatLit {
    /// The positive literal for `var` (the assignment `var = true`).
    pub fn positive(var: SatVar) -> Self {
        Self {
            var,
            polarity: true,
        }
    }

    /// The negative literal for `var` (the assignment `var = false`).
    pub fn negative(var: SatVar) -> Self {
        Self {
            var,
            polarity: false,
        }
    }

    /// The underlying variable handle.
    pub fn var(self) -> SatVar {
        self.var
    }

    /// `true` for the positive literal, `false` for the negated literal.
    pub fn polarity(self) -> bool {
        self.polarity
    }
}

/// Result of an incremental solve under assumptions.
///
/// `Unknown` is reported when the backend stops on a resource limit
/// (conflict / propagation budget) rather than proving SAT or UNSAT.
/// Deletion-based MUS extraction treats `Unknown` as a hard error at the
/// caller (a sound MUS cannot be derived from an inconclusive solve); the
/// conversion is the caller's responsibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatOutcome {
    /// A satisfying assignment exists under the supplied assumptions.
    Sat,
    /// The formula is unsatisfiable under the supplied assumptions. The
    /// unsat core (a subset of those assumptions) is then available via
    /// [`SatBackend::unsat_core`].
    Unsat,
    /// The solver hit a resource limit before deciding satisfiability.
    Unknown,
}

/// Stable error enum for SAT-backend operations. Maps backend-specific
/// failure modes to a solver-facing vocabulary, mirroring `BackendError`
/// on the BDD side (`solver/src/backend.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SatError {
    /// A clause was added that drove the solver into a permanently-unsat
    /// state at the top level (the SAT library reported the clause set
    /// unsatisfiable on insertion). Surfaced so the caller can decide
    /// whether that is expected (it is, for the contradiction that seeds
    /// an MUS query) or an invariant violation.
    TopLevelUnsat,
    /// The unsat core was requested but the most recent solve did not
    /// return `Unsat`. Calling `unsat_core` is only valid immediately
    /// after a solve that reported [`SatOutcome::Unsat`].
    NoCoreAvailable,
    /// An invariant internal to the backend was violated. Carries a
    /// static string for the call site that detected it.
    Invariant(&'static str),
}

impl fmt::Display for SatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SatError::TopLevelUnsat => {
                write!(f, "clause set became unsatisfiable at the top level")
            }
            SatError::NoCoreAvailable => {
                write!(f, "no unsat core: last solve did not return unsat")
            }
            SatError::Invariant(msg) => write!(f, "invariant violated: {msg}"),
        }
    }
}

impl std::error::Error for SatError {}

/// The solver-internal CDCL boundary trait. `Session::explain_rejection`
/// calls through this trait for every SAT operation; no file outside
/// `solver/src/sat_backend_batsat.rs` may call `batsat::*` directly
/// (ADR-0004 §4).
///
/// # Adding a method
///
/// This trait carries only the deletion-based MUS-extraction surface.
/// Growing it is a deliberate event: every method added here is surface
/// the concrete SAT library must expose and that the next dependency
/// audit must re-confirm. The MUS algorithm needs clause addition,
/// assumption-scoped solving, and core extraction — nothing else.
pub trait SatBackend {
    /// Construct a fresh SAT solver session with an empty clause database
    /// and no variables.
    fn new_solver() -> Self
    where
        Self: Sized;

    /// Allocate a new Boolean variable and return its opaque handle. The
    /// returned `SatVar` carries the next index in allocation order.
    fn new_var(&mut self) -> SatVar;

    /// Add a clause (a disjunction of the supplied literals) to the
    /// solver's clause database.
    ///
    /// Returns `Ok(())` on success. If the addition drives the solver
    /// into a top-level-unsat state (the empty clause is derivable from
    /// the current database), returns `Err(SatError::TopLevelUnsat)` —
    /// the caller decides whether that is the expected contradiction for
    /// an MUS query or a bug. Adding a clause that mentions a `SatVar`
    /// this backend never issued is an `Err(SatError::Invariant)`.
    fn add_clause(&mut self, literals: &[SatLit]) -> Result<(), SatError>;

    /// Search for a model that satisfies the clause database together
    /// with every literal in `assumptions`.
    ///
    /// The assumptions are the literals that may appear in the unsat
    /// core: after a result of [`SatOutcome::Unsat`], [`unsat_core`] returns
    /// the subset of `assumptions` sufficient to prove unsatisfiability,
    /// which is the input deletion-based MUS extraction shrinks.
    ///
    /// [`unsat_core`]: SatBackend::unsat_core
    fn solve_with_assumptions(
        &mut self,
        assumptions: &[SatLit],
    ) -> Result<SatOutcome, SatError>;

    /// Return the unsat core: the subset of the most recent solve's
    /// assumptions that is sufficient to prove unsatisfiability.
    ///
    /// Precondition: the most recent `solve_with_assumptions` returned
    /// [`SatOutcome::Unsat`]. Otherwise returns
    /// `Err(SatError::NoCoreAvailable)`.
    fn unsat_core(&self) -> Result<Vec<SatLit>, SatError>;

    /// Whether `lit` occurs in the unsat core of the most recent solve.
    ///
    /// A convenience predicate over [`unsat_core`] used by deletion-based
    /// shrinking to test membership without materializing the whole core
    /// each iteration. Precondition is the same as [`unsat_core`].
    ///
    /// [`unsat_core`]: SatBackend::unsat_core
    fn core_contains(&self, lit: SatLit) -> Result<bool, SatError>;

    /// Human-readable backend identifier (e.g. `"batsat-v0.6"`). Stable
    /// across the backend's lifetime for a given solver version; pinned
    /// as a literal so a silent SAT-library bump cannot change it.
    fn backend_id(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sat_lit_polarity_round_trips() {
        let v = SatVar(7);
        let pos = SatLit::positive(v);
        let neg = SatLit::negative(v);
        assert_eq!(pos.var(), v);
        assert_eq!(neg.var(), v);
        assert!(pos.polarity());
        assert!(!neg.polarity());
        // The positive and negative literals of the same variable are
        // distinct values — the deletion loop relies on this to tell an
        // assumption apart from its complement.
        assert_ne!(pos, neg);
    }

    #[test]
    fn sat_var_index_is_stable() {
        assert_eq!(SatVar(0).index(), 0);
        assert_eq!(SatVar(41).index(), 41);
    }

    #[test]
    fn sat_error_display_is_stable() {
        // These strings front the solver-facing error vocabulary; pinning
        // them here catches silent drift, mirroring the BackendError
        // display test on the BDD side.
        assert_eq!(
            format!("{}", SatError::TopLevelUnsat),
            "clause set became unsatisfiable at the top level"
        );
        assert_eq!(
            format!("{}", SatError::NoCoreAvailable),
            "no unsat core: last solve did not return unsat"
        );
        assert_eq!(
            format!("{}", SatError::Invariant("oops")),
            "invariant violated: oops"
        );
    }
}
