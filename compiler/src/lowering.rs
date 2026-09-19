// SPDX-License-Identifier: BUSL-1.1

//! Lowering `derive` tables and `accepts` lists into root conjuncts
//! (ADR-0057 §D3/§D4).
//!
//! # Why this is a lowering and not a feature
//!
//! ADR-0054 §5.1 gives the constraint model exactly one channel for authored
//! policy: the `constraints` namespace, folded into the BDD root and rostered
//! in `ccm.manifest.json` so `cfx explain` can name what a rejection broke.
//! ADR-0057 adds two authoring conveniences that are *policy in disguise* —
//! `derive: {site: {factory_a: c1}}` is the rule `site != 'factory_a' ||
//! line_container == 'c1'`, and `accepts: [c1, c2]` is `any_of(line_container
//! == 'c1', line_container == 'c2')`.
//!
//! Both become ordinary root conjuncts here, with an attribution id, and then
//! travel the EXISTING path: the emitter folds and rosters them
//! (`ccm_emitter::parse_condition_model`), `cfx options` prunes against them
//! (`shared_ops::option_is_valid`), `cfx resolve` fails closed on them
//! (`resolve_ops::evaluate_constraints`), and `cfx explain` names them
//! (`unsat_attribution::attribute_core_clauses`). Nothing downstream learns
//! what a binding or a requirement is.
//!
//! # Attribution ids
//!
//! `derive:<binding>:<source>=<source_value>` and `accepts:<component>.<slot>`.
//! Both prefixes are **reserved**, and cannot collide with an authored
//! constraint id: that id is `#snakeId` (`^[a-z]([a-z0-9]|_[a-z0-9])*_?$`) and
//! so can never contain `:`. The collision argument is a property of the
//! authoring grammar, not a convention this module maintains.
//!
//! # Order
//!
//! ADR-0057 §D4 fixes the fold order, and it is load-bearing: a conjunct's
//! `root_index` in the §5.4 roster is its position in the folded list, so the
//! order must be a function of the model and nothing else. Authored constraints
//! come first (the caller supplies those), then the `derive` conjuncts
//! (bindings, sources and source values each id-ascending), then the `accepts`
//! conjuncts (components then slots id-ascending). Within one `accepts`
//! conjunct the disjuncts keep the AUTHORED entry order — what the author
//! wrote is what the diagnostics echo back.
//!
//! # The condition guard
//!
//! A component's `condition` is an inclusion selector and asserts nothing
//! (ADR-0054 §3), so an `accepts` conjunct on a conditional component is
//! emitted as `!(<condition>) || <accepts>`. Dropping the guard would let an
//! excluded component veto entries for everyone else, which is the exact
//! conflation ADR-0054 removed from the model.

use crate::compiler_core::{facet_eq_predicate, facet_ne_predicate};
use crate::interface_summary::{BindingLink, RequirementLink};
use crate::schema::{Binding, Component};
use std::collections::BTreeMap;

/// The reserved attribution prefix for a conjunct lowered from a binding's
/// `derive` table.
pub const DERIVE_ATTRIBUTION_PREFIX: &str = "derive:";

/// The reserved attribution prefix for a conjunct lowered from a requirement's
/// `accepts` list.
pub const ACCEPTS_ATTRIBUTION_PREFIX: &str = "accepts:";

/// One lowered root conjunct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredConjunct {
    /// The attribution id — `derive:<binding>:<source>=<value>` or
    /// `accepts:<component>.<slot>`. This is what `cfx explain` reports as the
    /// constraint id and what `entity_path` carries on a resolve rejection.
    pub id: String,
    /// The conjunct, in the existing condition grammar.
    pub condition: String,
    /// The authored entity the conjunct came from: the BINDING id for a
    /// `derive:` conjunct, the COMPONENT id for an `accepts:` one. Callers that
    /// know which chunk declared that entity use it to attribute the conjunct
    /// to a `source_id`; this module deliberately knows nothing about chunks.
    pub origin: String,
}

/// The attribution id of one `(source_value → entry)` pair of a binding's
/// `derive` table.
pub fn derive_attribution_id(binding: &str, source: &str, source_value: &str) -> String {
    format!("{DERIVE_ATTRIBUTION_PREFIX}{binding}:{source}={source_value}")
}

/// The attribution id of one requirement's `accepts` list.
pub fn accepts_attribution_id(component: &str, slot: &str) -> String {
    format!("{ACCEPTS_ATTRIBUTION_PREFIX}{component}.{slot}")
}

