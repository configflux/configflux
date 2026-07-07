// SPDX-License-Identifier: BUSL-1.1
//
// `Session::explain_rejection()` — labeled minimal-unsatisfiable-subset
// (MUS) extraction per ADR-0004 §4 (configflux-kv5d). Split out of
// `session.rs` as a sibling `impl Session<B>` block so neither file
// exceeds the repository line cap; the MUS walk is cohesive enough to
// stand alone (the same shape `resolve.rs` follows).
//
// # Boundaries (inviolable)
//
//   - **ADR-0003 §2**: no compiler type crosses into `solver/`. This
//     module returns the solver-owned `RejectionExplanation { core:
//     Option<LabeledCore> }` defined in `session.rs`; the labeled core
//     carries labeled `{facet}.{value}` strings only. The
//     interpreter/runtime wrappers (configflux-whyt / configflux-3b5y)
//     convert it into the compiler-side `UnsatCore`.
//   - **ADR-0004 §4**: `batsat::*` is confined to
//     `sat_backend_batsat.rs`. This module calls through the `SatBackend`
//     trait only — it never names a batsat type.
//   - **Fail-closed (ADR-0030 D4 / ADR-0031 D4)**: a MUS-extraction fault,
//     an inconclusive solve, or a MUS variable index that does not resolve
//     to a symbol-table name is an `Err`, never a partial core and never a
//     raw BDD/batsat variable index in the returned value.
//
// # Algorithm (ADR-0004 §4 steps (a)–(f), Marques-Silva & Lynce 2011)
//
// (a) **CNF serialization.** The composite feasible space is the
//     conjunction across all partitions of each partition's `current`
//     formula (ADR-0012 §1). The CNF of a conjunction is the union of the
//     per-partition clause sets. Each partition's clauses come from
//     enumerating its BDD's **falsifying paths** (root → ⊥ terminal): a
//     path that ends at ⊥ is a partial assignment the formula forbids, so
//     its negation is one clause **over the original `{facet}.{value}`
//     symbol variables** — no Tseitin/auxiliary node variables are
//     introduced, so the MUS maps to user-meaningful constraints, not
//     encoding artifacts (Q2). Variable identity across partitions is by
//     **symbol name**: one SAT variable per distinct symbol across the
//     union of every partition's variable order (Q3).
//
// (b/c) Each model clause gets a fresh **selector** SAT variable `s_j` and
//     is added as `(¬s_j ∨ C_j)`. The candidate is a **fixed** assumption
//     literal. `solve_with_assumptions([all selectors, candidate])` must be
//     UNSAT (else the option is genuinely valid → `would_reject:false`).
//     Selectors are required: `sat_backend_batsat.rs` documents that a
//     conflict at decision level 0 yields an *empty* assumption core, so the
//     selectors force the conflict above level 0. They are never
//     symbol-table variables and never appear in the labeled output.
//
// (d) **Deletion-based shrinking** over the selectors: drop each, re-solve;
//     keep it only if removing it makes the remainder SAT. The survivors are
//     minimal. (e) Each survivor maps to its model clause; each clause
//     literal maps via the symbol table to a labeled `{facet}.{value}` — an
//     unmappable index is a fail-closed `Err`. (f) Returns `LabeledCore`.
//
// The full rationale (incl. Q2/Q3 dispositions) is recorded on the bd issue
// configflux-kv5d.

use std::collections::{BTreeMap, BTreeSet};

use crate::backend::{BackendError, SolverBackend};
use crate::ccm_format::{BddNode, TERMINAL_FALSE, TERMINAL_TRUE, TERMINAL_VAR_INDEX};
use crate::sat_backend::{SatBackend, SatLit, SatOutcome, SatVar};
use crate::sat_backend_batsat::BatsatBackend;
use crate::session::{
    CoreConstraintKind, Error, LabeledAtom, LabeledConstraint, LabeledCore, RejectionExplanation,
    Session,
};

/// Upper bound on BDD falsifying paths enumerated per partition before the
/// extraction fails closed. A single partition's committed feasible BDD is
/// small in every representative model (ADR-0004 "few-thousand valid
/// configurations"), and `explain_rejection` is a query-time tool on a
/// formula the BDD already committed as ground truth — not a hot loop. If a
/// pathological model exceeds this, returning `Err` (surfaced by callers as
/// `E_SELECTION_ENGINE_DIVERGENCE`) is correct: a truncated path set would
/// produce an unsound MUS, which the fail-closed rule forbids
/// (ADR-0030/0031 D4).
const MAX_FALSIFYING_PATHS_PER_PARTITION: usize = 100_000;

/// One CNF clause over global symbol indices, derived from a single BDD
/// falsifying path. Each `(global_var, positive)` literal is the negation
/// of the path's assignment to that variable: a path that took the
/// `high`/`true` branch at variable `v` (so the assignment is `v = 1`)
/// contributes the literal `¬v` (`positive = false`), and a path that took
/// the `low`/`false` branch contributes `v` (`positive = true`) — because
/// the clause is the negation of the forbidden conjunction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelClause {
    /// Literals as `(global symbol index, polarity)` pairs.
    literals: Vec<(u32, bool)>,
}

