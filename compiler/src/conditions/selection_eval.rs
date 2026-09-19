// SPDX-License-Identifier: BUSL-1.1

//! Typed-AST helpers for the loader_api selection-constraint heuristics.
//!
//! configflux-ccs.7 migrates the option-validity / facet-domain resolution in
//! `compiler/src/loader_api/shared_ops.rs` off the string-scanning
//! `parse_conjunction_atoms` / `scan_condition_predicates` path and onto the
//! typed [`ConditionExpr`] produced by [`super::parse_condition_expr`]. These
//! helpers are the typed evaluator that replaces the old
//! string-scanning `SelectionConstraintModel` walks.
//!
//! Behaviour is intentionally a faithful re-expression of the legacy
//! heuristics so the resolver stays byte-stable (ADR-0005). In particular the
//! legacy `option_is_valid` only ever consulted *pure conjunction* conditions
//! (those `parse_condition_conjunction` accepted as a `&&`-chain of `==`/`!=`
//! atoms); disjunctions, negations, parentheses, and the grammar-v2
//! cardinality operators contributed facet domains but never formed a
//! compatibility group. [`is_pure_conjunction`] reproduces that exact
//! eligibility test over the AST, and [`not_contradicted`] reproduces the
//! old `group_compatible` partial evaluation (an unassigned tag is skipped;
//! the condition is contradicted only when an *assigned* atom is false).
//!
//! configflux-vfh5 widened WHAT a caller may know without changing that rule.
//! A caller reading an unsat core knows a facet's still-possible VALUES rather
//! than its one value, so the evaluator takes a [`FacetWorld`] and the two
//! shapes share one walk. The single-value shape behaves exactly as it always
//! did, which is what keeps every selection and resolve verdict byte-stable.

use std::collections::{BTreeMap, BTreeSet};

use super::{ConditionExpr, ConditionPredicate, ConditionPredicateOp};

/// What a partial evaluation knows about the facets it may be asked about.
///
/// Two shapes, because two callers hold two different kinds of fact and both
/// evaluate the SAME grammar. Splitting the walk in two instead would put the
/// exhaustive-match discipline this module depends on in two places, free to
/// drift; splitting only the atom lookup keeps one walk (configflux-vfh5).
///
/// [`Bound`](Self::Bound) is the original: every fact is "this facet is this
/// value". [`Restricted`](Self::Restricted) is the strictly more general one:
/// every fact is "this facet is one of these values". A `Bound` fact IS a
/// `Restricted` one whose set is a singleton, and the two agree on every such
/// input — which is what lets [`not_contradicted`] keep its exact behavior for
/// the selection and resolve callers while attribution reads the wider form.
///
/// In BOTH shapes an unnamed facet is `Unknown`, never `False`. Nothing may be
/// concluded about a facet nobody said anything about.
pub(crate) enum FacetWorld<'a> {
    /// Each named facet is bound to exactly one value.
    Bound(&'a BTreeMap<String, String>),
    /// Each named facet is restricted to the set of values still open to it.
    ///
    /// An EMPTY set is read as `Unknown`, not as "no value satisfies anything".
    /// It is what an ADR-0054 §5.4 synthesized at-least-one clause produces —
    /// every declared value ruled out — and that clause is contradictory on its
    /// own, so it entails nothing about any policy. Reading it as `False`
    /// instead would let the model's own bookkeeping be reported as a rule the
    /// author wrote, which §5.4 forbids outright.
    Restricted(&'a BTreeMap<String, BTreeSet<String>>),
}

impl FacetWorld<'_> {
    /// Whether the facet certainly is, certainly is not, or may or may not be
    /// `value`.
    fn holds(&self, facet: &str, value: &str) -> Ternary {
        match self {
            FacetWorld::Bound(bound) => match bound.get(facet) {
                Some(bound) => Ternary::from_bool(bound == value),
                None => Ternary::Unknown,
            },
            FacetWorld::Restricted(open) => match open.get(facet) {
                Some(open) if open.is_empty() => Ternary::Unknown,
                Some(open) if !open.contains(value) => Ternary::False,
                Some(open) if open.len() == 1 => Ternary::True,
                _ => Ternary::Unknown,
            },
        }
    }

    /// Whether the two facets certainly hold the same value, certainly do not,
    /// or may or may not (ADR-0057 §D5).
    ///
    /// Under [`Restricted`](Self::Restricted) the decisive fact is DISJOINTNESS:
    /// two sets sharing no value cannot be equal under any completion, whatever
    /// their sizes. That is the case this variant exists for — a facet
    /// comparison contributes core clauses that pin one side and rule a single
    /// value out of the other, which no single-value reading can finish.
    fn agree(&self, left: &str, right: &str) -> Ternary {
        match self {
            FacetWorld::Bound(bound) => match (bound.get(left), bound.get(right)) {
                (Some(left), Some(right)) => Ternary::from_bool(left == right),
                _ => Ternary::Unknown,
            },
            FacetWorld::Restricted(open) => match (open.get(left), open.get(right)) {
                (Some(left), Some(right)) if !left.is_empty() && !right.is_empty() => {
                    if left.is_disjoint(right) {
                        Ternary::False
                    } else if left.len() == 1 && left == right {
                        Ternary::True
                    } else {
                        Ternary::Unknown
                    }
                }
                _ => Ternary::Unknown,
            },
        }
    }
}