/// Every root conjunct ADR-0057 §D4 lowers, in the fold order the module docs
/// fix: `derive` conjuncts first, then `accepts` conjuncts.
///
/// Total by construction — it never fails and never inspects a catalogue. A
/// `derive` key outside its source's domain, an `accepts` entry outside the
/// binding's catalogue, and a requirement naming no declared binding are all
/// rejected earlier, at link time (`link_verify::validate_binding_links` and
/// `link_verify::validate_requirements`). This runs on models that already
/// passed those checks, so it has nothing to decide.
///
/// Iterator-shaped rather than `&BTreeMap` because the three callers hold their
/// namespaces in different maps: the compiler's `Config` uses `HashMap`, the
/// two loader paths gather `BTreeMap`s across chunks. Ordering is imposed here
/// rather than trusted from the caller, so a `HashMap`'s seed cannot reach the
/// emitted bytes.
pub fn lowered_root_conjuncts<'a, B, C>(bindings: B, components: C) -> Vec<LoweredConjunct>
where
    B: IntoIterator<Item = (&'a String, &'a Binding)>,
    C: IntoIterator<Item = (&'a String, &'a Component)>,
{
    let mut out = derive_conjuncts(bindings);
    out.extend(accepts_conjuncts(components));
    out
}

/// Every root conjunct ADR-0057 §D4 lowers, read off the HEADER shapes a
/// linker holds instead of the authored ones (ADR-0058 §D4 stage 2).
///
/// The same rule, the same fold order, and — this is the point — the same
/// conjunct TEXT and the same attribution ids, because both entry points build
/// them with [`derive_attribution_id`], [`accepts_attribution_id`],
/// [`accepts_disjunction`] and [`guarded`]. Only the walk differs, because only
/// the input shape differs: [`BindingLink`] carries a binding's single `derive`
/// source and its `(source_value, entry)` pairs, and [`RequirementLink`] carries
/// one requirement's slot, binding, `accepts` list and guarding condition. A
/// unit test pins the two against each other on one model.
///
/// `derive_source_count > 1` is not handled here and does not need to be: v1
/// admits exactly one source, `link_verify::validate_binding_links` rejects
/// more with `E_BINDING_INVALID`, and this runs only on models that passed it.
pub fn lowered_root_conjuncts_from_summary(
    binding_links: &BTreeMap<String, BindingLink>,
    requirements: &[RequirementLink],
) -> Vec<LoweredConjunct> {
    let mut out = Vec::new();
    // `binding_links` is a `BTreeMap`, so this is binding-id ascending; the
    // pairs inside are source-value ascending (`interface_summary` builds them
    // from the authored `BTreeMap`).
    for (id, link) in binding_links {
        let Some(source) = &link.derive_source else {
            continue;
        };
        for (source_value, entry) in &link.derive_pairs {
            out.push(LoweredConjunct {
                id: derive_attribution_id(id, source, source_value),
                condition: format!(
                    "{} || {}",
                    facet_ne_predicate(source, source_value),
                    facet_eq_predicate(id, entry)
                ),
                origin: id.to_string(),
            });
        }
    }
    // Requirements arrive component-then-slot ascending — a property the merge
    // imposes (ADR-0057 §D4), not one this function may assume of any caller.
    for requirement in requirements {
        let Some(accepts) = &requirement.accepts else {
            continue;
        };
        if accepts.is_empty() {
            continue;
        }
        out.push(LoweredConjunct {
            id: accepts_attribution_id(&requirement.component, &requirement.slot),
            condition: guarded(
                requirement.condition.as_deref(),
                &accepts_disjunction(&requirement.binding, accepts),
            ),
            origin: requirement.component.clone(),
        });
    }
    out
}

/// The `derive` half of [`lowered_root_conjuncts`].
///
/// Each `(source_value → entry)` pair becomes `src != '<value>' || <binding> ==
/// '<entry>'`: an implication, so a source value the table does not cover
/// implies nothing (ADR-0057 §D3 — "a partial table is allowed"). Written as a
/// disjunction rather than an implication operator because the grammar has no
/// implication operator; this is the same shape an author would write by hand.
pub fn derive_conjuncts<'a, B>(bindings: B) -> Vec<LoweredConjunct>
where
    B: IntoIterator<Item = (&'a String, &'a Binding)>,
{
    let ordered: BTreeMap<&str, &Binding> = bindings
        .into_iter()
        .map(|(id, binding)| (id.as_str(), binding))
        .collect();

    let mut out = Vec::new();
    for (id, binding) in ordered {
        let Some(table) = &binding.derive else {
            continue;
        };
        // v1 admits exactly one source (`validate_binding_links` rejects more),
        // but the walk stays total over the authored table so this function
        // never has to decide which source is "the" one.
        for (source, pairs) in table {
            for (source_value, entry) in pairs {
                out.push(LoweredConjunct {
                    id: derive_attribution_id(id, source, source_value),
                    condition: format!(
                        "{} || {}",
                        facet_ne_predicate(source, source_value),
                        facet_eq_predicate(id, entry)
                    ),
                    origin: id.to_string(),
                });
            }
        }
    }
    out
}