/// The intermediate, index-keyed core produced by the SAT phase before the
/// labeled mapping: the surviving model clauses (each already over global
/// symbol indices) plus the candidate's global index.
#[derive(Debug)]
struct IndexedCore {
    /// Global symbol index of the rejected candidate.
    rejected_global: u32,
    /// The minimal set of model clauses (each over global symbol indices).
    clauses: Vec<ModelClause>,
}

impl<B: SolverBackend> Session<B> {
    /// Explain why a `(facet, option)` pair would be rejected. Pure query.
    ///
    /// Real MUS extraction per ADR-0004 §4 (configflux-kv5d). The outcomes:
    ///
    ///   - **Genuinely valid option** → `Ok(RejectionExplanation {
    ///     would_reject: false, core: None })`. The candidate does not
    ///     conflict with the committed feasible space.
    ///   - **Genuine constraint conflict** → `Ok(RejectionExplanation {
    ///     would_reject: true, core: Some(labeled MUS) })`. The labeled
    ///     core carries `{facet}.{value}` names only.
    ///   - **Unknown facet / option** → the same typed errors `apply`
    ///     surfaces (`Error::UnknownFacet` / `Error::UnknownOption`), so
    ///     callers can route the division-of-labor cases (ADR-0030 D5)
    ///     without a core. These are *not* a MUS fault.
    ///   - **MUS-extraction fault, inconclusive solve, or an unmappable
    ///     variable** → `Err` (fail closed, ADR-0030/0031 D4). Never a
    ///     partial core, never a raw index.
    ///
    /// Empty-Ccm path: an unsymbolled session has no constraints, so every
    /// `(facet, option)` is trivially "unknown" → `Error::UnknownOption`,
    /// matching the `apply`/`valid_options` contract for that path.
    pub fn explain_rejection(
        &self,
        facet: &str,
        option: &str,
    ) -> Result<RejectionExplanation, Error> {
        // Empty-Ccm shortcut: no symbol table at all. There is no modeled
        // option to explain — surface the same typed error `apply` does.
        if self.ccm().symbols().is_none() && self.multi_part_is_none_for_explain() {
            return Err(Error::UnknownOption {
                facet: facet.to_string(),
                value: option.to_string(),
            });
        }

        let compound = format!("{facet}.{option}");

        // Build the global symbol table: one entry per distinct symbol name
        // across the union of every partition's variable order, in a stable
        // (name-sorted) order. Identity across partitions is by name (Q3).
        let symbol_index = GlobalSymbolIndex::from_session(self);

        // The candidate must be a known symbol. An unknown `(facet, option)`
        // is a division-of-labor case (ADR-0030 D5), the same typed error
        // `apply` raises — distinct from a genuine MUS fault. Distinguish a
        // facet that exists with a different value (UnknownOption) from a
        // facet absent entirely (UnknownFacet), matching `apply`/`valid_options`.
        let Some(rejected_global) = symbol_index.index_of(&compound) else {
            let facet_prefix = format!("{facet}.");
            if symbol_index.any_with_prefix(&facet_prefix) {
                return Err(Error::UnknownOption {
                    facet: facet.to_string(),
                    value: option.to_string(),
                });
            }
            return Err(Error::UnknownFacet(facet.to_string()));
        };

        // (a) Serialize the composite feasible space into CNF over global
        // symbol indices: the union of every partition's falsifying-path
        // clauses. An empty clause set means the formula is ⊤ (no
        // constraints) so nothing can reject the candidate.
        let clauses = serialize_composite_cnf(self, &symbol_index)?;

        // (b)–(d) Run the selector-based assumption MUS over the SAT
        // backend. Returns `None` when the candidate is genuinely valid
        // (the assumption solve is SAT), or `Some(indexed minimal core)`.
        let Some(indexed) = extract_indexed_mus(&clauses, rejected_global)? else {
            // Genuinely valid option: no conflict, no core (ADR-0030 D5).
            return Ok(RejectionExplanation {
                would_reject: false,
                core: None,
            });
        };

        // (e)/(f) Map the indexed core back to labeled `{facet}.{value}`
        // names. An unmappable index fails closed (ADR-0031 D4).
        let core = label_indexed_core(self, &symbol_index, indexed)?;
        Ok(RejectionExplanation {
            would_reject: true,
            core: Some(core),
        })
    }
}

/// The union symbol table across all partitions, keyed by symbol name.
/// Identity across partitions is by name, not by per-partition BDD index
/// (Q3): a `{facet}.{value}` that lives in two partitions is the *same*
/// global SAT variable. The order is name-sorted so the global indices are
/// deterministic across runs.
struct GlobalSymbolIndex {
    /// Global index → symbol name, in name-sorted order.
    names: Vec<String>,
    /// Symbol name → global index.
    by_name: BTreeMap<String, u32>,
}

