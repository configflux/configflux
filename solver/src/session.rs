// SPDX-License-Identifier: BUSL-1.1
//
// `solver::Session` — transport-ready configuration session per ADR-0003 §4.
// Generic over `B: SolverBackend` so tests can swap NullBackend ↔ real
// backends without object-safety churn.
//
// State as of configflux-0r62: the session is the multi-partition
// boolean composition per ADR-0012 §1. It holds `Vec<PartitionSession<B>>`
// — one entry per cluster partition plus an optional bridge — and the
// public API methods (`valid_options`, `apply`, `retract`, `state_hash`)
// fan out across partitions. The public surface (return shapes, error
// variants, generic parameter, accessors) is unchanged from the v1
// single-current path per ADR-0003 §1: the multi-partition fan-out is
// implementation detail behind a stable API.

use core::fmt;

use crate::backend::{BackendError, FormulaHandle, SolverBackend};
use crate::ccm::{Ccm, CcmError};
use crate::partition_session::{PartitionSession, UndoEntry, BRIDGE_PARTITION_INDEX};

/// Content-addressed hash of the full current session state per ADR-0003 §4.
/// Stable byte-for-byte across runs given the same applies/retracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateHash(pub [u8; 32]);

impl StateHash {
    /// All-zero state hash — returned for an empty-Ccm session.
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }
}

/// Options still valid under the current selection for a facet.
/// ADR-0003 §4 pins this as a structured `Serialize` type. configflux-8dm.3
/// added `options: Vec<String>` (the canonical sub-facet option suffixes
/// under ADR-0005 §3 `tag.value` naming) and kept `count` for the
/// round-trip-empty and smoke callers that assert `count == 0`.
/// `count == options.len()` is an invariant established at construction.
/// Order is symbol-table order (BDD variable-index order); the
/// multi-partition path (configflux-0r62) preserves this by walking
/// the first partition that contains the facet in ascending order and
/// intersecting subsequent partitions' results without re-sorting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidOptions {
    /// Number of still-valid options for the facet. Invariant: equals
    /// `options.len()`.
    pub count: usize,
    /// Option values for which the current formula remains satisfiable.
    pub options: Vec<String>,
}

/// Structured explanation for a `(facet, option)` rejection per ADR-0003 §4.
///
/// configflux-kv5d (M4) lit up the `core` field: when `would_reject` is
/// `true` for a genuine constraint conflict, `core` carries the labeled
/// minimal unsatisfiable subset (MUS) extracted per ADR-0004 §4. When the
/// option is genuinely valid, `would_reject` is `false` and `core` is
/// `None` (ADR-0030 D5 division-of-labor: a valid option has no conflict to
/// explain). The labeled core is a **solver-owned** type carrying labeled
/// `{facet}.{value}` strings only — never a raw BDD or batsat variable
/// index (ADR-0003 §2 forbids a compiler type crossing into `solver/`; the
/// interpreter/runtime wrappers convert this into the compiler-side
/// `UnsatCore` envelope).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RejectionExplanation {
    /// Whether the pair would actually be rejected.
    pub would_reject: bool,
    /// The labeled minimal unsatisfiable subset, present iff
    /// `would_reject` is `true` and the rejection is a genuine constraint
    /// conflict (ADR-0004 §4). `None` for a genuinely-valid option.
    pub core: Option<LabeledCore>,
}

/// A single labeled `{facet}.{value}` atom inside a [`LabeledCore`]. The
/// solver's own analogue of the compiler-side `ConstraintFacet`
/// (ADR-0031 D3) — labeled strings only, defined under `solver/` so no
/// compiler type crosses the ADR-0003 §2 boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LabeledAtom {
    /// Facet name (the prefix before the last `.` of a `{facet}.{value}`
    /// symbol per ADR-0005 §3).
    pub facet: String,
    /// Option value (the suffix after the last `.`).
    pub value: String,
}

/// Whether a labeled constraint in a [`LabeledCore`] is a prior
/// *selection* the session already committed or a *model rule* baked into
/// the compiled `.ccm`. The solver's own analogue of the compiler-side
/// `ConstraintKind` (ADR-0031 D3) — the compiler never imports this; the
/// interpreter/runtime wrappers map it onto `ConstraintKind::Selection` /
/// `ConstraintKind::ModelRule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreConstraintKind {
    /// A `{facet}.{value}` the session has already pinned via `apply`.
    Selection,
    /// A `requires`/`excludes`-style rule from the compiled model.
    ModelRule,
}