/// The `accepts` half of [`lowered_root_conjuncts`].
///
/// A requirement without `accepts` contributes nothing: it accepts every entry,
/// which is the tautology the model already holds, and emitting it would add a
/// root conjunct — and a roster entry `cfx explain` could name — for a
/// component that ruled nothing out.
pub fn accepts_conjuncts<'a, C>(components: C) -> Vec<LoweredConjunct>
where
    C: IntoIterator<Item = (&'a String, &'a Component)>,
{
    let ordered: BTreeMap<&str, &Component> = components
        .into_iter()
        .map(|(id, component)| (id.as_str(), component))
        .collect();

    let mut out = Vec::new();
    for (id, component) in ordered {
        for (slot, requirement) in &component.requires {
            let Some(accepts) = &requirement.accepts else {
                continue;
            };
            if accepts.is_empty() {
                // `validate_requirements` rejects this; skipping keeps the
                // function total for callers that lower before validating
                // (none today), and an empty `any_of` would not parse.
                continue;
            }
            out.push(LoweredConjunct {
                id: accepts_attribution_id(id, slot),
                condition: guarded(
                    component.condition.as_deref(),
                    &accepts_disjunction(&requirement.binding, accepts),
                ),
                origin: id.to_string(),
            });
        }
    }
    out
}

/// `any_of(b == 'e1', …, b == 'eN')`, or the bare predicate when the list names
/// exactly one entry.
///
/// The single-entry case is not a style choice: the grammar's cardinality
/// operators require at least two arguments (ADR-0006 §3,
/// `AstParser::parse_cardinality_args`), and `any_of` over one disjunct means
/// precisely that disjunct. Emitting the bare predicate is the same proposition
/// the general form denotes, and it is the only form that parses.
fn accepts_disjunction(binding: &str, accepts: &[String]) -> String {
    let disjuncts: Vec<String> = accepts
        .iter()
        .map(|entry| facet_eq_predicate(binding, entry))
        .collect();
    match disjuncts.len() {
        1 => disjuncts.into_iter().next().unwrap_or_default(),
        _ => format!("any_of({})", disjuncts.join(", ")),
    }
}

/// Guard a conjunct with the declaring component's inclusion `condition`
/// (ADR-0054 §3). An absent or blank condition means the component is always
/// included, so the conjunct stands unguarded.
///
/// The condition is parenthesised before negation so an authored `a == 'x' ||
/// b == 'y'` negates as a whole rather than binding `!` to its first operand.
fn guarded(condition: Option<&str>, conjunct: &str) -> String {
    match condition.map(str::trim) {
        Some(condition) if !condition.is_empty() => format!("!({condition}) || {conjunct}"),
        _ => conjunct.to_string(),
    }
}

// ----------------------------------------------------------------------------
// Reading a lowered conjunct back
// ----------------------------------------------------------------------------

/// What a lowered attribution id and its condition text say, recovered for a
/// presentation layer.
///
/// `cfx explain` must render a lowered conjunct as the AUTHORING construct it
/// came from — "blocked by binding line_container, derived from site" — not as
/// the machine implication the model actually asserts. The id alone cannot say
/// it: a `derive:` id names the source value but not the entry it forces, and
/// an `accepts:` id names neither the binding nor the entries. The rest lives
/// in the condition text.
///
/// Recovering it HERE, beside the code that wrote the text, is the point. The
/// generator and the reader are one file apart, so a change to the emitted
/// shape that broke the reading is a change to two adjacent functions and is
/// caught by this module's own tests. A reader living in the renderer would be
/// a second, remote copy of the emitted grammar, free to drift — which is
/// exactly the failure ADR-0031 D5's layering rule is about. Presentation stays
/// with the renderers: this returns FACTS, never a rendered line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attribution {
    /// One `(source_value → entry)` pair of a binding's `derive` table.
    Derive {
        binding: String,
        source: String,
        source_value: String,
        entry: String,
    },
    /// One requirement's `accepts` list, entries in authored order.
    Accepts {
        component: String,
        slot: String,
        entries: Vec<String>,
    },
}

