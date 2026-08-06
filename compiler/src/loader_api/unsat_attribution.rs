// SPDX-License-Identifier: BUSL-1.1
//
// Constraint attribution for a labeled unsat core — ADR-0054 §5.4, the
// consumer side of the `.ccm` constraint roster (configflux-p571.8).
//
// # The problem
//
// A BDD root has no notion of clause identity. The solver's minimal unsat
// core (ADR-0004 §4) is a set of CNF clauses derived from the composite
// BDD's falsifying paths, and a path clause carries only the `{facet}.{value}`
// variables it happened to branch on — including variables that are pure
// bookkeeping. Reported raw, the hero example's core names
// `beta_dashboard.off` and `environment.dev` alongside the two atoms that
// actually matter, and never names `prod_forbids_debug` at all.
//
// # The mapping
//
// The top-level `ccm.manifest.json` carries the ADR-0054 §5.4 roster:
// `{id, condition, root_index}` per **authored** constraint. A model clause
// is the negation of a forbidden partial assignment, so the assignment it
// forbids can be reconstructed from the clause's signed literals. The
// declared constraints that account for that clause are exactly those the
// assignment **violates** — `eval_partial(condition, assignment) == False`,
// the same rule ADR-0054 §2/§6 use to fail `cfx resolve` closed. Reusing the
// resolve rule is deliberate: `cfx explain` must name the constraint
// `cfx resolve` would reject on, not a differently-derived one.
//
// # Synthesized cardinality is never named (§5.4, hard rule)
//
// Per-facet `exactly_one_of` / at-most-one conjuncts are NOT in the roster.
// They are not authored policy, and a clause that reduces to one is a
// statement about the *model*, not about a policy the user broke. Such a
// clause attributes to nothing, and when NO clause in a core attributes to a
// declared constraint the core is reported as the model being
// over-constrained — never by borrowing the nearest constraint's name.
//
// # Boundaries
//
// This is a pure function over plain data. It names no solver type
// (ADR-0003 §2 — the compiler never imports the solver); callers translate
// their own core representation into [`CoreClause`] first. It performs no
// I/O and reads no global state, so it is exhaustively unit-testable.

/// One declared constraint from the top-level `ccm.manifest.json` roster
/// (ADR-0054 §5.4), as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredConstraint {
    /// The authored `constraints:` entry id, e.g. `prod_forbids_debug`.
    pub id: String,
    /// The authored condition text, verbatim.
    pub condition: String,
    /// Position in the root AND-fold among the authored conjuncts. The
    /// reporting order: it is the model's own declaration order, stable
    /// across recompiles of the same sources.
    pub root_index: u32,
}

/// One clause of a labeled unsat core, as the caller's solver produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreClause {
    /// Prior selection or model rule (ADR-0031 D3).
    pub kind: ConstraintKind,
    /// The clause's labeled atoms, sign dropped — the ADR-0031 D3 `facets`
    /// shape.
    pub facets: Vec<ConstraintFacet>,
    /// The caller's advisory gloss, carried through verbatim for a
    /// `Selection`. Ignored for a `ModelRule`, whose gloss this function
    /// derives.
    pub summary: String,
    /// The signed partial assignment the clause forbids: `true` means the
    /// forbidden assignment took that option, `false` means it ruled it out.
    /// Empty for a `Selection` (its meaning is the atom itself).
    pub forbidden: Vec<(ConstraintFacet, bool)>,
}

/// The gloss for a model clause no declared constraint accounts for
/// (ADR-0054 §5.4). Says the model over-constrains the combination without
/// naming a synthesized cardinality conjunct as if it were policy.
pub const MODEL_OVER_CONSTRAINED_SUMMARY: &str =
    "the model is over-constrained here; no declared constraint accounts for this conflict";