impl GlobalSymbolIndex {
    /// Build the union index from every partition's `variable_order`.
    fn from_session<B: SolverBackend>(session: &Session<B>) -> Self {
        let mut set: BTreeSet<String> = BTreeSet::new();
        for order in session.partition_variable_orders_for_explain() {
            for sym in order {
                set.insert(sym.clone());
            }
        }
        let names: Vec<String> = set.into_iter().collect();
        let by_name = names
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();
        Self { names, by_name }
    }

    /// Global index for a symbol name, if present.
    fn index_of(&self, symbol: &str) -> Option<u32> {
        self.by_name.get(symbol).copied()
    }

    /// Symbol name for a global index, if in range.
    fn name_of(&self, global: u32) -> Option<&str> {
        self.names.get(global as usize).map(String::as_str)
    }

    /// Whether any symbol name starts with `prefix` (used to tell an
    /// unknown option from an unknown facet).
    fn any_with_prefix(&self, prefix: &str) -> bool {
        self.by_name.keys().any(|k| k.starts_with(prefix))
    }
}

/// Serialize the composite feasible space into CNF over global symbol
/// indices (ADR-0004 §4 step (a)). The CNF of a conjunction is the union of
/// the conjuncts' clause sets, so this concatenates each partition's
/// falsifying-path clauses, remapping each partition's local BDD variable
/// index to the shared global symbol index by name.
fn serialize_composite_cnf<B: SolverBackend>(
    session: &Session<B>,
    symbol_index: &GlobalSymbolIndex,
) -> Result<Vec<ModelClause>, Error> {
    let mut clauses: Vec<ModelClause> = Vec::new();

    // Model-rule clauses: the union of every partition's original-BDD
    // falsifying-path clauses (the conjunction's CNF is the union per
    // partition). The committed *current* formula is `original ∧
    // pinned-selections`; the `current` narrowed handle is a backend-opaque
    // BDD with no clause surface (ADR-0003 §3 keeps `SolverBackend`
    // minimal), so explain reconstructs the committed formula from the
    // serialized original node table plus the pinned-selection unit clauses
    // added below — semantically identical to walking `current` directly.
    for part in session.partition_bdd_sources_for_explain() {
        let Some((order, nodes, root)) = part else {
            // A partition with no BDD source is the empty-constant-true
            // partition (empty-Ccm path). It contributes ⊤ → no clauses.
            continue;
        };
        // Local index → global index, by symbol name. Every local symbol
        // must be in the global union (it was built from these same orders).
        let mut local_to_global: Vec<u32> = Vec::with_capacity(order.len());
        for sym in order {
            let g = symbol_index.index_of(sym).ok_or(Error::Backend(
                BackendError::Invariant(
                    "explain: partition symbol missing from the global union index",
                ),
            ))?;
            local_to_global.push(g);
        }
        enumerate_falsifying_clauses(nodes, root, &local_to_global, &mut clauses)?;
    }

    // Pinned-selection unit clauses: each committed `{facet}.{value}`
    // becomes the unit clause `(x)` forcing that symbol true. This is what
    // turns a candidate that is valid in the *original* model into one the
    // *committed* state forbids — and lets the MUS surface a prior
    // `Selection` as its own conflicting constraint (classified in
    // `label_indexed_core`). A pinned symbol must be in the global union.
    for compound in session.pinned_symbols_for_explain() {
        let g = symbol_index.index_of(&compound).ok_or(Error::Backend(
            BackendError::Invariant(
                "explain: pinned selection missing from the global union index",
            ),
        ))?;
        clauses.push(ModelClause {
            literals: vec![(g, true)],
        });
    }

    Ok(clauses)
}

/// Classify a child / root BDD reference as a terminal. ADR-0005 §4 admits
/// two encodings for the ⊥ / ⊤ terminals and the explain walker must accept
/// both, exactly as `backend_oxidd::lookup_child` / `deserialize_bdd` and the
/// `ccm_format` validator already do: the **sentinels** `TERMINAL_FALSE` /
/// `TERMINAL_TRUE` (used for a *constant* root and the terminal nodes' own
/// backpointers), or a **table index** `0` (⊥) / `1` (⊤) pointing at a
/// terminal-tagged node — the form the real oxidd/cudd serializer emits for
/// every non-terminal child reference (`ccm_emitter/bdd.rs::child_id`).
///
/// `Ok(Some(true/false))` = ⊤/⊥; `Ok(None)` = a non-terminal node (the caller
/// recurses); `Err` (fail closed) = a terminal-tagged node at an index other
/// than 0/1, or an out-of-range index — both malformed tables (ADR-0031 D4).
fn resolve_terminal_ref(nodes: &[BddNode], id: u32) -> Result<Option<bool>, Error> {
    // Sentinel form first: these values are out of any real table's index
    // range, so they can never collide with a node index.
    if id == TERMINAL_TRUE {
        return Ok(Some(true));
    }
    if id == TERMINAL_FALSE {
        return Ok(Some(false));
    }
    // Index form: only a terminal-*tagged* node at index 0/1 is a terminal; a
    // non-terminal node reference returns `None` so the caller recurses.
    match nodes.get(id as usize) {
        Some(node) if node.var_index == TERMINAL_VAR_INDEX => match id {
            0 => Ok(Some(false)),
            1 => Ok(Some(true)),
            _ => Err(Error::Backend(BackendError::Invariant(
                "explain: terminal-tagged node at an index other than 0 (⊥) or 1 (⊤)",
            ))),
        },
        Some(_) => Ok(None),
        None => Err(Error::Backend(BackendError::Invariant(
            "explain: BDD child index out of range during path walk",
        ))),
    }
}