/// One labeled literal of the partial assignment a MUS clause forbids.
///
/// A model clause is the negation of a forbidden partial assignment, so the
/// clause `(¬environment.prod ∨ ¬log_level.debug)` forbids
/// `environment.prod = true ∧ log_level = debug = true`. [`Self::asserted`]
/// records that polarity, which [`LabeledConstraint::atoms`] deliberately
/// drops (it is the flat, sorted atom set the ADR-0031 D3 `facets` field
/// carries).
///
/// The polarity is what makes constraint attribution possible downstream
/// (ADR-0054 §5.4): a consumer reconstructs the forbidden facet assignment
/// and asks which *declared* constraint that assignment violates. Without
/// the sign, `environment.prod` alone cannot say whether `prod` was chosen
/// or ruled out.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LabeledLiteral {
    /// The labeled `{facet}.{value}` this literal is about.
    pub atom: LabeledAtom,
    /// `true` when the forbidden assignment sets the symbol true (the option
    /// was taken), `false` when it sets it false (the option was ruled out).
    pub asserted: bool,
}

/// One entry in the labeled minimal unsatisfiable subset: a single
/// constraint (a disjunction of labeled atoms) that, together with the
/// rejected candidate, contributes to the unsatisfiability. The solver's
/// own analogue of the compiler-side `ConflictingConstraint` (ADR-0031 D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabeledConstraint {
    /// Whether this constraint is a prior selection or a model rule.
    pub kind: CoreConstraintKind,
    /// The labeled atoms naming the facets/options the constraint relates.
    /// For a model clause this is the set of `{facet}.{value}` literals on
    /// the BDD falsifying path that produced the clause; for a selection
    /// it is the single pinned atom.
    pub atoms: Vec<LabeledAtom>,
    /// The signed partial assignment this clause forbids, sorted by atom then
    /// polarity so the shape is stable for a given MUS witness. Same literal
    /// set as [`Self::atoms`], but carrying the sign each atom had on the BDD
    /// falsifying path — see [`LabeledLiteral`].
    pub forbidden: Vec<LabeledLiteral>,
}

/// The labeled minimal unsatisfiable subset (MUS) attached to a genuine
/// `(facet, option)` rejection per ADR-0004 §4. **Solver-owned**: carries
/// labeled `{facet}.{value}` strings only and is defined under `solver/`
/// so no compiler type crosses the ADR-0003 §2 boundary. The
/// interpreter/runtime wrappers (configflux-whyt / configflux-3b5y)
/// convert this into the compiler-side `UnsatCore` envelope (ADR-0031 D3).
///
/// Every atom in [`Self::rejected`] and in every
/// [`LabeledConstraint::atoms`] is a labeled name resolved through
/// `ccm.symbols.json`; a MUS variable that does not resolve to a
/// symbol-table name is a fail-closed `Err` (ADR-0031 D4), never emitted
/// here. No raw BDD or batsat variable index ever appears in this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabeledCore {
    /// The candidate `{facet}.{value}` being explained (the rejected
    /// selection).
    pub rejected: LabeledAtom,
    /// The minimal set of conflicting constraints. Sorted deterministically
    /// (by kind then by atoms) so the labeled output is stable in shape for
    /// a given MUS witness.
    pub conflicting_constraints: Vec<LabeledConstraint>,
    /// Whether the subset is minimal — `true` after deletion-based
    /// shrinking (ADR-0004 §4 step (d)).
    pub minimal: bool,
}

