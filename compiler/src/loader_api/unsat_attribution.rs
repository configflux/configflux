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
// `{id, condition, root_index}` per root conjunct the model can NAME. That is
// every **authored** `constraints:` entry, plus — since ADR-0057 §D4 — the
// conjuncts lowered from a binding's `derive` table and a requirement's
// `accepts` list, whose ids carry the reserved `derive:` / `accepts:` prefixes
// (`crate::lowering`). Nothing here distinguishes them: a lowered conjunct is a
// declared rule with an id and a condition, which is all this function needs,
// and turning its id back into the authoring construct it came from is the
// presentation layer's job. Synthesized cardinality is still absent from the
// roster, for the reason below. A model clause
// is the negation of a forbidden partial assignment, so the assignment it
// forbids can be reconstructed from the clause's signed literals. The
// declared constraints that account for that clause are exactly those the
// assignment **violates** — `eval_partial(condition, assignment) == False`,
// the same rule ADR-0054 §2/§6 use to fail `cfx resolve` closed. Reusing the
// resolve rule is deliberate: `cfx explain` must name the constraint
// `cfx resolve` would reject on, not a differently-derived one.
//
// Reconstructing that assignment reads BOTH halves of the clause: an asserted
// literal pins a facet to one value, and — for a facet the clause mentions
// negatively — the CLOSED declared domain minus what the clause ruled out is
// the set of values it may still take (configflux-pt6v, widened from one value
// to a set by configflux-vfh5). A constraint no value in those sets can satisfy
// is broken by every configuration the clause forbids. See
// [`forbidden_candidates`]; the closed domains arrive as [`ClosedFacetDomains`]
// from the caller, which is why this stays a pure function over plain data.
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

/// The declared value domains of the model's **closed** facets, keyed by
/// facet name (configflux-pt6v).
///
/// A closed facet is one the model synthesizes `exactly_one_of` over
/// (`compiler_core::synthesize_facet_cardinality`): at-least-one AND
/// at-most-one. That at-least-one conjunct is what makes the declared values
/// the FULL range a facet can hold, so a clause that rules some of them out
/// leaves the rest as the values still open to it — which is how
/// [`attribute_core_clauses`] reads a clause before evaluating a declared
/// constraint.
///
/// **Only closed facets belong here.** An open facet is synthesized with
/// at-most-one ONLY, so negating one of its values entails nothing — it may
/// still take a value the model never declared. Keeping open facets out of the
/// map rather than carrying an `open` flag alongside makes that a property of
/// the type: there is no way to hold an open facet's domain here and no way for
/// a caller to forget to check the flag. False attribution — naming a policy
/// the user did not break — is worse than the under-attribution this fixes, so
/// the unrepresentable-state encoding is deliberate.
///
/// The unrepresentable-state encoding is a **Rust-side** guarantee. This type
/// is also wire-representable (ADR-0060 D1), and `Deserialize` is a public
/// constructor from arbitrary JSON: a payload can hand it an open facet's
/// roster and the type cannot object. What defends the wire is ADR-0060 D6 —
/// `runtime_open_with_solver_validation` rejects, fail-closed, any
/// `(facet, value)` pair the bound `.ccm` symbol table cannot account for. That
/// check verifies the pair EXISTS in the model; the symbol table carries no
/// cardinality, so it cannot verify the facet is closed. The residual is
/// bounded by construction: [`attribute_core_clauses`] consults domains only
/// for facets a core clause already mentions, and narrows only to values the
/// roster it was handed actually declares.
///
/// That bound LOOSENED with configflux-vfh5. While a facet completed only where
/// exactly one declared value went un-negated, a truncated or wrongly-open wire
/// roster could mislead only where it collapsed to that value; over a candidate
/// SET, every value the roster omits is one a constraint might have been
/// satisfied by, so a domain narrower than the model's own can report a false
/// disjointness wherever one or more values remain.
///
/// An EMPTY map means "no facet declarations are reachable", and attribution
/// then reads asserted literals only — a clause's negations say nothing without
/// a domain to subtract them from. That is what a caller with no declarations
/// passes, and what an open payload that omits the table yields (ADR-0060 D7) —
/// honest degradation rather than a guess.
///
/// The JSON form is the bare object `{"<facet>": ["<value>", ...]}`
/// (`#[serde(transparent)]`), so the wire carries no `by_facet` wrapper key to
/// keep in step with the field name.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClosedFacetDomains {
    by_facet: BTreeMap<String, BTreeSet<String>>,
}