/// Enumerate the falsifying paths of one BDD (root → ⊥ terminal) and append
/// one clause per path to `out`. Each clause is the negation of the path's
/// forbidden partial assignment, over global symbol indices.
///
/// The node table follows ADR-0005 §4 exactly (mirrors
/// `backend_oxidd::deserialize_bdd`): index 0 = ⊥, index 1 = ⊤, both tagged
/// `TERMINAL_VAR_INDEX`; a non-terminal node is `ite(var, high, low)` with
/// `low_id`/`high_id` either a terminal sentinel, a terminal-by-index
/// reference (0/1), or an index into `nodes`. A path that descends `high`
/// assigns `var = 1`; descending `low` assigns `var = 0`. Only paths
/// terminating at ⊥ are forbidden.
fn enumerate_falsifying_clauses(
    nodes: &[BddNode],
    root: u32,
    local_to_global: &[u32],
    out: &mut Vec<ModelClause>,
) -> Result<(), Error> {
    // A constant-⊤ root forbids nothing; a constant-⊥ root forbids everything
    // (the empty clause), making the formula unsatisfiable on its own. The
    // root may be a sentinel (the constant-root form) or a terminal-by-index
    // reference; `resolve_terminal_ref` accepts both.
    match resolve_terminal_ref(nodes, root)? {
        Some(true) => return Ok(()),
        Some(false) => {
            out.push(ModelClause {
                literals: Vec::new(),
            });
            return Ok(());
        }
        None => { /* non-terminal root: walk it below */ }
    }

    // Recursive DFS over the node table. Depth is bounded by the variable
    // count (a path visits each BDD level at most once) and is small per
    // partition, so native recursion is safe and clearest. `assignment` is
    // the current root→node path as `(local_var, assigned_true)` literals.
    let mut assignment: Vec<(u32, bool)> = Vec::new();
    let mut paths_emitted: usize = 0;
    walk_node(
        nodes,
        root,
        local_to_global,
        &mut assignment,
        out,
        &mut paths_emitted,
    )
}

/// Recursive worker for [`enumerate_falsifying_clauses`].
fn walk_node(
    nodes: &[BddNode],
    node_id: u32,
    local_to_global: &[u32],
    assignment: &mut Vec<(u32, bool)>,
    out: &mut Vec<ModelClause>,
    paths_emitted: &mut usize,
) -> Result<(), Error> {
    // Terminals, in either ADR-0005 §4 form (sentinel or terminal-by-index);
    // `resolve_terminal_ref` returns `None` only for a genuine non-terminal
    // node, read below.
    match resolve_terminal_ref(nodes, node_id)? {
        Some(true) => return Ok(()), // satisfying path — not forbidden
        Some(false) => {
            out.push(clause_from_assignment(assignment, local_to_global)?);
            *paths_emitted += 1;
            if *paths_emitted > MAX_FALSIFYING_PATHS_PER_PARTITION {
                return Err(Error::Backend(BackendError::Invariant(
                    "explain: BDD falsifying-path enumeration exceeded the safety cap",
                )));
            }
            return Ok(());
        }
        None => { /* non-terminal node: read it below */ }
    }

    // `resolve_terminal_ref` already range-checked the index and confirmed the
    // node is non-terminal; keep the guard so a future bypass still fails closed.
    let node = nodes.get(node_id as usize).ok_or(Error::Backend(
        BackendError::Invariant("explain: BDD child index out of range during path walk"),
    ))?;
    if node.var_index as usize >= local_to_global.len() {
        return Err(Error::Backend(BackendError::Invariant(
            "explain: BDD node var_index out of range vs symbol table",
        )));
    }
    let var = node.var_index;

    // Low branch: var = 0.
    assignment.push((var, false));
    walk_node(
        nodes,
        node.low_id,
        local_to_global,
        assignment,
        out,
        paths_emitted,
    )?;
    assignment.pop();

    // High branch: var = 1.
    assignment.push((var, true));
    walk_node(
        nodes,
        node.high_id,
        local_to_global,
        assignment,
        out,
        paths_emitted,
    )?;
    assignment.pop();

    Ok(())
}

/// Build one CNF clause from a forbidden partial assignment: the clause is
/// the disjunction of the negations of the assignment literals, remapped to
/// global symbol indices. A path that assigned `var = 1` contributes `¬var`
/// (`positive = false`); a path that assigned `var = 0` contributes `var`
/// (`positive = true`).
fn clause_from_assignment(
    assignment: &[(u32, bool)],
    local_to_global: &[u32],
) -> Result<ModelClause, Error> {
    let mut literals: Vec<(u32, bool)> = Vec::with_capacity(assignment.len());
    for &(local_var, assigned_true) in assignment {
        let global = *local_to_global.get(local_var as usize).ok_or(Error::Backend(
            BackendError::Invariant("explain: assignment variable out of range vs symbol table"),
        ))?;
        // Negate the assignment for the clause literal.
        literals.push((global, !assigned_true));
    }
    Ok(ModelClause { literals })
}

