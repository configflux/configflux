// SPDX-License-Identifier: BUSL-1.1
//
// batsat-backed implementation of `SatBackend`.
//
// Per ADR-0004 §4, this is the ONLY file in the workspace allowed to
// `use batsat::*`. Every other file in `solver/` — including
// `solver::Session` and `Session::explain_rejection` — calls through the
// `SatBackend` trait defined in `sat_backend.rs`. The confinement is the
// AC-5 mitigation leg from `docs/security/dep-audit-v0.4.0.md` §2: a
// future swap to `cadical`, `kissat` (via FFI), or another CDCL library
// is mechanical because no batsat type crosses the trait boundary.
// Reviewers should reject any diff that surfaces a `batsat::Lit`,
// `batsat::Var`, `batsat::lbool`, `batsat::Solver`, or any other batsat
// type across the `pub` surface, or that imports `batsat` in any other
// `solver/` file.
//
// Scope (configflux-tcjw): the deletion-based MUS-extraction surface
// only — variable allocation, clause addition, assumption-scoped
// solving, and unsat-core (conflict/reason) extraction. No caller is
// wired yet; `Session::explain_rejection` is a separate issue
// (configflux-kv5d). batsat is MiniSat-derived (Eén & Sörensson 2003);
// its `add_clause_reuse` / `solve_limited` / `unsat_core` map directly to
// the canonical deletion-based MUS loop (Marques-Silva & Lynce 2011). The
// SAT side is heuristic-dependent by design (ADR-0004 §4): its output is
// a subset of the input, not a canonical representation, and nothing here
// touches `ccm.bdd.bin` byte stability — that is the BDD side's job.

use batsat::interface::SolverInterface;
use batsat::{lbool, BasicSolver, Lit, Var};

use crate::sat_backend::{SatBackend, SatError, SatLit, SatOutcome, SatVar};

/// Stable identifier for the batsat backend, surfaced through
/// `SatBackend::backend_id`. Pinned as a literal so a silent batsat
/// source bump cannot change the identifier; changing this string is a
/// deliberate review event linked to the pinned `batsat = "= 0.6.0"`
/// dependency in `solver/Cargo.toml`.
const BATSAT_BACKEND_ID: &str = "batsat-v0.6";

/// Concrete `SatBackend` wrapping the batsat CDCL SAT solver.
///
/// State:
///   - `solver`: the owned batsat `BasicSolver` (a `Solver` with the
///     default basic callbacks and no theory). Every batsat call routes
///     through it.
///   - `vars`: a handle table mapping `SatVar.0` to the batsat `Var` it
///     was issued for. `new_var` pushes one entry per allocation, so
///     `vars[i]` is the batsat variable for `SatVar(i)`. Keeping the map
///     explicit (rather than reconstructing batsat `Var`s from raw
///     indices) means a forged or out-of-range `SatVar` is caught here as
///     a typed `Invariant` error instead of fabricating a batsat literal.
///   - `last_outcome`: the result of the most recent
///     `solve_with_assumptions`, used to enforce the `unsat_core`
///     precondition (the core is only meaningful right after an `Unsat`
///     result). `None` before any solve.
///
/// `Debug` is hand-implemented because batsat's `Solver` does not derive
/// a useful `Debug` and the crate-wide `deny(missing_debug_implementations)`
/// requires one. The impl emits only summary counts — no clause-database
/// dump.
pub struct BatsatBackend {
    solver: BasicSolver,
    vars: Vec<Var>,
    last_outcome: Option<SatOutcome>,
}

impl core::fmt::Debug for BatsatBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BatsatBackend")
            .field("backend_id", &BATSAT_BACKEND_ID)
            .field("var_count", &self.vars.len())
            .field("last_outcome", &self.last_outcome)
            .finish_non_exhaustive()
    }
}

impl BatsatBackend {
    /// Resolve a solver-side `SatVar` back to the batsat `Var` it was
    /// issued for. Returns `None` for a forged/out-of-range handle;
    /// handles issued by `new_var` are always in range.
    #[inline]
    fn lookup(&self, var: SatVar) -> Option<Var> {
        self.vars.get(var.0 as usize).copied()
    }