/// Three-valued result of partially evaluating a [`ConditionExpr`] against an
/// incomplete [`FacetWorld`]. `Unknown` means the world does not decide the
/// expression — a referenced tag it says nothing about, or one it leaves a
/// choice of values — mirroring the legacy "skip the predicate" behaviour
/// rather than the strict evaluator's missing-tag error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ternary {
    True,
    False,
    Unknown,
}

impl Ternary {
    fn from_bool(value: bool) -> Self {
        if value {
            Ternary::True
        } else {
            Ternary::False
        }
    }

    fn not(self) -> Self {
        match self {
            Ternary::True => Ternary::False,
            Ternary::False => Ternary::True,
            Ternary::Unknown => Ternary::Unknown,
        }
    }

    /// Kleene AND: `False` is absorbing, then `Unknown`, else `True`.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Ternary::False, _) | (_, Ternary::False) => Ternary::False,
            (Ternary::Unknown, _) | (_, Ternary::Unknown) => Ternary::Unknown,
            _ => Ternary::True,
        }
    }

    /// Kleene OR: `True` is absorbing, then `Unknown`, else `False`.
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Ternary::True, _) | (_, Ternary::True) => Ternary::True,
            (Ternary::Unknown, _) | (_, Ternary::Unknown) => Ternary::Unknown,
            _ => Ternary::False,
        }
    }
}

/// Partially evaluate `expr` against `world`, skipping any predicate whose tag
/// the world says nothing about (`Unknown`). This is the typed-AST replacement
/// for the legacy `group_compatible` walk and generalises it to every grammar-v2
/// node, so non-conjunction conditions evaluate sensibly even though the
/// legacy code never grouped them.
///
/// Kleene's strong three-valued semantics, which is what makes the `False` this
/// returns trustworthy: a formula is `False` only when it is false under EVERY
/// completion of the facts the world holds. Correlated atoms (`a == 'x'` and
/// `a == 'y'` cannot both be true) only make more formulas determinate than
/// this detects; they never make a determinate answer wrong. Under-determination
/// is safe here because the sole caller acts on `False` alone.
fn eval_partial(expr: &ConditionExpr, world: &FacetWorld<'_>) -> Ternary {
    match expr {
        ConditionExpr::Bool(value) => Ternary::from_bool(*value),
        ConditionExpr::Predicate(predicate) => eval_predicate(predicate, world),
        ConditionExpr::Not(inner) => eval_partial(inner, world).not(),
        ConditionExpr::And(lhs, rhs) => eval_partial(lhs, world).and(eval_partial(rhs, world)),
        ConditionExpr::Or(lhs, rhs) => eval_partial(lhs, world).or(eval_partial(rhs, world)),
        ConditionExpr::AnyOf(children) => children
            .iter()
            .map(|child| eval_partial(child, world))
            .fold(Ternary::False, Ternary::or),
        ConditionExpr::AllOf(children) => children
            .iter()
            .map(|child| eval_partial(child, world))
            .fold(Ternary::True, Ternary::and),
        ConditionExpr::ExactlyOneOf(children) => eval_exactly_one_of(children, world),
        // configflux-secb.2 / ADR-0057 §D5. `Unknown` while either side may
        // still take a value the other can match, because "Unknown is not a
        // violation" (ADR-0054 §2) — a selection that has bound one of the two
        // facets has not yet violated an agreement rule about both.
        ConditionExpr::FacetCompare { left, op, right } => {
            let agree = world.agree(left, right);
            match op {
                ConditionPredicateOp::Eq => agree,
                ConditionPredicateOp::NotEq => agree.not(),
            }
        }
    }
}