/// Run the selector-based assumption MUS extraction (ADR-0004 §4 steps
/// (b)–(d)) over the `SatBackend` trait. Returns:
///
///   - `Ok(None)` — the candidate is genuinely valid: asserting it together
///     with the full model is satisfiable, so there is no conflict.
///   - `Ok(Some(IndexedCore))` — the minimal set of model clauses (over
///     global symbol indices) whose conjunction with the candidate is
///     unsatisfiable.
///   - `Err(..)` — a SAT fault or an inconclusive solve (fail closed).
fn extract_indexed_mus(
    clauses: &[ModelClause],
    rejected_global: u32,
) -> Result<Option<IndexedCore>, Error> {
    let mut backend = BatsatBackend::new_solver();

    // Determine the highest global symbol index referenced so we can
    // allocate the symbol variables densely (SatVar indices are allocation
    // order; we keep `symbol var i == global symbol index i`).
    let mut max_global = rejected_global;
    for c in clauses {
        for &(g, _) in &c.literals {
            max_global = max_global.max(g);
        }
    }
    // Allocate one SAT variable per global symbol index 0..=max_global.
    let mut symbol_vars: Vec<SatVar> = Vec::with_capacity(max_global as usize + 1);
    for _ in 0..=max_global {
        symbol_vars.push(backend.new_var());
    }

    // For each model clause C_j, allocate a fresh selector s_j and add the
    // guarded clause `(¬s_j ∨ C_j)`. A clause with no literals (a ⊥-root
    // partition) becomes `(¬s_j)` — selecting it forces unsat, which is the
    // correct meaning of an already-unsatisfiable composite formula.
    let mut selectors: Vec<SatVar> = Vec::with_capacity(clauses.len());
    for clause in clauses {
        let selector = backend.new_var();
        selectors.push(selector);
        let mut lits: Vec<SatLit> = Vec::with_capacity(clause.literals.len() + 1);
        lits.push(SatLit::negative(selector)); // ¬s_j
        for &(g, positive) in &clause.literals {
            let var = symbol_vars[g as usize];
            lits.push(if positive {
                SatLit::positive(var)
            } else {
                SatLit::negative(var)
            });
        }
        // `add_clause` returning TopLevelUnsat would mean a unit selector
        // clause drove the database unsat on insertion; for a guarded clause
        // (always containing the fresh ¬s_j literal) that cannot happen, so
        // any such report is a real fault → fail closed.
        backend
            .add_clause(&lits)
            .map_err(|_| sat_fault("add_clause failed while seeding model clauses"))?;
    }

    // The candidate is a FIXED assumption literal (the positive literal for
    // its symbol): we are explaining "what forbids selecting it". It is
    // never dropped during shrinking — the rejected atom is reported
    // separately, not as a conflicting constraint.
    let candidate_lit = SatLit::positive(symbol_vars[rejected_global as usize]);

    // Step (c): confirm UNSAT with ALL selectors plus the candidate on.
    let assumptions_all: Vec<SatLit> = selectors
        .iter()
        .map(|&s| SatLit::positive(s))
        .chain(std::iter::once(candidate_lit))
        .collect();
    match backend
        .solve_with_assumptions(&assumptions_all)
        .map_err(|_| sat_fault("initial solve_with_assumptions failed"))?
    {
        SatOutcome::Sat => {
            // The candidate is satisfiable alongside every model clause —
            // genuinely valid, no core.
            return Ok(None);
        }
        SatOutcome::Unsat => { /* proceed to shrink */ }
        SatOutcome::Unknown => {
            // A sound MUS needs a definite UNSAT (ADR-0004 §4); an
            // inconclusive solve fails closed.
            return Err(sat_fault("initial solve was inconclusive (resource limit)"));
        }
    }

    // Step (d): deletion-based shrinking over the selectors. `kept` is the
    // set of selectors currently believed necessary; we try removing each
    // one and keep it only if removing it makes the remainder SAT.
    let mut kept: Vec<SatVar> = selectors.clone();
    let mut i = 0;
    while i < kept.len() {
        // Trial assumption set: every kept selector EXCEPT kept[i], plus the
        // fixed candidate.
        let trial: Vec<SatLit> = kept
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, &s)| SatLit::positive(s))
            .chain(std::iter::once(candidate_lit))
            .collect();
        match backend
            .solve_with_assumptions(&trial)
            .map_err(|_| sat_fault("shrink solve_with_assumptions failed"))?
        {
            SatOutcome::Unsat => {
                // kept[i] is not needed — drop it permanently and re-test the
                // element that slid into index i.
                kept.remove(i);
            }
            SatOutcome::Sat => {
                // kept[i] is necessary — keep it and advance.
                i += 1;
            }
            SatOutcome::Unknown => {
                return Err(sat_fault("shrink solve was inconclusive (resource limit)"));
            }
        }
    }

    // Map the kept selectors back to their model clauses (by allocation
    // order: selectors[k] corresponds to clauses[k]).
    let mut kept_set: BTreeSet<u32> = BTreeSet::new();
    for s in &kept {
        kept_set.insert(s.index());
    }
    let mut surviving: Vec<ModelClause> = Vec::with_capacity(kept.len());
    for (k, selector) in selectors.iter().enumerate() {
        if kept_set.contains(&selector.index()) {
            surviving.push(clauses[k].clone());
        }
    }

    Ok(Some(IndexedCore {
        rejected_global,
        clauses: surviving,
    }))
}