    /// Translate a solver-side `SatLit` into a batsat `Lit`.
    ///
    /// `SatLit::polarity() == true` is the positive literal; batsat's
    /// `Lit::new(var, sign)` uses `sign == true` for the positive
    /// literal, so the polarity maps straight through. An unknown
    /// variable is a typed `Invariant` error rather than a panic.
    #[inline]
    fn to_batsat_lit(&self, lit: SatLit) -> Result<Lit, SatError> {
        let var = self.lookup(lit.var()).ok_or(SatError::Invariant(
            "sat literal references a variable this backend never issued",
        ))?;
        Ok(Lit::new(var, lit.polarity()))
    }

    /// Translate a batsat unsat-core literal back into the corresponding
    /// *assumption* literal in solver-side terms.
    ///
    /// batsat (MiniSat convention) expresses the final conflict as a clause
    /// implied false under the assumptions, so it stores the **negations**
    /// of the assumption literals forming the core (`core.rs::analyze_final`
    /// inserts `!p` / `!lit`). The `SatBackend` contract returns the subset
    /// of the supplied *assumptions* in the caller's polarity, so we negate
    /// to recover the assumption. The variable index batsat reports is the
    /// same allocation-order index `new_var` recorded.
    #[inline]
    fn assumption_from_core_lit(lit: Lit) -> SatLit {
        let var = SatVar(lit.var().idx());
        // Negate: a core entry `¬x` corresponds to the assumption `x`.
        if lit.sign() {
            SatLit::negative(var)
        } else {
            SatLit::positive(var)
        }
    }
}

impl SatBackend for BatsatBackend {
    fn new_solver() -> Self {
        // `BasicSolver::default()` constructs a `Solver` with default
        // options and the basic (no-op) callbacks. The clause database is
        // empty and no variables exist until `new_var` is called.
        Self {
            solver: BasicSolver::default(),
            vars: Vec::new(),
            last_outcome: None,
        }
    }

    fn new_var(&mut self) -> SatVar {
        // `new_var_default` allocates a decision variable with the
        // default polarity. batsat hands back variables in allocation
        // order starting at index 0, which is exactly the `SatVar`
        // contract; we still record the mapping explicitly so handle
        // resolution never reconstructs a batsat `Var` from a raw int.
        let var = self.solver.new_var_default();
        let idx = self.vars.len() as u32;
        self.vars.push(var);
        SatVar(idx)
    }

    fn add_clause(&mut self, literals: &[SatLit]) -> Result<(), SatError> {
        // Adding a clause invalidates any previously-extracted core; the
        // clause database has changed, so a stale `last_outcome` must not
        // be trusted to gate `unsat_core`.
        self.last_outcome = None;

        // batsat's `add_clause_reuse` takes `&mut Vec<Lit>` (it may
        // reorder/dedup in place), so we build an owned buffer.
        let mut clause: Vec<Lit> = Vec::with_capacity(literals.len());
        for &lit in literals {
            clause.push(self.to_batsat_lit(lit)?);
        }

        // `add_clause_reuse` returns `false` when the addition leaves the
        // solver in a top-level-unsat state (e.g. adding `¬x` after `x`,
        // or an explicit empty clause). That is a legitimate state for an
        // MUS query — the contradiction is the whole point — so we report
        // it as a typed, non-panicking error and let the caller decide.
        let still_ok = self.solver.add_clause_reuse(&mut clause);
        if still_ok {
            Ok(())
        } else {
            Err(SatError::TopLevelUnsat)
        }
    }

    fn solve_with_assumptions(
        &mut self,
        assumptions: &[SatLit],
    ) -> Result<SatOutcome, SatError> {
        let mut assumps: Vec<Lit> = Vec::with_capacity(assumptions.len());
        for &lit in assumptions {
            assumps.push(self.to_batsat_lit(lit)?);
        }

        // `solve_limited` runs an incremental solve under the assumption
        // literals and returns an `lbool`. The assumption set is exactly
        // the set of literals that may appear in the unsat core, which is
        // what deletion-based MUS extraction shrinks.
        let result = self.solver.solve_limited(&assumps);
        let outcome = if result == lbool::TRUE {
            SatOutcome::Sat
        } else if result == lbool::FALSE {
            SatOutcome::Unsat
        } else {
            // `lbool::UNDEF` — the solver hit a resource limit before
            // deciding. Surfaced as `Unknown`; the caller maps it to a
            // hard error because a sound MUS needs a definite UNSAT.
            SatOutcome::Unknown
        };
        self.last_outcome = Some(outcome);
        Ok(outcome)
    }