/// Stable error enum for `Session` methods. ADR-0003 flags the variant
/// list as an implementation concern; configflux-0r62 keeps the
/// existing variants and adds `Invariant` for the cross-partition
/// apply guard (a symbol that lives in two cluster partitions is a
/// partitioner bug per ADR-0012 §1 "apply semantics").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Underlying `.ccm` load failed.
    Ccm(CcmError),
    /// Backend (oxidd, NullBackend, future BuDDy) failed while
    /// reconstructing the BDD or evaluating a query.
    Backend(BackendError),
    /// The facet name is not a prefix of any symbol in the loaded Ccm.
    UnknownFacet(String),
    /// `apply(facet, value)` was called with a `(facet, value)` pair
    /// whose compound symbol (`{facet}.{value}` per ADR-0005 §3) is
    /// not present in the loaded CCM's symbol table. Distinct from
    /// `UnknownFacet` (which signals "no option under this facet at
    /// all") — this variant fires when the facet exists but the
    /// specific value does not.
    UnknownOption {
        /// Facet name passed to `apply`.
        facet: String,
        /// Value name passed to `apply`.
        value: String,
    },
    /// `apply(facet, value)` conjoined the current formula with the
    /// unit clause for `{facet}.{value}` and the result reduced to
    /// the FALSE terminal — the selection is inconsistent with the
    /// active formula. Session state is left unchanged per ADR-0004
    /// §"Session state". For multi-partition apply (cluster + bridge
    /// per ADR-0012 §1), at least one partition's conjoined formula
    /// reduced to ⊥; the session is rolled back to the pre-apply
    /// state for ALL touched partitions before this error surfaces.
    Conflict {
        /// Facet name whose selection caused the conflict.
        facet: String,
        /// Value name whose selection caused the conflict.
        value: String,
    },
    /// An internal invariant on multi-partition apply was violated —
    /// specifically, the applied `{facet}.{value}` symbol was found
    /// in two cluster partitions (no bridge participant). Per
    /// ADR-0012 §1 "apply semantics" the partitioner emits disjoint
    /// cluster variable sets by construction; a duplicate is a
    /// partitioner bug, not a legitimate runtime case. Carries a
    /// short identifier for debug only.
    Invariant(&'static str),
}

impl From<CcmError> for Error {
    fn from(err: CcmError) -> Self {
        Error::Ccm(err)
    }
}