/// A SAT-phase fault, surfaced as `Error::Backend(Invariant(..))` so callers
/// see the fail-closed signal (ADR-0030/0031 D4). The SAT side is internal;
/// it never leaks a batsat type, so the BDD-side `BackendError::Invariant`
/// is the natural carrier (the public `Error` enum has no SAT variant and
/// growing it is out of scope for kv5d).
fn sat_fault(msg: &'static str) -> Error {
    Error::Backend(BackendError::Invariant(msg))
}

/// Map the index-keyed minimal core back to a labeled [`LabeledCore`]
/// (ADR-0004 §4 steps (e)/(f)). Every global symbol index MUST resolve to a
/// symbol-table name; an unmappable index is a fail-closed `Err`
/// (ADR-0031 D4) — never a raw index in the returned value.
fn label_indexed_core<B: SolverBackend>(
    session: &Session<B>,
    symbol_index: &GlobalSymbolIndex,
    indexed: IndexedCore,
) -> Result<LabeledCore, Error> {
    // The rejected candidate's labeled atom.
    let rejected_name = symbol_index.name_of(indexed.rejected_global).ok_or(Error::Backend(
        BackendError::Invariant("explain: rejected candidate index has no symbol-table label"),
    ))?;
    let rejected = atom_from_symbol(rejected_name)?;

    // The set of `{facet}.{value}` the session has already pinned, used to
    // classify a single-literal clause as a prior Selection vs a ModelRule.
    let pinned = session.pinned_symbols_for_explain();

    let mut constraints: Vec<LabeledConstraint> = Vec::with_capacity(indexed.clauses.len());
    for clause in &indexed.clauses {
        let mut atoms: Vec<LabeledAtom> = Vec::with_capacity(clause.literals.len());
        for &(global, _positive) in &clause.literals {
            let name = symbol_index.name_of(global).ok_or(Error::Backend(
                BackendError::Invariant(
                    "explain: MUS clause variable index has no symbol-table label",
                ),
            ))?;
            atoms.push(atom_from_symbol(name)?);
        }
        atoms.sort();
        atoms.dedup();

        // Classification (recorded design decision, bounded for v0.4.0): a
        // single-atom clause whose atom is a currently-pinned selection is a
        // `Selection`; everything else is a `ModelRule`. A multi-atom clause
        // is always a model rule (a `requires`/`excludes`-style relation).
        let kind = if atoms.len() == 1 {
            let only = &atoms[0];
            let compound = format!("{}.{}", only.facet, only.value);
            if pinned.contains(&compound) {
                CoreConstraintKind::Selection
            } else {
                CoreConstraintKind::ModelRule
            }
        } else {
            CoreConstraintKind::ModelRule
        };

        constraints.push(LabeledConstraint { kind, atoms });
    }

    // Deterministic ordering of the conflicting constraints so the labeled
    // output shape is stable for a given MUS witness: by kind (Selection
    // before ModelRule) then by the atom list.
    constraints.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| a.atoms.cmp(&b.atoms))
    });
    constraints.dedup();

    Ok(LabeledCore {
        rejected,
        conflicting_constraints: constraints,
        minimal: true,
    })
}

/// Split a `{facet}.{value}` symbol into a [`LabeledAtom`]. A symbol with no
/// `.` separator is a malformed symbol table (every solver symbol is
/// `{facet}.{value}` per ADR-0005 §3) → fail closed.
fn atom_from_symbol(symbol: &str) -> Result<LabeledAtom, Error> {
    let (facet, value) = symbol.rsplit_once('.').ok_or(Error::Backend(
        BackendError::Invariant("explain: symbol is not in {facet}.{value} form"),
    ))?;
    Ok(LabeledAtom {
        facet: facet.to_string(),
        value: value.to_string(),
    })
}

/// Stable rank for the constraint-kind sort (Selection before ModelRule).
fn kind_rank(kind: CoreConstraintKind) -> u8 {
    match kind {
        CoreConstraintKind::Selection => 0,
        CoreConstraintKind::ModelRule => 1,
    }
}

#[cfg(test)]
mod tests {
    //! White-box unit tests for the MUS-extraction internals. The
    //! end-to-end labeled behaviour is covered by the
    //! `explain_rejection_mus` integration test; these pin the pieces that
    //! are hard to reach through the public API — the BDD falsifying-path
    //! walk, the clause-from-assignment negation, and the **fail-closed**
    //! mapping when a MUS variable index does not resolve to a symbol-table
    //! name (ADR-0031 D4 acceptance criterion).
    use super::*;
    use crate::backend::NullBackend;
    use crate::ccm::Ccm;