    fn unsat_core(&self) -> Result<Vec<SatLit>, SatError> {
        // The core is only meaningful immediately after an `Unsat`
        // result. Any other state (no solve yet, or last solve was Sat /
        // Unknown, or the database mutated since) is a precondition
        // violation reported as a typed error rather than returning a
        // stale or empty slice.
        if self.last_outcome != Some(SatOutcome::Unsat) {
            return Err(SatError::NoCoreAvailable);
        }
        let core = self
            .solver
            .unsat_core()
            .iter()
            .map(|&lit| Self::assumption_from_core_lit(lit))
            .collect();
        Ok(core)
    }

    fn core_contains(&self, lit: SatLit) -> Result<bool, SatError> {
        if self.last_outcome != Some(SatOutcome::Unsat) {
            return Err(SatError::NoCoreAvailable);
        }
        // A literal whose variable this backend never issued cannot be in
        // any core; resolving it through `to_batsat_lit` also guards
        // against a forged handle reaching batsat's membership query.
        //
        // batsat stores the core as the negated assumptions (see
        // `assumption_from_core_lit`), so to ask "is the assumption `lit`
        // in the core?" we query batsat for its negation. We invert the
        // resolved batsat literal rather than the `SatLit` so the variable
        // bounds check in `to_batsat_lit` still runs first.
        let batsat_lit = self.to_batsat_lit(lit)?;
        Ok(self.solver.unsat_core_contains_lit(!batsat_lit))
    }

    fn backend_id(&self) -> &'static str {
        BATSAT_BACKEND_ID
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the batsat-backed `SatBackend` implementation.
    //!
    //! These live inside the backend module so they can exercise the
    //! concrete `BatsatBackend` directly. The crate-boundary invariant
    //! (no batsat type leaks across the public surface) is upheld by
    //! construction: `SatBackend`'s signatures mention only `SatVar`,
    //! `SatLit`, `SatOutcome`, and `SatError`.
    use super::*;

    #[test]
    fn two_contradictory_unit_clauses_are_unsat_under_assumptions() {
        // The configflux-tcjw acceptance test: construct a BatsatBackend,
        // add two contradictory unit clauses (`x` and `¬x`), and confirm
        // the solver reports UNSAT under assumptions.
        //
        // Adding `x` then `¬x` makes the formula unconditionally
        // unsatisfiable. batsat reports the second unit clause as driving
        // the database top-level-unsat (`add_clause_reuse` returns false),
        // which our wrapper surfaces as `TopLevelUnsat`. A subsequent
        // solve under any assumption set therefore returns `Unsat`.
        let mut backend = BatsatBackend::new_solver();
        let x = backend.new_var();

        backend
            .add_clause(&[SatLit::positive(x)])
            .expect("adding the first unit clause `x` must succeed");

        // The second contradictory unit clause `¬x` drives the database
        // unsat at the top level — expected for this contradiction.
        let second = backend.add_clause(&[SatLit::negative(x)]);
        assert_eq!(
            second,
            Err(SatError::TopLevelUnsat),
            "adding `¬x` after `x` must report top-level unsat"
        );

        // Under assumptions (here, asserting `x` as the candidate), the
        // solve must return UNSAT — the clause set is contradictory.
        let outcome = backend
            .solve_with_assumptions(&[SatLit::positive(x)])
            .expect("solve must not error");
        assert_eq!(
            outcome,
            SatOutcome::Unsat,
            "two contradictory unit clauses must be UNSAT under assumptions"
        );
    }