/// Rewrite a labeled core's clauses so declared constraints are named by id
/// (ADR-0054 §5.4). `rejected` is the candidate being explained.
///
/// - `Selection` clauses pass through verbatim, in input order.
/// - `ModelRule` clauses are attributed to the declared constraints their
///   forbidden assignment violates. The result carries **one entry per
///   distinct constraint**, `root_index` ascending, and the unattributed
///   clauses are dropped: they are the model's cardinality bookkeeping, and
///   §5.4 forbids naming them.
/// - When no clause attributes to anything, a single `constraint_id: None`
///   entry reports the core as model-over-constrained, carrying the union of
///   the unattributed atoms.
///
/// **Candidate relevance.** A BDD falsifying path is not a prime implicate:
/// it carries every variable the walk branched on, including facets the user
/// never chose. Such a path can incidentally violate a constraint that has
/// nothing to do with the rejection — on the `s_labeled_mus` fixture,
/// explaining `psu.bronze` otherwise drags in the unrelated cooling rule. So
/// a constraint is preferred when the *candidate* is what breaks it: it is
/// violated by the assignment but NOT by the same assignment with the
/// candidate removed. Preferred, not required — if nothing clears that bar
/// the plain violation set is reported rather than degrading a real
/// explanation to "over-constrained".
///
/// A roster entry whose `condition` does not parse is skipped rather than
/// guessed at: naming a constraint the evaluator could not check would be
/// exactly the "paper over it with the nearest constraint" failure §5.4
/// rules out.
pub fn attribute_core_clauses(
    clauses: &[CoreClause],
    rejected: &ConstraintFacet,
    roster: &[DeclaredConstraint],
) -> Vec<ConflictingConstraint> {
    let mut out: Vec<ConflictingConstraint> = Vec::new();
    for clause in clauses.iter().filter(|c| c.kind == ConstraintKind::Selection) {
        out.push(ConflictingConstraint {
            kind: ConstraintKind::Selection,
            facets: clause.facets.clone(),
            summary: clause.summary.clone(),
            constraint_id: None,
        });
    }

    // Both keyed by (root_index, id) so the report order is the model's own
    // declaration order and is total even if two entries shared an index.
    let mut candidate_relevant: BTreeMap<(u32, String), NamedConstraint> = BTreeMap::new();
    let mut all_violated: BTreeMap<(u32, String), NamedConstraint> = BTreeMap::new();
    let mut unattributed: BTreeSet<(String, String)> = BTreeSet::new();
    let mut saw_model_clause = false;

    for clause in clauses.iter().filter(|c| c.kind == ConstraintKind::ModelRule) {
        saw_model_clause = true;
        // A forbidden assignment that takes two values of one facet is an
        // intra-facet at-most-one clause by construction. It cannot be a
        // policy violation — no resolvable configuration looks like that.
        let Some(assignment) = forbidden_assignment(&clause.forbidden) else {
            record_unattributed(&clause.facets, &mut unattributed);
            continue;
        };
        let hits = violated_declarations(roster, &assignment);
        if hits.is_empty() {
            record_unattributed(&clause.facets, &mut unattributed);
            continue;
        }
        // The same assignment with the candidate's choice withdrawn. A
        // constraint still violated there was already broken before the
        // candidate — the rejection is not attributable to it.
        let mut without_candidate = assignment.clone();
        if without_candidate.get(&rejected.facet) == Some(&rejected.option) {
            without_candidate.remove(&rejected.facet);
        }
        for declaration in hits {
            record_hit(&mut all_violated, declaration, &assignment);
            if !is_violated(declaration, &without_candidate) {
                record_hit(&mut candidate_relevant, declaration, &assignment);
            }
        }
    }

    let named = if !candidate_relevant.is_empty() {
        candidate_relevant
    } else {
        all_violated
    };

    if !named.is_empty() {
        for ((_, id), entry) in named {
            out.push(ConflictingConstraint {
                kind: ConstraintKind::ModelRule,
                facets: entry.facets.into_iter().map(to_facet).collect(),
                summary: entry.condition,
                constraint_id: Some(id),
            });
        }
    } else if saw_model_clause {
        out.push(ConflictingConstraint {
            kind: ConstraintKind::ModelRule,
            facets: unattributed.into_iter().map(to_facet).collect(),
            summary: MODEL_OVER_CONSTRAINED_SUMMARY.to_string(),
            constraint_id: None,
        });
    }

    out
}