    fn node(var: u32, low: u32, high: u32) -> BddNode {
        BddNode {
            var_index: var,
            low_id: low,
            high_id: high,
            flags: 0,
        }
    }

    /// The `engine.v6 XOR engine.v8` node table (var order [v6=0, v8=1]):
    ///   0: ⊥, 1: ⊤, 2: ¬v8, 3: v8, 4: v6 (root).
    fn xor_nodes() -> Vec<BddNode> {
        vec![
            node(TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE), // 0: ⊥
            node(TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE),   // 1: ⊤
            node(1, TERMINAL_TRUE, TERMINAL_FALSE),                   // 2: ¬v8
            node(1, TERMINAL_FALSE, TERMINAL_TRUE),                   // 3: v8
            node(0, 3, 2),                                           // 4: v6
        ]
    }

    #[test]
    fn falsifying_paths_of_xor_are_the_two_exactly_one_clauses() {
        // Identity local→global map [0,1]. XOR forbids (v6=0,v8=0) and
        // (v6=1,v8=1), so the clauses are `(v6 ∨ v8)` and `(¬v6 ∨ ¬v8)`.
        let nodes = xor_nodes();
        let local_to_global = [0u32, 1u32];
        let mut out: Vec<ModelClause> = Vec::new();
        enumerate_falsifying_clauses(&nodes, 4, &local_to_global, &mut out)
            .expect("walk must succeed on a well-formed table");
        assert_eq!(out.len(), 2, "XOR has exactly two falsifying paths");
        // (v6=0,v8=0) → (v6 ∨ v8): both positive literals.
        assert!(
            out.contains(&ModelClause {
                literals: vec![(0, true), (1, true)],
            }),
            "missing the at-least-one clause (v6 ∨ v8); got {out:?}",
        );
        // (v6=1,v8=1) → (¬v6 ∨ ¬v8): both negative literals.
        assert!(
            out.contains(&ModelClause {
                literals: vec![(0, false), (1, false)],
            }),
            "missing the at-most-one clause (¬v6 ∨ ¬v8); got {out:?}",
        );
    }

    #[test]
    fn constant_true_and_false_roots_are_handled() {
        let nodes = xor_nodes();
        let mut out: Vec<ModelClause> = Vec::new();
        // ⊤ root forbids nothing.
        enumerate_falsifying_clauses(&nodes, TERMINAL_TRUE, &[0, 1], &mut out)
            .expect("⊤ root is fine");
        assert!(out.is_empty(), "⊤ forbids nothing");
        // ⊥ root forbids everything → the empty clause.
        enumerate_falsifying_clauses(&nodes, TERMINAL_FALSE, &[0, 1], &mut out)
            .expect("⊥ root is fine");
        assert_eq!(out, vec![ModelClause { literals: Vec::new() }]);
    }

    /// The same XOR table but with terminal children referenced BY INDEX
    /// (0 = ⊥, 1 = ⊤) instead of sentinels — the real oxidd/cudd serializer's
    /// convention (`ccm_emitter/bdd.rs::child_id`). Before configflux-autp the
    /// walk faulted here; it must now produce the identical two clauses.
    fn xor_nodes_index_terminals() -> Vec<BddNode> {
        vec![
            node(TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE), // 0: ⊥
            node(TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE),   // 1: ⊤
            node(1, 1, 0), // 2: ¬v8  (low→⊤ idx1, high→⊥ idx0)
            node(1, 0, 1), // 3:  v8  (low→⊥ idx0, high→⊤ idx1)
            node(0, 3, 2), // 4:  v6 (root)
        ]
    }

    #[test]
    fn index_based_terminal_refs_walk_like_sentinels() {
        // Regression (configflux-autp): index-form terminal children must walk.
        let nodes = xor_nodes_index_terminals();
        let mut out: Vec<ModelClause> = Vec::new();
        enumerate_falsifying_clauses(&nodes, 4, &[0u32, 1u32], &mut out)
            .expect("index-based terminal references must walk, not fault");
        assert_eq!(out.len(), 2, "XOR still has exactly two falsifying paths");
        assert!(out.contains(&ModelClause {
            literals: vec![(0, true), (1, true)],
        }));
        assert!(out.contains(&ModelClause {
            literals: vec![(0, false), (1, false)],
        }));
        // A constant root supplied as the ⊥ terminal BY INDEX (0) also works.
        let mut out2: Vec<ModelClause> = Vec::new();
        enumerate_falsifying_clauses(&nodes, 0, &[0u32, 1u32], &mut out2)
            .expect("⊥ root by index 0 is fine");
        assert_eq!(out2, vec![ModelClause { literals: Vec::new() }]);
    }