    #[test]
    fn unsat_core_under_assumptions_is_the_conflicting_subset() {
        // The MUS-extraction shape `Session::explain_rejection` will use:
        // a mutual-exclusion constraint `(¬a ∨ ¬b)` (a and b cannot both
        // hold) with both selections asserted as assumptions `[a, b]`.
        // The solve is UNSAT and the unsat core is the subset of
        // assumptions sufficient to prove it — here exactly `{a, b}`.
        //
        // Why this shape and not "clause `x` + assume `¬x`": batsat
        // (MiniSat-derived) proves a contradiction between an assumption
        // and a *unit fact already propagated at decision level 0*
        // without placing the assumption on the trail, so `analyze_final`
        // returns at level 0 with an empty assumption core (batsat
        // `core.rs::analyze_final`, the `decision_level() == 0` early
        // return). A non-empty core therefore requires the conflict to
        // arise from the assumptions themselves at a decision level > 0,
        // which is exactly what the mutual-exclusion encoding produces.
        // This is the encoding contract `explain_rejection` (configflux-kv5d)
        // must honour: model the rejection so the conflicting selections
        // are assumptions, not level-0 unit facts.
        let mut backend = BatsatBackend::new_solver();
        let a = backend.new_var();
        let b = backend.new_var();

        backend
            .add_clause(&[SatLit::negative(a), SatLit::negative(b)])
            .expect("adding the mutual-exclusion clause `¬a ∨ ¬b` must succeed");

        let outcome = backend
            .solve_with_assumptions(&[SatLit::positive(a), SatLit::positive(b)])
            .expect("solve must not error");
        assert_eq!(
            outcome,
            SatOutcome::Unsat,
            "asserting both mutually-exclusive selections must be UNSAT"
        );

        let core = backend.unsat_core().expect("core available after unsat");
        // The core is a subset of the assumptions sufficient for the
        // contradiction. Both `a` and `b` are required (dropping either
        // makes the clause satisfiable), so both must appear.
        assert!(
            core.contains(&SatLit::positive(a)),
            "the unsat core must contain assumption `a`"
        );
        assert!(
            core.contains(&SatLit::positive(b)),
            "the unsat core must contain assumption `b`"
        );
        // `core_contains` must agree with the materialized core, and must
        // report a literal that is NOT in the core as absent.
        assert!(
            backend
                .core_contains(SatLit::positive(a))
                .expect("core_contains available after unsat"),
            "core_contains must agree with unsat_core membership for `a`"
        );
        assert!(
            !backend
                .core_contains(SatLit::negative(a))
                .expect("core_contains available after unsat"),
            "the complementary literal `¬a` must not be reported in the core"
        );
    }

    #[test]
    fn satisfiable_clause_set_solves_sat_and_guards_the_core_precondition() {
        // Before any solve, requesting a core is a precondition error.
        let mut backend = BatsatBackend::new_solver();
        assert_eq!(
            backend.unsat_core(),
            Err(SatError::NoCoreAvailable),
            "a core request before any solve must be a precondition error"
        );

        // `new_var` issues sequential allocation-order indices.
        let x = backend.new_var();
        let y = backend.new_var();
        assert_eq!(x, SatVar(0));
        assert_eq!(y, SatVar(1));

        // A single clause `x ∨ y` is satisfiable; with no assumptions the
        // solve must report SAT, and the core precondition must then
        // reject a request again (last result was not Unsat).
        backend
            .add_clause(&[SatLit::positive(x), SatLit::positive(y)])
            .expect("adding `x ∨ y` must succeed");

        let outcome = backend
            .solve_with_assumptions(&[])
            .expect("solve must not error");
        assert_eq!(outcome, SatOutcome::Sat);

        assert_eq!(
            backend.unsat_core(),
            Err(SatError::NoCoreAvailable),
            "requesting a core after a SAT result must be a precondition error"
        );
    }

    #[test]
    fn forged_variable_handle_is_a_typed_error() {
        // A `SatVar` this backend never issued must not fabricate a
        // batsat literal; it is caught as a typed invariant error.
        let mut backend = BatsatBackend::new_solver();
        let _real = backend.new_var();
        let forged = SatVar(9_999);
        assert_eq!(
            backend.add_clause(&[SatLit::positive(forged)]),
            Err(SatError::Invariant(
                "sat literal references a variable this backend never issued"
            ))
        );
    }

    #[test]
    fn backend_id_is_stable() {
        let backend = BatsatBackend::new_solver();
        assert_eq!(backend.backend_id(), BATSAT_BACKEND_ID);
        assert_eq!(backend.backend_id(), "batsat-v0.6");
    }
}