impl ClosedFacetDomains {
    /// Record one closed facet's declared values. Declaration ORDER is not
    /// carried: entailment is a set operation, and the committed goldens must
    /// not depend on which value happens to sort first (configflux-gpwf).
    pub(crate) fn insert(&mut self, facet: String, values: BTreeSet<String>) {
        self.by_facet.insert(facet, values);
    }

    /// Whether any closed facet is recorded. Read by `skip_serializing_if` on
    /// [`crate::loader_api::ResolveResult`] so a facet-free model's envelope
    /// stays byte-identical (ADR-0060 D8.1), and by the runtime's open-time
    /// validation to skip the symbol-table walk entirely.
    pub fn is_empty(&self) -> bool {
        self.by_facet.is_empty()
    }

    /// Read-only walk over `(facet, declared values)`, facet-ascending. The
    /// runtime's ADR-0060 D6 check needs every recorded pair to test it against
    /// the bound `.ccm` symbol table, and it lives in the `runtime` crate
    /// because the compiler may not import `solver` (ADR-0003 §2). Reading
    /// cannot construct an invalid table, so this does not weaken the
    /// unrepresentable-state encoding above; only `insert` and `Deserialize`
    /// can, and D6 is what screens the latter.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &BTreeSet<String>)> {
        self.by_facet
            .iter()
            .map(|(facet, values)| (facet.as_str(), values))
    }

    /// The facet's declared values, or `None` when it is open or undeclared —
    /// in both cases nothing may be entailed about it.
    fn declared_values(&self, facet: &str) -> Option<&BTreeSet<String>> {
        self.by_facet.get(facet)
    }
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
/// **Closed-facet domains.** `domains` carries the model's closed facet
/// declarations, letting a clause that mentions a facet only negatively still
/// narrow it to the values the model leaves standing (see
/// [`forbidden_candidates`]). An EMPTY map reads asserted literals only, and is
/// what a caller with no access to the declarations passes.
///
/// A roster entry whose `condition` does not parse is skipped rather than
/// guessed at: naming a constraint the evaluator could not check would be
/// exactly the "paper over it with the nearest constraint" failure §5.4
/// rules out.
pub fn attribute_core_clauses(
    clauses: &[CoreClause],
    rejected: &ConstraintFacet,
    roster: &[DeclaredConstraint],
    domains: &ClosedFacetDomains,
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
        let Some(open) = forbidden_candidates(&clause.forbidden, domains) else {
            record_unattributed(&clause.facets, &mut unattributed);
            continue;
        };
        let hits = violated_declarations(roster, &open);
        if hits.is_empty() {
            record_unattributed(&clause.facets, &mut unattributed);
            continue;
        }
        // The same facts with the candidate's choice withdrawn. A constraint
        // still violated there was already broken before the candidate — the
        // rejection is not attributable to it.
        let mut without_candidate = open.clone();
        if pinned_to(&open, &rejected.facet) == Some(rejected.option.as_str()) {
            without_candidate.remove(&rejected.facet);
        }
        // Only the facets the clause pins to a single value can be NAMED as
        // choices; "one of c2 or c3" is not a `(facet, option)` pair and there
        // is nowhere honest to put it.
        let assignment = pinned_assignment(&open);
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

/// The values a clause leaves open to each facet it mentions. `None` when one
/// facet is asserted at two distinct values, which no resolvable configuration
/// can be — the signature of an at-most-one cardinality clause.
///
/// # Both halves of a clause are facts
///
/// An asserted literal ("the forbidden assignment took this option") pins the
/// facet to that one value. A NEGATED literal says what a facet is *not*, and
/// that is a fact about the clause too: read together with the CLOSED declared
/// domain it leaves the facet a smaller set, and a constraint no value in that
/// set can satisfy is genuinely broken by every configuration the clause
/// forbids. Naming it is a deduction, not a preference, which is why this
/// preserves ADR-0054 §2 rather than weakening it.
///
/// Carrying sets rather than single values is configflux-vfh5. The previous
/// form kept only what collapsed to ONE value — an asserted literal, or a
/// closed facet with every declared value but one negated (configflux-pt6v) —
/// and a facet with two values left was dropped entirely. A facet-to-facet
/// equality (ADR-0057 §D5) contributes exactly the clauses that shape cannot
/// finish: it pins one side and rules a SINGLE value out of the other, so on a
/// three-entry catalogue two values remained, the facet went unassigned, the
/// equality evaluated `Unknown`, and the one rule that made the selection
/// impossible went unnamed. As a set the two sides are plainly disjoint.
///
/// Three shapes deliberately say NOTHING about a facet, because a false
/// attribution — naming a policy the user may not have broken — is worse than
/// the under-attribution this fixes:
///
/// * **An open or undeclared facet** — absent from `domains` by construction.
///   An open facet is synthesized with at-most-one only, so ruling a value out
///   leaves it free to hold one the model never declared, and there is no set
///   to reason over.
/// * **Every declared value negated** — a synthesized at-least-one clause. It
///   is contradictory on its own, and §5.4 requires it stay unattributed, so
///   the empty set is omitted rather than read as "satisfies nothing".
/// * **A facet the clause never mentions** — nothing was said, so nothing
///   follows; restricting it would attribute a policy to a clause that never
///   referenced it.
fn forbidden_candidates(
    literals: &[(ConstraintFacet, bool)],
    domains: &ClosedFacetDomains,
) -> Option<BTreeMap<String, BTreeSet<String>>> {
    let mut open: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut negated: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (facet, asserted) in literals {
        if !asserted {
            negated
                .entry(facet.facet.as_str())
                .or_default()
                .insert(facet.option.as_str());
            continue;
        }
        match pinned_to(&open, &facet.facet) {
            Some(existing) if existing != facet.option => return None,
            _ => {
                open.insert(
                    facet.facet.clone(),
                    BTreeSet::from([facet.option.clone()]),
                );
            }
        }
    }

    for (facet, ruled_out) in negated {
        if open.contains_key(facet) {
            // Already pinned by an asserted literal; the negations are implied.
            continue;
        }
        let Some(declared) = domains.declared_values(facet) else {
            continue;
        };
        let remaining: BTreeSet<String> = declared
            .iter()
            .filter(|value| !ruled_out.contains(value.as_str()))
            .cloned()
            .collect();
        if remaining.is_empty() {
            continue;
        }
        open.insert(facet.to_string(), remaining);
    }

    Some(open)
}

/// The one value `facet` is pinned to, or `None` when the clause left it a
/// choice or said nothing about it.
fn pinned_to<'a>(
    open: &'a BTreeMap<String, BTreeSet<String>>,
    facet: &str,
) -> Option<&'a str> {
    let values = open.get(facet)?;
    match values.len() {
        1 => values.iter().next().map(String::as_str),
        _ => None,
    }
}