    #[test]
    fn clause_from_assignment_negates_each_literal() {
        // A path that assigned v0=1 and v1=0 forbids that combination, so
        // the clause is (¬v0 ∨ v1).
        let assignment = [(0u32, true), (1u32, false)];
        let local_to_global = [10u32, 11u32];
        let clause = clause_from_assignment(&assignment, &local_to_global)
            .expect("mapping in range");
        assert_eq!(
            clause,
            ModelClause {
                literals: vec![(10, false), (11, true)],
            },
        );
    }

    #[test]
    fn out_of_range_child_index_fails_closed() {
        // A node whose high_id points past the table is a malformed BDD;
        // the walk must return Err, never a partial clause set.
        let nodes = vec![
            node(TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE), // 0: ⊥
            node(TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE),   // 1: ⊤
            node(0, TERMINAL_FALSE, 99),                             // 2: child 99 oob
        ];
        let mut out: Vec<ModelClause> = Vec::new();
        let err = enumerate_falsifying_clauses(&nodes, 2, &[0], &mut out)
            .expect_err("an out-of-range child index must fail closed");
        assert!(matches!(err, Error::Backend(BackendError::Invariant(_))));
    }

    #[test]
    fn var_index_out_of_range_vs_symbol_table_fails_closed() {
        // A node referencing variable 5 when the symbol table has only one
        // entry is a corrupt table → Err.
        let nodes = vec![
            node(TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE),
            node(TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE),
            node(5, TERMINAL_FALSE, TERMINAL_TRUE), // var 5 not in [0]
        ];
        let mut out: Vec<ModelClause> = Vec::new();
        let err = enumerate_falsifying_clauses(&nodes, 2, &[0], &mut out)
            .expect_err("var_index out of range must fail closed");
        assert!(matches!(err, Error::Backend(BackendError::Invariant(_))));
    }

    #[test]
    fn unmappable_mus_variable_index_fails_closed() {
        // ADR-0031 D4 acceptance criterion: an MUS variable index that has
        // no symbol-table label must return Err — never a raw index, never a
        // partial core. We hand `label_indexed_core` a `GlobalSymbolIndex`
        // with a single labeled symbol, but an `IndexedCore` whose clause
        // references a global index (7) beyond it.
        let symbol_index = GlobalSymbolIndex {
            names: vec!["engine.v6".to_string()],
            by_name: std::iter::once(("engine.v6".to_string(), 0u32)).collect(),
        };
        // A throwaway empty session for the `pinned_symbols_for_explain`
        // call inside `label_indexed_core` (returns the empty pin set).
        let session = Session::<NullBackend>::new(Ccm::empty()).expect("empty session");

        // The rejected atom resolves (index 0), but a clause references the
        // unmappable index 7.
        let indexed = IndexedCore {
            rejected_global: 0,
            clauses: vec![ModelClause {
                literals: vec![(7, false)],
            }],
        };
        let err = label_indexed_core(&session, &symbol_index, indexed)
            .expect_err("an unmappable MUS variable index must fail closed");
        assert!(
            matches!(err, Error::Backend(BackendError::Invariant(_))),
            "unmappable index must be a fail-closed Invariant error, got {err:?}",
        );

        // And an unmappable *rejected* index likewise fails closed.
        let indexed2 = IndexedCore {
            rejected_global: 7, // beyond the one-symbol table
            clauses: Vec::new(),
        };
        let err2 = label_indexed_core(&session, &symbol_index, indexed2)
            .expect_err("an unmappable rejected index must fail closed");
        assert!(matches!(err2, Error::Backend(BackendError::Invariant(_))));
    }

    #[test]
    fn label_maps_clauses_to_named_atoms_with_no_integers() {
        // A well-formed indexed core maps to labeled atoms; the rejected
        // atom and every clause atom are `{facet}.{value}` names.
        let symbol_index = GlobalSymbolIndex {
            names: vec!["engine.v6".to_string(), "engine.v8".to_string()],
            by_name: [("engine.v6".to_string(), 0u32), ("engine.v8".to_string(), 1u32)]
                .into_iter()
                .collect(),
        };
        let session = Session::<NullBackend>::new(Ccm::empty()).expect("empty session");
        let indexed = IndexedCore {
            rejected_global: 1, // engine.v8
            clauses: vec![ModelClause {
                literals: vec![(0, false), (1, false)], // ¬v6 ∨ ¬v8
            }],
        };
        let core = label_indexed_core(&session, &symbol_index, indexed)
            .expect("well-formed core must map");
        assert_eq!(
            core.rejected,
            LabeledAtom {
                facet: "engine".to_string(),
                value: "v8".to_string(),
            },
        );
        assert!(core.minimal);
        assert_eq!(core.conflicting_constraints.len(), 1);
        let c = &core.conflicting_constraints[0];
        // No pins on the empty session ⇒ the two-atom clause is a ModelRule.
        assert_eq!(c.kind, CoreConstraintKind::ModelRule);
        assert_eq!(
            c.atoms,
            vec![
                LabeledAtom {
                    facet: "engine".to_string(),
                    value: "v6".to_string(),
                },
                LabeledAtom {
                    facet: "engine".to_string(),
                    value: "v8".to_string(),
                },
            ],
        );
    }
}