fn eval_predicate(predicate: &ConditionPredicate, world: &FacetWorld<'_>) -> Ternary {
    let holds = world.holds(&predicate.tag, &predicate.value);
    match predicate.op {
        ConditionPredicateOp::Eq => holds,
        ConditionPredicateOp::NotEq => holds.not(),
    }
}

/// `exactly_one_of` is "at-least-one OR-reduction" conjoined with pairwise
/// "at-most-one" (ADR-0006 §4). Under partial information we keep the same
/// shape so an unassigned child yields `Unknown` rather than a decision.
fn eval_exactly_one_of(children: &[ConditionExpr], world: &FacetWorld<'_>) -> Ternary {
    let evaluated: Vec<Ternary> = children
        .iter()
        .map(|child| eval_partial(child, world))
        .collect();

    let at_least_one = evaluated.iter().copied().fold(Ternary::False, Ternary::or);

    let mut at_most_one = Ternary::True;
    for i in 0..evaluated.len() {
        for j in (i + 1)..evaluated.len() {
            // `!(a && b)` for each pair — both true violates at-most-one.
            at_most_one = at_most_one.and(evaluated[i].and(evaluated[j]).not());
        }
    }

    at_least_one.and(at_most_one)
}

/// Returns `true` when `expr` is not contradicted by `assignments`: the typed
/// replacement for the legacy `group_compatible`. A condition that evaluates
/// to `False` under the (partial) assignment is contradicted; `True` and
/// `Unknown` are both compatible, exactly as the old walk skipped unassigned
/// tags and accepted any group with no failing assigned atom.
///
/// A thin adapter over [`is_contradicted`] since configflux-vfh5. A bound value
/// is a one-element restriction, and [`FacetWorld`]'s two shapes agree on every
/// such input, so the option-validity and resolve callers see byte-identical
/// verdicts to the ones they saw before the generalisation.
pub(crate) fn not_contradicted(
    expr: &ConditionExpr,
    assignments: &BTreeMap<String, String>,
) -> bool {
    !is_contradicted(expr, &FacetWorld::Bound(assignments))
}

/// Returns `true` when `expr` is contradicted by `world` — false under every
/// completion of the facts it holds.
///
/// The seam ADR-0054 §5.4 attribution reads a core clause through
/// (`loader_api::unsat_attribution`): a clause forbids a partial assignment,
/// and the declared constraints that ACCOUNT for it are the ones that
/// assignment contradicts. Exposed alongside [`not_contradicted`] rather than
/// folded into it because the two callers hold different facts, not different
/// rules — this is the same evaluator, asked the same question.
pub(crate) fn is_contradicted(expr: &ConditionExpr, world: &FacetWorld<'_>) -> bool {
    eval_partial(expr, world) == Ternary::False
}

/// Returns `true` when `expr` is a pure conjunction of `==`/`!=` predicates
/// (`p1 && p2 && …`), the typed equivalent of the legacy
/// `parse_condition_conjunction` acceptance test. The parser lowers `&&`
/// left-associatively (`And(And(p1, p2), p3)`), so we recurse through `And`
/// nodes and require every leaf to be a `Predicate`.
///
/// The match is deliberately EXHAUSTIVE rather than falling through a `_`
/// arm: this predicate decides whether a selector condition forms an
/// option-validity compatibility group, and a new grammar node classified by
/// accident would change that silently, with no compile error and no failing
/// test (configflux-secb.2). A facet-to-facet comparison is not a
/// `(facet, option)` atom — it constrains a *pair* of facets — so it is not a
/// group, exactly like the disjunctive and cardinality forms.
pub(crate) fn is_pure_conjunction(expr: &ConditionExpr) -> bool {
    match expr {
        ConditionExpr::Predicate(_) => true,
        ConditionExpr::And(lhs, rhs) => is_pure_conjunction(lhs) && is_pure_conjunction(rhs),
        ConditionExpr::Bool(_)
        | ConditionExpr::Not(_)
        | ConditionExpr::Or(_, _)
        | ConditionExpr::AnyOf(_)
        | ConditionExpr::AllOf(_)
        | ConditionExpr::ExactlyOneOf(_)
        | ConditionExpr::FacetCompare { .. } => false,
    }
}