fn record_unattributed(facets: &[ConstraintFacet], out: &mut BTreeSet<(String, String)>) {
    for facet in facets {
        out.insert((facet.facet.clone(), facet.option.clone()));
    }
}

/// Fold one violated declaration into the accumulator, naming only the
/// choices the constraint itself talks about. The clause's other atoms are
/// variables the BDD path branched on, not part of what the user violated.
fn record_hit(
    into: &mut BTreeMap<(u32, String), NamedConstraint>,
    declaration: &DeclaredConstraint,
    assignment: &BTreeMap<String, String>,
) {
    let entry = into
        .entry((declaration.root_index, declaration.id.clone()))
        .or_insert_with(|| NamedConstraint {
            condition: declaration.condition.clone(),
            facets: BTreeSet::new(),
        });
    if let Ok(identifiers) = crate::conditions::condition_identifiers(&declaration.condition) {
        for tag in identifiers.tags {
            if let Some(value) = assignment.get(&tag) {
                entry.facets.insert((tag, value.clone()));
            }
        }
    }
}

/// Accumulator for one named constraint while clauses are being folded in.
struct NamedConstraint {
    condition: String,
    facets: BTreeSet<(String, String)>,
}

fn to_facet((facet, option): (String, String)) -> ConstraintFacet {
    ConstraintFacet { facet, option }
}

/// The facet→value assignment a clause forbids, taken from the literals it
/// asserts. `None` when one facet is assigned two distinct values, which no
/// resolvable configuration can be — the signature of an at-most-one
/// cardinality clause.
///
/// Negated literals ("this option was ruled out") are deliberately not
/// folded in: they say what a facet is *not*, which the facet→value
/// evaluator cannot represent, and under the roster's closed-facet
/// cardinality they are implied by the asserted literal for that facet
/// anyway.
fn forbidden_assignment(
    literals: &[(ConstraintFacet, bool)],
) -> Option<BTreeMap<String, String>> {
    let mut assignment: BTreeMap<String, String> = BTreeMap::new();
    for (facet, asserted) in literals {
        if !asserted {
            continue;
        }
        match assignment.get(&facet.facet) {
            Some(existing) if existing != &facet.option => return None,
            _ => {
                assignment.insert(facet.facet.clone(), facet.option.clone());
            }
        }
    }
    Some(assignment)
}

/// The declared constraints `assignment` violates, `root_index` ascending.
/// A constraint is violated iff it evaluates to `False` under the (partial)
/// assignment — `Unknown` is not a violation (ADR-0054 §2).
fn violated_declarations<'a>(
    roster: &'a [DeclaredConstraint],
    assignment: &BTreeMap<String, String>,
) -> Vec<&'a DeclaredConstraint> {
    let mut hits: Vec<&DeclaredConstraint> = roster
        .iter()
        .filter(|declaration| is_violated(declaration, assignment))
        .collect();
    hits.sort_by(|a, b| a.root_index.cmp(&b.root_index).then_with(|| a.id.cmp(&b.id)));
    hits
}

/// Whether `declaration` evaluates to `False` under the (partial)
/// `assignment`. An unparseable roster condition cannot be checked, so it is
/// never reported violated — identity fails closed, not the explanation.
fn is_violated(
    declaration: &DeclaredConstraint,
    assignment: &BTreeMap<String, String>,
) -> bool {
    match parse_condition_expr(&declaration.condition) {
        Ok(expr) => !not_contradicted(&expr, assignment),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "unsat_attribution_tests.rs"]
mod unsat_attribution_tests;