impl From<BackendError> for Error {
    fn from(err: BackendError) -> Self {
        Error::Backend(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Ccm(inner) => write!(f, "ccm load failed: {inner}"),
            Error::Backend(inner) => write!(f, "backend failed: {inner}"),
            Error::UnknownFacet(name) => write!(f, "unknown facet: {name}"),
            Error::UnknownOption { facet, value } => {
                write!(f, "unknown option: {facet}.{value}")
            }
            Error::Conflict { facet, value } => {
                write!(f, "apply conflict: {facet}.{value} is inconsistent")
            }
            Error::Invariant(msg) => {
                write!(f, "session invariant violated: {msg}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Transport-ready configuration session. Generic over `B: SolverBackend`;
/// the test suite uses `NullBackend`, `OxiddBackend`, and `CuddBackend`.
/// See ADR-0003 §4 for the method contract and §5 for the v0.3.0
/// resident execution model.
///
/// Internal layout (configflux-0r62 / ADR-0012 §1): the session owns
/// one `PartitionSession<B>` per partition (cluster + optional
/// bridge). Each `PartitionSession` carries its own backend instance,
/// its own current handle, and its own variable-order — so the CUDD /
/// oxidd manager isolation rule is structurally enforced (a handle
/// from partition K can only flow to partition K's backend). The
/// shared undo stack records the partition-of-origin per entry so
/// `retract` restores only the partition that was modified.
#[derive(Debug)]
pub struct Session<B: SolverBackend> {
    /// Bound CCM handle. `pub(crate)` for the `resolve.rs` walk (i0ne).
    pub(crate) ccm: Ccm,
    /// One entry per partition. For an empty `Ccm`, contains exactly
    /// one trivial partition holding the constant ⊤ — the M0
    /// round-trip-empty smoke test path. For a v2 multi-part `Ccm`
    /// (`Ccm::multi_part().is_some()`), contains N cluster entries
    /// followed by an optional bridge entry in partition-index
    /// ascending order. The bridge, when present, uses the sentinel
    /// `BRIDGE_PARTITION_INDEX` (`u32::MAX`) per ADR-0012 §8.
    /// `pub(crate)` for the `resolve.rs` walk (configflux-i0ne).
    pub(crate) parts: Vec<PartitionSession<B>>,
    /// Shared undo stack: one entry per partition that was modified
    /// in the most recent atomic apply. Multi-partition applies push
    /// multiple entries in partition-index ascending order; each
    /// `retract` pops a single entry and restores only the partition
    /// it names. ADR-0012 §7.
    undo: Vec<UndoEntry>,
    /// Committed `{facet}.{value}` selections, in apply order. One entry
    /// per successful `apply` (NOT per touched partition), carrying the
    /// number of undo entries that apply pushed so `retract` can pop the
    /// selection only after its last partition has been restored. Consumed
    /// by `explain_rejection` (configflux-kv5d) to classify a MUS clause as
    /// a prior `Selection` versus a `ModelRule`, and to reconstruct the
    /// committed feasible formula as `original ∧ pinned-selections`. Private
    /// session state — it does not affect `state_hash` (whose pre-image is
    /// the undo-contribution counts per ADR-0012 §8) nor any wire format.
    committed: Vec<CommittedSelection>,
}

/// One committed selection tracked for `explain_rejection`. `compound` is
/// the `{facet}.{value}` symbol; `undo_entries` is how many undo-stack
/// entries the originating `apply` pushed (1 for a single-partition apply,
/// N for an atomic multi-partition apply), so `retract` decrements it and
/// drops the selection only when it reaches zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedSelection {
    pub(crate) compound: String,
    pub(crate) undo_entries: usize,
}

impl<B: SolverBackend> Session<B> {
    /// Load a `.ccm` artifact from disk. Delegates to `Ccm::load_from_cmp`
    /// which runs the v2 multi-part load procedure for real CCM
    /// directories (ADR-0012 §4–§5) and falls back to the empty-Ccm
    /// stub for the non-existent sentinel path used by `round_trip_empty`.
    pub fn load_ccm(path: &std::path::Path) -> Result<Ccm, Error> {
        Ok(Ccm::load_from_cmp(path)?)
    }

    /// Construct a new Session bound to a loaded CCM handle. Threads
    /// every partition's BDD through a fresh `SolverBackend` instance,
    /// so each partition's `current` after `new` is the real root of
    /// the per-partition compiled constraint model (or `⊤` for the
    /// empty-Ccm path).
    pub fn new(ccm: Ccm) -> Result<Self, Error> {
        let parts = match ccm.multi_part() {
            Some(payload) => {
                let mut out: Vec<PartitionSession<B>> = Vec::with_capacity(
                    payload.clusters.len() + payload.bridge.iter().count(),
                );
                for (idx, cluster) in payload.clusters.iter().enumerate() {
                    out.push(PartitionSession::<B>::from_partition(idx as u32, cluster)?);
                }
                if let Some(bridge) = payload.bridge.as_ref() {
                    out.push(PartitionSession::<B>::from_partition(
                        BRIDGE_PARTITION_INDEX,
                        bridge,
                    )?);
                }
                out
            }
            None => {
                // Empty-Ccm path: one trivial partition holding `⊤` so
                // every public API call still has a `PartitionSession`
                // to fan out to. Preserves the M0 round-trip-empty
                // contract that pins every method as Ok-returning on
                // an empty session.
                vec![PartitionSession::<B>::empty_constant_true()?]
            }
        };

        Ok(Self {
            ccm,
            parts,
            undo: Vec::new(),
            committed: Vec::new(),
        })
    }

    /// Options still valid under the current session state for `facet`.
    /// Pure query per ADR-0003 §4. Per ADR-0012 §1 the multi-partition
    /// path returns the intersection of per-partition surviving sets:
    /// every partition whose symbol table contains a `{facet}.*` symbol
    /// contributes its per-partition `is_var_sat_under` enumeration,
    /// and the session returns the set-intersection of those
    /// contributions in symbol-table order (the first contributing
    /// partition's order is preserved; later contributors only narrow
    /// the membership). A facet that appears in no partition surfaces
    /// `Error::UnknownFacet` per the single-partition contract;
    /// empty-Ccm short-circuits to `ValidOptions::default()`.
    pub fn valid_options(&self, facet: &str) -> Result<ValidOptions, Error> {
        // Empty-Ccm shortcut: no symbols loaded → preserve the round-
        // trip-empty contract (`count == 0`, empty options).
        if self.ccm.symbols().is_none() && self.ccm.multi_part().is_none() {
            return Ok(ValidOptions::default());
        }
        let prefix = format!("{facet}.");
        let mut acc: Option<Vec<String>> = None;
        let mut saw_any = false;
        for part in &self.parts {
            let Some(contrib) = part.valid_options_for(&prefix)? else {
                continue;
            };
            saw_any = true;
            acc = Some(match acc {
                None => contrib,
                Some(prior) => intersect_preserving_order(&prior, &contrib),
            });
        }
        if !saw_any {
            return Err(Error::UnknownFacet(facet.to_string()));
        }
        let options = acc.unwrap_or_default();
        Ok(ValidOptions {
            count: options.len(),
            options,
        })
    }

    /// Apply `value` to `facet` by conjoining the active formula with
    /// the unit clause for the `{facet}.{value}` symbol.
    ///
    /// Multi-partition semantics (ADR-0012 §1):
    ///   1. Resolve the compound symbol `{facet}.{value}` against
    ///      every partition's symbol table. Collect the partitions
    ///      that own it.
    ///   2. If no partition owns it, return `Error::UnknownOption`.
    ///   3. If two cluster partitions own it (no bridge in the set),
    ///      return `Error::Invariant` — this is a partitioner bug.
    ///   4. Stage `apply_and` on every touched partition (computes
    ///      `current ∧ x_i` on each without mutating `current`). If
    ///      any staged conjunction reduced to ⊥, OR any backend
    ///      returned an error, the session rolls back: no
    ///      `commit_apply` runs, no undo entries are pushed, and the
    ///      method returns `Error::Conflict` (for the ⊥ case) or the
    ///      backend error (otherwise).
    ///   5. Commit every staged apply (in partition-index ascending
    ///      order) and push one undo entry per touched partition (in
    ///      the same order). `retract` pops them one at a time per
    ///      ADR-0012 §7.
    ///
    /// Empty-Ccm shortcut: an empty session is a single trivial
    /// partition whose only symbol set is empty; every apply call
    /// resolves to "no partition owns this symbol" → return
    /// `Error::UnknownOption`. The M0 round-trip-empty smoke test
    /// pins this as a successful no-op via a separate code path that
    /// asserts only `apply().is_ok()` — for that path the bound `Ccm`
    /// has no symbol table at all and we collapse to the legacy
    /// "treat any apply as a no-op success" shortcut to keep the
    /// existing scaffolding tests green.
    pub fn apply(&mut self, facet: &str, value: &str) -> Result<(), Error> {
        // Legacy empty-Ccm contract (round-trip-empty smoke test):
        // the unsymbolled session treats any apply as a successful
        // no-op. Multi-partition fan-out is skipped for this path.
        if self.ccm.symbols().is_none() && self.ccm.multi_part().is_none() {
            return Ok(());
        }
        let compound = format!("{facet}.{value}");

        // Find every partition that owns the symbol, plus a flag
        // recording whether each owner is the bridge (ADR-0012 §3
        // bridge-uniform). The cluster-vs-bridge distinction is used
        // for the "two clusters → Invariant" guard below.
        let mut touched: Vec<(usize, u32, bool)> = Vec::new();
        for (idx, part) in self.parts.iter().enumerate() {
            if let Some(var_idx) = part.var_for_symbol(&compound) {
                let is_bridge = part.partition_index == BRIDGE_PARTITION_INDEX;
                touched.push((idx, var_idx, is_bridge));
            }
        }
        if touched.is_empty() {
            return Err(Error::UnknownOption {
                facet: facet.to_string(),
                value: value.to_string(),
            });
        }
        // Cluster+cluster overlap is a partitioner bug per ADR-0012 §1.
        let cluster_owners = touched.iter().filter(|(_, _, b)| !*b).count();
        if cluster_owners > 1 {
            return Err(Error::Invariant(
                "apply symbol lives in two cluster partitions; expected disjoint clusters",
            ));
        }

        // Stage the apply on every touched partition. Any backend
        // error or any ⊥ result triggers atomic rollback before
        // anything is committed.
        let mut staged: Vec<crate::partition_session::StagedApply> =
            Vec::with_capacity(touched.len());
        for (idx, var_idx, _) in &touched {
            match self.parts[*idx].stage_apply(*var_idx) {
                Ok(s) if !s.is_false => staged.push(s),
                Ok(_) => {
                    return Err(Error::Conflict {
                        facet: facet.to_string(),
                        value: value.to_string(),
                    });
                }
                Err(e) => return Err(Error::Backend(e)),
            }
        }

        // Commit every staged transition in partition-index ascending
        // order, pushing one undo entry per partition in the same
        // order. After this loop the session is in the post-apply
        // state; the undo stack carries len(touched) new entries.
        for (i, (idx, _, _)) in touched.iter().enumerate() {
            self.parts[*idx].commit_apply(&staged[i]);
            self.undo.push(UndoEntry {
                prior: staged[i].prior,
            });
        }
        // Record the committed selection once per `apply` (not per
        // partition), carrying the count of undo entries this apply pushed
        // so `retract` can drop it only after its last partition has been
        // restored. Consumed by `explain_rejection` (configflux-kv5d).
        self.committed.push(CommittedSelection {
            compound,
            undo_entries: touched.len(),
        });
        Ok(())
    }

    /// Retract the most recent `apply` by popping one entry from the
    /// shared undo stack and restoring only the named partition's
    /// `current` handle. ADR-0012 §7 makes the undo stack
    /// partition-scoped: a single `apply` that touched two partitions
    /// pushed two entries, and reverting it requires two `retract`
    /// calls.
    ///
    /// The `facet` argument is accepted for API symmetry with `apply`
    /// and for future extension (ADR-0003 §4 pins the signature) but
    /// is not consulted: the undo stack is LIFO. Callers that want
    /// facet-scoped retraction can layer it on top by tracking their
    /// own (facet → undo-depth) map.
    ///
    /// Idempotent per ADR-0003 §4: `retract` with nothing on the
    /// undo stack is a successful no-op, not an error.
    pub fn retract(&mut self, _facet: &str) -> Result<(), Error> {
        if let Some(entry) = self.undo.pop() {
            let part_idx = self
                .parts
                .iter()
                .position(|p| p.partition_index == entry.prior.partition_index)
                .ok_or(Error::Invariant(
                    "retract: undo entry references unknown partition_index",
                ))?;
            self.parts[part_idx].restore(entry.prior);
            // Decrement the most recent committed selection's undo counter,
            // dropping it once its last partition has been restored. Keeps
            // the `explain_rejection` pin set (configflux-kv5d) in lockstep
            // with the LIFO undo stack across multi-partition applies.
            if let Some(top) = self.committed.last_mut() {
                top.undo_entries = top.undo_entries.saturating_sub(1);
                if top.undo_entries == 0 {
                    self.committed.pop();
                }
            }
        }
        Ok(())
    }

    // `explain_rejection()` lives in `explain.rs` (configflux-kv5d) — a
    // split-out `impl Session<B>` block holding the ADR-0004 §4 MUS
    // extraction so neither file exceeds the repository line cap (the same
    // shape `resolve.rs` follows). It calls through the `SatBackend` trait
    // only; no `batsat::*` import lives outside `sat_backend_batsat.rs`.

    // `resolve()` lives in `resolve.rs` (configflux-i0ne) — a split-out
    // `impl Session<B>` block; see it for the ADR-0017 §3 adapter rationale.

    /// Content-addressed hash of the full current session state per
    /// ADR-0012 §8. Pre-image (v2):
    ///   `b"configflux.session-state.v2\n"
    ///    || ccm_hash || bound_model_hash
    ///    || for each partition in partition-index ascending order:
    ///         u32_be(partition_index) || u32_be(undo_contribution_count)`
    ///
    /// where `undo_contribution_count` is the number of `UndoEntry`
    /// records on the shared undo stack whose `partition_index`
    /// matches this partition. Bridge uses `u32::MAX` as its sentinel
    /// index per ADR-0012 §8. Empty-Ccm short-circuits to
    /// `StateHash::zero()` to preserve the round-trip-empty contract.
    pub fn state_hash(&self) -> StateHash {
        let ccm_hash = self.ccm.ccm_hash();
        let bound = self.ccm.bound_model_hash();
        if ccm_hash == [0u8; 32] && bound == [0u8; 32] {
            return StateHash::zero();
        }
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"configflux.session-state.v2\n");
        hasher.update(ccm_hash);
        hasher.update(bound);
        // Partition contributions in partition-index ascending order
        // (cluster 0, 1, … N-1, bridge last with u32::MAX sentinel).
        // `self.parts` is already in that order by construction in
        // `Session::new`; we still re-derive the contribution counts
        // from the shared undo stack to keep the recipe declarative.
        for part in &self.parts {
            let count = self
                .undo
                .iter()
                .filter(|e| e.prior.partition_index == part.partition_index)
                .count() as u32;
            hasher.update(part.partition_index.to_be_bytes());
            hasher.update(count.to_be_bytes());
        }
        StateHash(hasher.finalize().into())
    }

    /// Accessor for the bound `Ccm` handle.
    pub fn ccm(&self) -> &Ccm {
        &self.ccm
    }

    /// Accessor for the partition 0 backend. Exposed for unit tests
    /// that verify backend-side state after a `Session` call. The
    /// multi-partition session has N+1 backends internally; this
    /// accessor returns the first one (which is partition 0, or the
    /// single trivial empty-Ccm partition). The bd-0r62 ADR-0003
    /// hard constraint forbids leaking per-partition fan-out through
    /// the public API; downstream consumers needing multi-partition
    /// introspection use `Session::valid_options` / `state_hash` etc.
    pub fn backend(&self) -> &B {
        &self.parts[0].backend
    }

    /// Handle to partition 0's active BDD formula. Only meaningful
    /// when passed back into the partition 0 backend (which is what
    /// `backend()` returns). The bd-0r62 ADR-0003 hard constraint
    /// keeps this surface unchanged from v1 by serving partition 0's
    /// current; multi-partition consumers query through the
    /// public `Session` API instead of touching raw handles.
    pub fn current(&self) -> FormulaHandle {
        self.parts[0].current
    }

    // --- Crate-internal accessors for `explain.rs` (configflux-kv5d) ---
    //
    // These give the MUS-extraction walk in `explain.rs` read-only access
    // to the data it serializes into CNF, without widening the public
    // `Session` surface (ADR-0003 §1) and without touching the
    // `SolverBackend` trait (whose minimal surface ADR-0003 §3 protects).
    // The walk reads the *original* per-partition node tables from the
    // bound `Ccm` payload and reconstructs the committed feasible formula
    // as `original ∧ pinned-selections` — see `explain.rs` for the
    // rationale (the `current` narrowed handle is a backend-opaque BDD with
    // no clause surface, so explain rebuilds the committed formula from the
    // serialized node table plus the tracked pins).

    /// Whether the bound `Ccm` carries no multi-part payload (the empty-Ccm
    /// stub path). `explain.rs` uses this together with `ccm().symbols()`
    /// to short-circuit the unsymbolled session.
    pub(crate) fn multi_part_is_none_for_explain(&self) -> bool {
        self.ccm.multi_part().is_none()
    }

    /// Iterate every partition's symbol `variable_order` (live order from
    /// the `PartitionSession`, in `self.parts` order: clusters then bridge).
    /// Used by `explain.rs` to build the global symbol union (Q3).
    pub(crate) fn partition_variable_orders_for_explain(
        &self,
    ) -> impl Iterator<Item = &Vec<String>> {
        self.parts.iter().map(|p| &p.variable_order)
    }

    /// Iterate each partition's BDD source as
    /// `Some((variable_order, node_table, primary_root))`, or `None` for the
    /// empty-constant-true partition (empty-Ccm path). Yielded in
    /// `self.parts` order (clusters ascending, then bridge), which matches
    /// the bound `Ccm` payload's `clusters ++ bridge` order by construction
    /// in `Session::new`.
    pub(crate) fn partition_bdd_sources_for_explain(
        &self,
    ) -> Vec<Option<(&[String], &[crate::ccm_format::BddNode], u32)>> {
        let Some(payload) = self.ccm.multi_part() else {
            // Empty-Ccm: a single trivial ⊤ partition with no node table.
            return vec![None];
        };
        let mut out: Vec<Option<(&[String], &[crate::ccm_format::BddNode], u32)>> =
            Vec::with_capacity(payload.clusters.len() + payload.bridge.iter().count());
        for cluster in &payload.clusters {
            out.push(Some((
                cluster.symbols.variable_order.as_slice(),
                cluster.bdd.nodes.as_slice(),
                cluster.bdd.roots[0],
            )));
        }
        if let Some(bridge) = payload.bridge.as_ref() {
            out.push(Some((
                bridge.symbols.variable_order.as_slice(),
                bridge.bdd.nodes.as_slice(),
                bridge.bdd.roots[0],
            )));
        }
        out
    }

    /// The set of committed `{facet}.{value}` selections (deduplicated).
    /// `explain.rs` injects these as the pinned-selection unit clauses and
    /// uses them to classify a single-atom MUS clause as a prior
    /// `Selection` versus a `ModelRule`.
    pub(crate) fn pinned_symbols_for_explain(&self) -> std::collections::BTreeSet<String> {
        self.committed.iter().map(|c| c.compound.clone()).collect()
    }
}

/// Set-intersection preserving the order of `first`. Used by
/// `valid_options` to keep the returned option list in the first
/// participating partition's symbol-table order (ADR-0012 §1 implicit;
/// the per-partition contributions come back in symbol-table order, so
/// the intersection inherits it). O(N + M) for typical small option
/// sets; the inner `contains` is acceptable because option vectors
/// are tiny in practice (every facet has on the order of single-digit
/// values in real models).
fn intersect_preserving_order(first: &[String], next: &[String]) -> Vec<String> {
    first.iter().filter(|v| next.contains(v)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::NullBackend;
    use std::path::Path;

    fn fresh_session() -> Session<NullBackend> {
        let ccm = Session::<NullBackend>::load_ccm(Path::new("unused"))
            .expect("M0 stub never fails");
        Session::<NullBackend>::new(ccm).expect("M0 stub never fails")
    }

    #[test]
    fn session_new_is_infallible_against_null_backend() {
        let _s = fresh_session();
    }

    #[test]
    fn query_methods_never_panic_and_return_ok() {
        let s = fresh_session();
        let opts = s.valid_options("any-facet").expect("stub returns Ok");
        assert_eq!(opts.count, 0);
        // configflux-kv5d: `explain_rejection` is now real MUS extraction
        // (ADR-0004 §4). On the empty-Ccm session a symbol is "unknown", so
        // the call surfaces the same typed `UnknownOption` error `apply`
        // raises — not a panic and not a stub `Ok`. (A conflict-bearing run
        // is exercised by the `explain_rejection_mus` integration test.)
        let rej = s.explain_rejection("any-facet", "any-option");
        assert!(
            matches!(&rej, Err(Error::UnknownOption { .. })),
            "empty-Ccm explain_rejection must be UnknownOption, got {rej:?}",
        );
        let res = s.resolve().expect("resolve returns Ok on the empty session");
        assert!(res.satisfiable);
        // Empty-Ccm path: no facets to resolve, but the hash is still
        // computed (deterministically) over the empty canonical output.
        assert!(res.resolved_output.is_empty());
        let res2 = s.resolve().expect("resolve is a pure, repeatable query");
        assert_eq!(
            res.resolve_hash, res2.resolve_hash,
            "empty-Ccm resolve_hash must be deterministic across calls",
        );
        let hash = s.state_hash();
        assert_eq!(hash, StateHash::zero());
    }

    #[test]
    fn mutating_methods_never_panic_and_return_ok() {
        let mut s = fresh_session();
        s.apply("engine", "v6").expect("stub returns Ok");
        s.retract("engine").expect("stub returns Ok");
    }

    #[test]
    fn apply_and_retract_are_idempotent_under_the_stub() {
        // ADR-0003 Section 4 requires idempotency on repeat calls with the
        // same arguments. The M0 stub trivially satisfies this because it
        // holds no state, but the assertion is here so that M1 sees the
        // contract in the test and preserves it during the real wiring.
        let mut s = fresh_session();
        s.apply("engine", "v6").unwrap();
        s.apply("engine", "v6").unwrap();
        s.retract("engine").unwrap();
        s.retract("engine").unwrap();
    }

    #[test]
    fn error_display_is_stable() {
        let err: Error = CcmError::BoundModelHashMismatch.into();
        let rendered = format!("{err}");
        assert!(rendered.starts_with("ccm load failed:"));
        assert!(rendered.contains("bound_model_hash"));
        let unknown = Error::UnknownFacet("transmission".into());
        assert_eq!(format!("{unknown}"), "unknown facet: transmission");
        let unknown_option = Error::UnknownOption {
            facet: "engine".into(),
            value: "v12".into(),
        };
        assert_eq!(
            format!("{unknown_option}"),
            "unknown option: engine.v12"
        );
        let conflict = Error::Conflict {
            facet: "engine".into(),
            value: "v8".into(),
        };
        assert_eq!(
            format!("{conflict}"),
            "apply conflict: engine.v8 is inconsistent"
        );
        // configflux-0r62 adds Invariant for the cluster+cluster
        // overlap guard. Pin the wire string for daemon envelopes.
        let invariant = Error::Invariant("apply symbol lives in two cluster partitions");
        assert_eq!(
            format!("{invariant}"),
            "session invariant violated: apply symbol lives in two cluster partitions"
        );
    }

    #[test]
    fn backend_error_displays_through_session_error() {
        let err: Error = BackendError::Serialization("bad node").into();
        let rendered = format!("{err}");
        assert!(rendered.starts_with("backend failed:"));
        assert!(rendered.contains("bad node"));
    }

    #[test]
    fn state_hash_is_zero_for_empty_ccm_and_session() {
        // Empty-Ccm path: state_hash MUST short-circuit to zero so the
        // round-trip-empty smoke test stays green after the 0r62
        // pre-image gained the per-partition contribution field.
        assert_eq!(fresh_session().state_hash(), StateHash::zero());
    }

    #[test]
    fn intersect_preserves_first_input_order() {
        // Internal helper sanity check: the intersection used by
        // valid_options must return options in the order of the
        // first-contributing partition's symbol-table walk.
        let first = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let next = vec!["c".to_string(), "a".to_string()];
        let out = intersect_preserving_order(&first, &next);
        assert_eq!(out, vec!["a".to_string(), "c".to_string()]);
    }
}