/// Returns `true` when `expr` contains an equality predicate `tag == value`.
/// Reproduces the legacy "group mentions this facet option" membership test
/// (`predicate.op == Eq && predicate.tag == facet && predicate.value ==
/// option`) directly over the AST.
pub(crate) fn mentions_eq(expr: &ConditionExpr, tag: &str, value: &str) -> bool {
    let mut found = false;
    visit_predicates(expr, &mut |predicate| {
        if predicate.op == ConditionPredicateOp::Eq
            && predicate.tag == tag
            && predicate.value == value
        {
            found = true;
        }
    });
    found
}

/// Invoke `sink` for every equality predicate (`tag == value`) in `expr`, in
/// left-to-right source order. The typed replacement for feeding
/// `scan_condition_predicates` output into `register_facet_domains`: the old
/// code only widened a facet domain for `Eq` atoms, so we expose only those.
pub(crate) fn for_each_eq_predicate<F: FnMut(&str, &str)>(expr: &ConditionExpr, mut sink: F) {
    visit_predicates(expr, &mut |predicate| {
        if predicate.op == ConditionPredicateOp::Eq {
            sink(&predicate.tag, &predicate.value);
        }
    });
}

/// Visit every predicate atom in `expr` — **both** `==` and `!=` — in
/// left-to-right DFS pre-order, reporting each as its `(tag, value)` pair.
///
/// This is the *symbol-universe* walk, and it is deliberately broader than
/// [`for_each_eq_predicate`] (the domain-widening walk, which is `Eq`-only).
/// A `(tag, value)` pair names the same `.ccm` variable under either operator:
/// `ccm_emitter::compile_predicate` lowers `Eq` to `var` and `NotEq` to
/// `not(var)` over that one variable (ADR-0054 §5.2). So the set of pairs
/// reported here is exactly the set of symbols the `var_order` traversal
/// collects from the same expression.
///
/// configflux-9xxq / ADR-0054 §5.1 uses this to land a branch selector's
/// symbols without asserting the selector on the BDD root.
pub(crate) fn for_each_predicate_symbol<F: FnMut(&str, &str)>(expr: &ConditionExpr, mut sink: F) {
    visit_predicates(expr, &mut |predicate| {
        sink(&predicate.tag, &predicate.value);
    });
}

fn visit_predicates<F: FnMut(&ConditionPredicate)>(expr: &ConditionExpr, sink: &mut F) {
    match expr {
        ConditionExpr::Bool(_) => {}
        ConditionExpr::Predicate(predicate) => sink(predicate),
        ConditionExpr::Not(inner) => visit_predicates(inner, sink),
        ConditionExpr::And(lhs, rhs) | ConditionExpr::Or(lhs, rhs) => {
            visit_predicates(lhs, sink);
            visit_predicates(rhs, sink);
        }
        ConditionExpr::AnyOf(children)
        | ConditionExpr::AllOf(children)
        | ConditionExpr::ExactlyOneOf(children) => {
            for child in children {
                visit_predicates(child, sink);
            }
        }
        // A facet-to-facet comparison names no VALUE, so it contributes no
        // `(tag, value)` symbol here (configflux-secb.2 / ADR-0057 §D5). Its
        // symbols are the declared values of both operands, which the
        // declared-facet symbol pass already contributes in full
        // (`compiler_core::synthesize_facet_clauses`, ADR-0047 §4 Amendment 1),
        // and which the lowering re-derives when it expands the node. Reporting
        // the right-hand FACET name here as if it were a value would be a lie
        // the closed-domain check in `link_verify` would then act on.
        ConditionExpr::FacetCompare { .. } => {}
    }
}

#[cfg(test)]
mod tests;