/// Read a lowered conjunct back into the authoring construct it came from.
///
/// `None` for an authored constraint id (neither reserved prefix), and for a
/// conjunct whose text does not have the shape this module emits. A caller that
/// gets `None` reports the conjunct generically, which is honest degradation:
/// every fact in the returned value comes out of the inputs, so a shape this
/// cannot read yields nothing rather than a guess.
pub fn describe_attribution(id: &str, condition: &str) -> Option<Attribution> {
    if let Some(rest) = id.strip_prefix(DERIVE_ATTRIBUTION_PREFIX) {
        let (binding, tail) = rest.split_once(':')?;
        let (source, source_value) = tail.split_once('=')?;
        // `derive_conjuncts` emits `<src> != '<k>' || <binding> == '<e>'`, so
        // the entry is the last quoted literal in the text.
        let entry = last_quoted_literal(condition)?;
        return Some(Attribution::Derive {
            binding: binding.to_string(),
            source: source.to_string(),
            source_value: source_value.to_string(),
            entry,
        });
    }
    if let Some(rest) = id.strip_prefix(ACCEPTS_ATTRIBUTION_PREFIX) {
        let (component, slot) = rest.split_once('.')?;
        let entries = accepted_entries(condition);
        if entries.is_empty() {
            return None;
        }
        return Some(Attribution::Accepts {
            component: component.to_string(),
            slot: slot.to_string(),
            entries,
        });
    }
    None
}

/// The entries an `accepts` conjunct names, in authored order.
///
/// Strips the [`guarded`] prefix first rather than searching the whole string:
/// the guard is an arbitrary authored condition and may itself contain
/// `any_of(...)` and quoted literals, so anything that scanned the text as a
/// whole would sooner or later read the guard's values as accepted entries.
fn accepted_entries(condition: &str) -> Vec<String> {
    let payload = unguard(condition.trim());
    match payload.strip_prefix("any_of(") {
        Some(args) => match balanced_paren_end(args) {
            Some(end) => quoted_literals(&args[..end]),
            None => Vec::new(),
        },
        // The single-entry form is the bare predicate `<binding> == '<e>'`.
        None => last_quoted_literal(payload).into_iter().collect(),
    }
}

/// Drop the `!(<condition>) || ` guard [`guarded`] adds, if present.
///
/// Matches the opening paren rather than searching for `||`, because the guard
/// is authored text: `!(a == 'x' || b == 'y') || any_of(...)` has three `||`
/// and only the last one is the separator.
fn unguard(condition: &str) -> &str {
    let Some(rest) = condition.strip_prefix("!(") else {
        return condition;
    };
    let Some(end) = balanced_paren_end(rest) else {
        return condition;
    };
    let after = rest[end + 1..].trim_start();
    match after.strip_prefix("||") {
        Some(payload) => payload.trim_start(),
        None => condition,
    }
}

/// Index of the `)` that closes the already-consumed opening paren, ignoring
/// parens inside quoted literals.
fn balanced_paren_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (idx, ch) in text.char_indices() {
        match (quote, ch) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '\'') | (None, '"') => quote = Some(ch),
            (None, '(') => depth += 1,
            (None, ')') => {
                if depth == 0 {
                    return Some(idx);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Every quoted literal in `text`, in order. Both quote characters are
/// recognised because [`facet_eq_predicate`] falls back to double quotes for a
/// value containing a single one.
fn quoted_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(['\'', '"']) {
        let Some(quote) = rest[start..].chars().next() else {
            break;
        };
        let body = &rest[start + quote.len_utf8()..];
        let Some(end) = body.find(quote) else {
            break;
        };
        out.push(body[..end].to_string());
        rest = &body[end + quote.len_utf8()..];
    }
    out
}

/// The last quoted literal in `text`, or `None` when it does not end in one.
fn last_quoted_literal(text: &str) -> Option<String> {
    let trimmed = text.trim_end();
    let quote = trimmed.chars().last()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let body = &trimmed[..trimmed.len() - quote.len_utf8()];
    let start = body.rfind(quote)?;
    Some(body[start + quote.len_utf8()..].to_string())
}

#[cfg(test)]
#[path = "lowering_tests.rs"]
mod lowering_tests;