/// The facet→value assignment of everything the clause pinned. This is what a
/// named entry may report as the choices behind a violation: a facet left a
/// choice of values names no `(facet, option)` pair.
fn pinned_assignment(open: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, String> {
    open.iter()
        .filter(|(_, values)| values.len() == 1)
        .filter_map(|(facet, values)| {
            values
                .iter()
                .next()
                .map(|value| (facet.clone(), value.clone()))
        })
        .collect()
}

/// The declared constraints the clause violates, `root_index` ascending.
/// A constraint is violated iff it evaluates to `False` under the values the
/// clause leaves open — `Unknown` is not a violation (ADR-0054 §2).
fn violated_declarations<'a>(
    roster: &'a [DeclaredConstraint],
    open: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<&'a DeclaredConstraint> {
    let mut hits: Vec<&DeclaredConstraint> = roster
        .iter()
        .filter(|declaration| is_violated(declaration, open))
        .collect();
    hits.sort_by(|a, b| a.root_index.cmp(&b.root_index).then_with(|| a.id.cmp(&b.id)));
    hits
}

/// Whether `declaration` evaluates to `False` under the values the clause
/// leaves open. An unparseable roster condition cannot be checked, so it is
/// never reported violated — identity fails closed, not the explanation.
fn is_violated(
    declaration: &DeclaredConstraint,
    open: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    match parse_condition_expr(&declaration.condition) {
        Ok(expr) => is_contradicted(&expr, &FacetWorld::Restricted(open)),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "unsat_attribution_tests.rs"]
mod unsat_attribution_tests;
