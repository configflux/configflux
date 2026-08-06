// SPDX-License-Identifier: BUSL-1.1

use crate::conditions;
use crate::schema::{Component, Constraint, Facet, Parameter};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Copy, Clone, PartialEq)]
enum VisitState {
    Visiting,
    Done,
}

/// Maximum inheritance / dependency chain depth accepted by the DFS guards
/// below. Cycles are rejected by the Visiting/Done coloring, but a deeply
/// nested ACYCLIC chain still recurses once per hop — without a ceiling, a
/// crafted config (tens of thousands of chained nodes fit inside the 8 MiB
/// input bound) overflows the stack and ABORTS the process instead of failing
/// closed with a diagnostic (configflux-xowl.5). Real models sit well under
/// depth 100; 1_000 gives an order of magnitude of headroom while keeping
/// worst-case recursion inside even a 2 MiB test-thread stack in unoptimized
/// builds (frames are largest at opt-level 0, so the ceiling must be sized
/// for debug, not release).
const MAX_CHAIN_DEPTH: usize = 1_000;

// Inheritance-cycle detection is RETAINED in Rust (ADR-0027 Decision 6
// carve-out, configflux-73fr). ConfigFlux carries `inherits` as a string
// pointer resolved later, so a cyclic chain (`a -> b -> a`) is not necessarily
// a CUE structural cycle. The carve-out's deletion condition is now resolved to
// OUTCOME B and the guard is PERMANENT: the missing evidence was supplied by the
// fixture in `compiler/cue/validate_fixtures.sh` (the "cyclic inherits a->b->a is
// structurally valid" case, configflux-0zql), which measured with the pinned cue
// that `cue vet -d '#Config'` ACCEPTS the cycle (exit 0) and the whole-pack
// `#ResolvePack` resolution path ALSO accepts it (exit 0) — CUE does NOT reject
// an inheritance cycle, because `#ResolveParam` dereferences `inherits` by a
// single string-pointer hop, never structurally. So deterministic rejection is
// this DFS guard's job; `lib_tests::test_link_and_verify_definition_inheritance_cycle`
// proves it bails on `a -> b -> a`. The *target-existence* half
// (`validate_param_inherits` / `collect_param_inherits`) was deleted: unknown-
// definition targets are now owned by the CUE authoring layer, and
// `detect_definition_cycle` itself still rejects an unknown parent on the
// definition `inherits` chain.
pub(crate) fn validate_definition_inheritance(
    definitions: &HashMap<String, Parameter>,
) -> Result<()> {
    let mut states: HashMap<String, VisitState> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    // Sorted roots keep the traversal — and therefore which diagnostic
    // surfaces first — deterministic across runs (HashMap order is not).
    let mut roots: Vec<&String> = definitions.keys().collect();
    roots.sort();
    for name in roots {
        detect_definition_cycle(name, definitions, &mut states, &mut stack)?;
    }

    Ok(())
}

pub(crate) fn validate_component_dependencies(
    components: &HashMap<String, Component>,
) -> Result<()> {
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for (name, comp) in components {
        let mut seen = HashSet::new();
        let mut deps = Vec::new();
        for dep in &comp.depends_on {
            if seen.insert(dep.as_str()) {
                deps.push(dep.clone());
            }
        }
        edges.insert(name.clone(), deps);
    }

    for (name, deps) in &edges {
        for dep in deps {
            if name == dep {
                bail!("Component '{}' depends_on itself", name);
            }
            if !components.contains_key(dep) {
                bail!(
                    "Component '{}' depends_on unknown component '{}'",
                    name,
                    dep
                );
            }
        }
    }

    // Component-parameter `inherits` target-existence validation was deleted in
    // B-5 (ADR-0027 Decision 5): unknown-definition targets are owned by the CUE
    // authoring layer, and any that slip through are still rejected at resolve
    // time (`resolver::resolve_parameter`). Definition-chain target existence
    // and cycles remain guarded by `detect_definition_cycle`.

    for (name, deps) in &edges {
        let comp = components
            .get(name)
            .with_context(|| format!("Missing component '{}'", name))?;
        for dep in deps {
            let dep_comp = components
                .get(dep)
                .with_context(|| format!("Missing component '{}'", dep))?;
            let implies = conditions::condition_implies_typed(
                comp.condition.as_deref(),
                dep_comp.condition.as_deref(),
            )
            .with_context(|| {
                format!(
                    "Failed condition implication check for '{}' -> '{}'",
                    name, dep
                )
            })?;
            if !implies {
                bail!(
                    "Condition incompatibility: '{}' ({}) depends_on '{}' ({})",
                    name,
                    comp.condition.as_deref().unwrap_or("<none>"),
                    dep,
                    dep_comp.condition.as_deref().unwrap_or("<none>")
                );
            }
        }
    }

    let mut states: HashMap<String, VisitState> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    // Sorted roots: deterministic traversal order, see
    // validate_definition_inheritance.
    let mut roots: Vec<&String> = components.keys().collect();
    roots.sort();
    for name in &roots {
        detect_cycle(name, &edges, &mut states, &mut stack)?;
    }

    // Diamond dependencies (a component reached via multiple paths from one
    // root) are permitted: the component graph may be any DAG. The former
    // per-root diamond check was retired by ADR-0048 — it was conservative
    // policy, not a correctness requirement. Acyclicity is still enforced by
    // `detect_cycle` above, and the genuine hazard (a component enabled where a
    // shared dependency is disabled) is caught by the per-edge
    // condition-implication check earlier in this function.

    Ok(())
}


/// Validate the pack's facet declarations over the fully merged model
/// (ADR-0047 §§1,3). Two layers:
///
///   1. Per-facet shape invariants, re-validated in Rust even though CUE also
///      checks the shape (ADR-0021 "CUE authors, Rust re-validates" — the Rust
///      pass is defense-in-depth, not a second authoring path): `values` is
///      non-empty, `values` has no duplicates, and `default` (when present) is
///      one of `values`.
///   2. The closed-domain condition check: a *closed* facet's declared
///      `values` are its exhaustive domain, so an equality predicate on that
///      facet that names a value outside the domain is an authoring error
///      (`E_FACET_VALUE_UNDECLARED`). An *open* facet extends its effective
///      domain with condition-referenced values (no error), and a facet with
///      no declaration at all is not consulted here — its domain stays the
///      condition-inferred set, exactly as before this feature (incremental
///      adoption, §3).
///
/// Only `==` predicates are checked, matching the domain-inference contract:
/// `register_facet_domains` widens a facet's domain on `Eq` atoms only
/// (`selection_eval::for_each_eq_predicate`), so an `Eq` literal is precisely
/// what "the facet's domain must contain this value" means. Conditions that do
/// not parse widen no domain today (mirroring `collect_ccm_clauses`), so they
/// cannot violate a closed domain and are skipped.
pub(crate) fn validate_facets(
    facets: &HashMap<String, Facet>,
    components: &HashMap<String, Component>,
    definitions: &HashMap<String, Parameter>,
) -> Result<()> {
    // Sorted ids: the first surfaced diagnostic is deterministic across runs
    // (HashMap iteration order is not), matching the other validators.
    let mut facet_ids: Vec<&String> = facets.keys().collect();
    facet_ids.sort();

    for id in &facet_ids {
        let facet = &facets[*id];
        if facet.values.is_empty() {
            bail!("Facet '{}' declares an empty value domain", id);
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for value in &facet.values {
            if !seen.insert(value.as_str()) {
                bail!("Facet '{}' declares duplicate value '{}'", id, value);
            }
        }
        if let Some(default) = &facet.default {
            if !facet.values.iter().any(|v| v == default) {
                bail!(
                    "Facet '{}' default '{}' is not one of its declared values [{}]",
                    id,
                    default,
                    facet.values.join(", ")
                );
            }
        }
    }

    // Index the closed facets by id. Open and undeclared facets never gate a
    // condition value here.
    let mut closed: BTreeMap<&str, &Facet> = BTreeMap::new();
    for id in &facet_ids {
        let facet = &facets[*id];
        if !facet.open {
            closed.insert(id.as_str(), facet);
        }
    }
    if closed.is_empty() {
        return Ok(());
    }

    let mut conditions: Vec<String> = Vec::new();
    collect_model_conditions(components, definitions, &mut conditions);
    for condition in &conditions {
        let trimmed = condition.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(expr) = conditions::parse_condition_expr(trimmed) else {
            continue;
        };
        let mut violation: Option<(String, String)> = None;
        conditions::for_each_eq_predicate(&expr, |tag, value| {
            if violation.is_some() {
                return;
            }
            if let Some(facet) = closed.get(tag) {
                if !facet.values.iter().any(|v| v == value) {
                    violation = Some((tag.to_string(), value.to_string()));
                }
            }
        });
        if let Some((tag, value)) = violation {
            let facet = closed[tag.as_str()];
            bail!(
                "Condition value '{}' is not in the closed facet '{}' domain [{}]",
                value,
                tag,
                facet.values.join(", ")
            );
        }
    }

    Ok(())
}

/// Validate the pack's `constraints` declarations over the fully merged model
/// (ADR-0054 §4). Three rules, checked in constraint-id-ascending order so the
/// first surfaced diagnostic is deterministic:
///
///   1. **Every constraint expression parses.** This is the one place where a
///      constraint and a selector `condition` are treated differently at
///      ingest, and the difference is deliberate: an unparseable *selector*
///      widens no facet domain and is skipped (see `validate_facets`), because
///      the worst case is a filter that includes too much. An unparseable
///      *constraint* is a policy nobody can evaluate — silently dropping it
///      would fail OPEN on the surface whose whole job is to say "no". So it is
///      a hard ingest ERROR (ADR-0054 §1).
///   2. **Every facet it names is DECLARED** (ADR-0047), not merely mentioned
///      by some selector condition (configflux-6j91). A constraint asserts over
///      a domain; it does not create one, and a condition-inferred domain is an
///      artifact of what conditions happened to mention rather than something
///      the author wrote down. The rule is not stylistic — it is what keeps the
///      three surfaces in agreement. ADR-0054 §5.2 synthesizes intra-facet
///      cardinality for DECLARED facets only, so a constraint over an inferred
///      facet gets no at-most-one clauses: a positive equality
///      (`arch == 'x86'`) leaves `root ∧ arch.x86 ∧ arch.arm` satisfiable over
///      independent variables, so `options`/`select` keep offering `arm` while
///      `resolve`, which evaluates the constraint concretely under a total
///      assignment, rejects it. Failing the compile closes that fail-open
///      corner from the validation side; §5.2's refusal to synthesize over an
///      inferred domain closes it from the other.
///
///      Note this rule needs no parseable/unparseable carve-out: unlike
///      `validate_facets`, it never consults conditions at all, so a selector
///      cannot widen a constraint's legal name set whether it parses or not.
///   3. **Every value it names is a member of a closed facet's declared
///      domain** — the existing `E_FACET_VALUE_UNDECLARED` rule, now applied to
///      constraints too. Unlike `validate_facets`, BOTH operators are checked,
///      not just `==`: `validate_facets` checks only `==` because only `==`
///      widens an inferred domain, but ADR-0054 §4 says every value a
///      constraint *names*, and against a closed domain a mistyped
///      `environment != 'prod0'` is not a harmless no-op — it is a tautology
///      that silently voids the policy. Open and undeclared facets are not
///      gated here, exactly as in `validate_facets`.
///
/// Takes no components or definitions on purpose: the legal name set is exactly
/// `facets.keys()`, and a function that cannot see the model's conditions cannot
/// regress into unioning them back in (configflux-6j91).
pub(crate) fn validate_constraints(
    constraints: &HashMap<String, Constraint>,
    facets: &HashMap<String, Facet>,
) -> Result<()> {
    if constraints.is_empty() {
        return Ok(());
    }

    let mut constraint_ids: Vec<&String> = constraints.keys().collect();
    constraint_ids.sort();

    for id in constraint_ids {
        let text = constraints[id].condition.trim();
        let expr = conditions::parse_condition_expr(text).with_context(|| {
            format!(
                "Constraint '{}' expression does not parse: '{}'",
                id, text
            )
        })?;

        // First violation in AST order wins, so the reported diagnostic is a
        // deterministic function of the authored text.
        let mut violation: Option<ConstraintViolation> = None;
        conditions::for_each_predicate_symbol(&expr, |tag, value| {
            if violation.is_some() {
                return;
            }
            let Some(facet) = facets.get(tag) else {
                violation = Some(ConstraintViolation::UndeclaredFacet(tag.to_string()));
                return;
            };
            if !facet.open && !facet.values.iter().any(|v| v == value) {
                violation = Some(ConstraintViolation::UndeclaredValue(
                    tag.to_string(),
                    value.to_string(),
                ));
            }
        });

        match violation {
            // Deliberately NOT "unknown facet": the facet a model most often
            // trips this rule with is one the author can see all over their own
            // conditions, and calling it unknown would be false. The offending
            // name and the remedy both have to be in the message, because
            // "declare it" is the only fix that keeps the policy.
            //
            // "is not declared under `facets`" is also the phrase
            // `product_api::map_graph_error` keys on to code this
            // E_FACET_VALUE_UNDECLARED, so it is load-bearing, not decorative
            // (configflux-6j91). Rewording it without the matching arm demotes
            // the diagnostic to the generic ingest bucket;
            // //compiler:constraint_facet_diagnostic_test is what catches that.
            Some(ConstraintViolation::UndeclaredFacet(tag)) => bail!(
                "Constraint '{}' references facet '{}', which is not declared under `facets`: \
                 declare the facet with its value domain, or remove it from the constraint",
                id,
                tag
            ),
            Some(ConstraintViolation::UndeclaredValue(tag, value)) => {
                let facet = &facets[&tag];
                // Phrasing carries "closed facet", which
                // `product_api::map_graph_error` maps to E_FACET_VALUE_UNDECLARED.
                bail!(
                    "Constraint '{}' value '{}' is not in the closed facet '{}' domain [{}]",
                    id,
                    value,
                    tag,
                    facet.values.join(", ")
                );
            }
            None => {}
        }
    }

    Ok(())
}

/// The first rule a constraint's predicate walk broke. Carried out of the
/// `for_each_predicate_symbol` closure so the `bail!` happens outside it.
enum ConstraintViolation {
    UndeclaredFacet(String),
    UndeclaredValue(String, String),
}

/// Collect every authored `condition` string from the merged model in a stable,
/// id-sorted order: definition override chains first, then each component's own
/// activation condition and its params' override chains. Mirrors
/// `compiler_core::collect_ccm_clauses`'s harvest (that method walks the raw
/// chunks; this one walks the merged repository) so the closed-domain check
/// sees exactly the conditions the selection model will.
///
/// This check is deliberately **kind-agnostic** (configflux-9xxq / ADR-0054
/// §5.1): `collect_ccm_clauses` now lowers a component condition and a
/// parameter-override condition differently — assertion vs. symbol-only branch
/// selector — but naming a value outside a closed facet's declared domain is an
/// authoring error under either one, and both still contribute that value's
/// symbol. So the harvest stays flat here.
fn collect_model_conditions(
    components: &HashMap<String, Component>,
    definitions: &HashMap<String, Parameter>,
    out: &mut Vec<String>,
) {
    let mut definition_ids: Vec<&String> = definitions.keys().collect();
    definition_ids.sort();
    for id in definition_ids {
        collect_parameter_conditions(&definitions[id], out);
    }

    let mut component_ids: Vec<&String> = components.keys().collect();
    component_ids.sort();
    for id in component_ids {
        let component = &components[id];
        if let Some(condition) = &component.condition {
            out.push(condition.clone());
        }
        let mut param_ids: Vec<&String> = component.params.keys().collect();
        param_ids.sort();
        for pid in param_ids {
            collect_parameter_conditions(&component.params[pid], out);
        }
    }
}

/// Recursively collect the `condition` strings from a parameter's override
/// chain, in override order then nested-override order.
fn collect_parameter_conditions(parameter: &Parameter, out: &mut Vec<String>) {
    for override_block in &parameter.overrides {
        out.push(override_block.condition.clone());
        collect_parameter_conditions(override_block.payload.as_ref(), out);
    }
}

// DFS over the definition `inherits` string-pointer graph. Permanent per the
// ADR-0027 Decision 6 carve-out (OUTCOME B): CUE does not reject an inheritance
// cycle — see the "cyclic inherits a->b->a" fixture in
// compiler/cue/validate_fixtures.sh (configflux-0zql) and the header comment on
// `validate_definition_inheritance` above. This function is the deterministic
// rejection; `a -> b -> a` bails with "Definition inheritance cycle detected".
fn detect_definition_cycle(
    node: &str,
    definitions: &HashMap<String, Parameter>,
    states: &mut HashMap<String, VisitState>,
    stack: &mut Vec<String>,
) -> Result<()> {
    match states.get(node).copied() {
        Some(VisitState::Visiting) => {
            let cycle_start = stack.iter().position(|n| n == node).unwrap_or(0);
            let mut cycle = stack[cycle_start..].to_vec();
            cycle.push(node.to_string());
            bail!(
                "Definition inheritance cycle detected: {}",
                cycle.join(" -> ")
            );
        }
        Some(VisitState::Done) => return Ok(()),
        None => {}
    }

    states.insert(node.to_string(), VisitState::Visiting);
    stack.push(node.to_string());
    if stack.len() > MAX_CHAIN_DEPTH {
        bail!(
            "Definition inheritance chain exceeds the maximum supported depth {} at '{}'",
            MAX_CHAIN_DEPTH,
            node
        );
    }

    if let Some(def) = definitions.get(node) {
        if let Some(parent) = &def.inherits {
            if !definitions.contains_key(parent) {
                bail!(
                    "Definition '{}' inherits unknown definition '{}'",
                    node,
                    parent
                );
            }
            detect_definition_cycle(parent, definitions, states, stack)?;
        }
    }

    stack.pop();
    states.insert(node.to_string(), VisitState::Done);
    Ok(())
}

fn detect_cycle(
    node: &str,
    edges: &HashMap<String, Vec<String>>,
    states: &mut HashMap<String, VisitState>,
    stack: &mut Vec<String>,
) -> Result<()> {
    match states.get(node).copied() {
        Some(VisitState::Visiting) => {
            let cycle_start = stack.iter().position(|n| n == node).unwrap_or(0);
            let mut cycle = stack[cycle_start..].to_vec();
            cycle.push(node.to_string());
            bail!(
                "Component dependency cycle detected: {}",
                cycle.join(" -> ")
            );
        }
        Some(VisitState::Done) => return Ok(()),
        None => {}
    }

    states.insert(node.to_string(), VisitState::Visiting);
    stack.push(node.to_string());
    if stack.len() > MAX_CHAIN_DEPTH {
        bail!(
            "Component dependency chain exceeds the maximum supported depth {} at '{}'",
            MAX_CHAIN_DEPTH,
            node
        );
    }

    if let Some(deps) = edges.get(node) {
        for dep in deps {
            detect_cycle(dep, edges, states, stack)?;
        }
    }

    stack.pop();
    states.insert(node.to_string(), VisitState::Done);
    Ok(())
}

#[cfg(test)]
mod facet_tests {
    use super::*;
    use crate::schema::{Component, Facet, Parameter};

    fn facet(values: &[&str], default: Option<&str>, open: bool) -> Facet {
        Facet {
            values: values.iter().map(|s| s.to_string()).collect(),
            default: default.map(|s| s.to_string()),
            open,
            doc: None,
        }
    }

    fn facets(pairs: Vec<(&str, Facet)>) -> HashMap<String, Facet> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn component_with_condition(cond: &str) -> HashMap<String, Component> {
        let mut map = HashMap::new();
        map.insert(
            "c".to_string(),
            Component {
                r#type: None,
                condition: Some(cond.to_string()),
                depends_on: Vec::new(),
                params: HashMap::new(),
            },
        );
        map
    }

    fn param_with_override_condition(cond: &str) -> HashMap<String, Parameter> {
        let mut inner = empty_param();
        let base = empty_param();
        inner.overrides = vec![crate::schema::ConditionalBlock {
            condition: cond.to_string(),
            payload: Box::new(base),
        }];
        let mut map = HashMap::new();
        map.insert("d".to_string(), inner);
        map
    }

    fn empty_param() -> Parameter {
        Parameter {
            inherits: None,
            r#type: None,
            unit: None,
            doc: None,
            value: None,
            lifecycle: None,
            safety: None,
            access: None,
            limits: None,
            req_id: None,
            overrides: Vec::new(),
        }
    }

    #[test]
    fn empty_values_is_rejected() {
        let f = facets(vec![("region", facet(&[], None, false))]);
        let err = validate_facets(&f, &HashMap::new(), &HashMap::new()).unwrap_err();
        assert!(format!("{err}").contains("empty value domain"), "err: {err}");
    }

    #[test]
    fn duplicate_values_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "eu"], None, false))]);
        let err = validate_facets(&f, &HashMap::new(), &HashMap::new()).unwrap_err();
        assert!(format!("{err}").contains("duplicate value 'eu'"), "err: {err}");
    }

    #[test]
    fn default_not_in_values_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("mars"), false))]);
        let err = validate_facets(&f, &HashMap::new(), &HashMap::new()).unwrap_err();
        assert!(
            format!("{err}").contains("default 'mars' is not one of"),
            "err: {err}"
        );
    }

    #[test]
    fn valid_facet_with_default_is_accepted() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        assert!(validate_facets(&f, &HashMap::new(), &HashMap::new()).is_ok());
    }

    #[test]
    fn closed_facet_undeclared_condition_value_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let comps = component_with_condition("region == 'mars'");
        let err = validate_facets(&f, &comps, &HashMap::new()).unwrap_err();
        // The message names the facet, the value, and the declared domain, and
        // carries the phrase product_api maps to E_FACET_VALUE_UNDECLARED.
        let msg = format!("{err}");
        assert!(msg.contains("closed facet 'region'"), "err: {msg}");
        assert!(msg.contains("'mars'"), "err: {msg}");
        assert!(msg.contains("eu, us"), "err: {msg}");
    }

    #[test]
    fn closed_facet_declared_condition_value_is_accepted() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let comps = component_with_condition("region == 'us'");
        assert!(validate_facets(&f, &comps, &HashMap::new()).is_ok());
    }

    #[test]
    fn open_facet_extends_with_undeclared_condition_value() {
        // `open: true` means the declared domain is extensible, so a condition
        // value outside it is NOT an error (ADR-0047 §3).
        let f = facets(vec![("region", facet(&["eu"], Some("eu"), true))]);
        let comps = component_with_condition("region == 'mars'");
        assert!(validate_facets(&f, &comps, &HashMap::new()).is_ok());
    }

    #[test]
    fn undeclared_facet_keeps_legacy_inferred_behavior() {
        // A condition referencing a facet with no declaration at all is never
        // gated — its domain stays the inferred set, exactly as before.
        let comps = component_with_condition("variant == 'heavy'");
        assert!(validate_facets(&HashMap::new(), &comps, &HashMap::new()).is_ok());
    }

    #[test]
    fn closed_facet_check_also_scans_param_override_conditions() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let defs = param_with_override_condition("region == 'mars'");
        let err = validate_facets(&f, &HashMap::new(), &defs).unwrap_err();
        assert!(
            format!("{err}").contains("closed facet 'region'"),
            "err: {err}"
        );
    }

    #[test]
    fn unparseable_condition_cannot_violate_a_closed_domain() {
        // A condition the BDD grammar cannot represent widens no domain today,
        // so it must not trip the closed-domain check either.
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let comps = component_with_condition("this is not <> a condition");
        assert!(validate_facets(&f, &comps, &HashMap::new()).is_ok());
    }
}

#[cfg(test)]
mod constraint_tests {
    use super::*;
    use crate::schema::{Constraint, Facet};

    fn facet(values: &[&str], open: bool) -> Facet {
        Facet {
            values: values.iter().map(|s| s.to_string()).collect(),
            default: None,
            open,
            doc: None,
        }
    }

    fn facets(pairs: Vec<(&str, Facet)>) -> HashMap<String, Facet> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn constraints(pairs: Vec<(&str, &str)>) -> HashMap<String, Constraint> {
        pairs
            .into_iter()
            .map(|(id, condition)| {
                (
                    id.to_string(),
                    Constraint {
                        condition: condition.to_string(),
                        doc: None,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_model_with_no_constraints_is_accepted() {
        assert!(validate_constraints(&HashMap::new(), &HashMap::new()).is_ok());
    }

    #[test]
    fn unparseable_constraint_is_an_ingest_error() {
        // THE asymmetry with a selector condition (ADR-0054 §1): an
        // unparseable selector is skipped, an unparseable CONSTRAINT is fatal.
        // A policy nobody can evaluate must never be silently dropped.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("bogus", "this is not <> a condition")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Constraint 'bogus'"), "err: {msg}");
        assert!(msg.contains("does not parse"), "err: {msg}");
    }

    #[test]
    fn constraint_over_a_declared_facet_is_accepted() {
        let f = facets(vec![
            ("environment", facet(&["dev", "staging", "prod"], false)),
            ("log_level", facet(&["info", "debug"], false)),
        ]);
        let c = constraints(vec![(
            "prod_forbids_debug",
            "environment != 'prod' || log_level != 'debug'",
        )]);
        assert!(validate_constraints(&c, &f).is_ok());
    }

    #[test]
    fn constraint_over_an_undeclared_facet_is_rejected() {
        // configflux-6j91. Undeclared is undeclared: this function cannot see
        // the model's conditions, so a facet some selector mentions and a facet
        // nothing mentions are the same input here and take the same diagnostic
        // — which is the point of dropping the union. The two shapes are told
        // apart where whole models exist:
        // `compiler_core_tests::a_constraint_over_a_condition_inferred_facet_
        // fails_before_anything_is_emitted` pins the condition-inferred one.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("ghost", "tls_mode == 'strict'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        let msg = format!("{err}");
        // Constraint id, offending facet, and the remedy all have to be here:
        // "declare it" is the only fix that keeps the policy.
        assert!(msg.contains("Constraint 'ghost'"), "err: {msg}");
        assert!(msg.contains("facet 'tls_mode'"), "err: {msg}");
        assert!(msg.contains("not declared"), "err: {msg}");
        assert!(msg.contains("declare the facet"), "err: {msg}");
        // Both rules carry E_FACET_VALUE_UNDECLARED, but this one is not a
        // closed-domain violation — `tls_mode` has no domain at all — so
        // borrowing the sibling's phrasing would make the message false.
        assert!(!msg.contains("closed facet"), "err: {msg}");
    }

    #[test]
    fn the_offending_facet_is_named_even_when_a_later_predicate_is_declared() {
        // The walk must report the first illegal facet in AST order, not fall
        // through because some other predicate in the same expression is fine.
        let f = facets(vec![("environment", facet(&["dev", "prod"], false))]);
        let c = constraints(vec![("mixed", "arch == 'x86' || environment != 'prod'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        assert!(format!("{err}").contains("facet 'arch'"), "err: {err}");
    }

    #[test]
    fn closed_facet_undeclared_eq_value_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("bad", "region == 'mars'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        // Carries the phrase product_api maps to E_FACET_VALUE_UNDECLARED, and
        // names the constraint so the author knows which policy is wrong.
        let msg = format!("{err}");
        assert!(msg.contains("Constraint 'bad'"), "err: {msg}");
        assert!(msg.contains("closed facet 'region'"), "err: {msg}");
        assert!(msg.contains("eu, us"), "err: {msg}");
    }

    #[test]
    fn closed_facet_undeclared_ne_value_is_also_rejected() {
        // Deliberately STRICTER than `validate_facets`, which checks `==` only.
        // Against a closed domain `region != 'mars'` is a tautology, so a typo
        // here does not merely fail to filter — it silently voids the policy.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("typo", "region != 'marz'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        assert!(
            format!("{err}").contains("closed facet 'region'"),
            "err: {err}"
        );
    }

    #[test]
    fn open_facet_accepts_a_value_outside_its_declared_domain() {
        // An open domain is extensible (ADR-0047 §3), so the closed-domain rule
        // does not apply — same carve-out `validate_facets` makes.
        let f = facets(vec![("region", facet(&["eu"], true))]);
        let c = constraints(vec![("ok", "region != 'mars'")]);
        assert!(validate_constraints(&c, &f).is_ok());
    }

    #[test]
    fn disjunctive_and_negated_constraints_are_supported_verbatim() {
        // The whole point of the construct: a policy is an arbitrary
        // `ConditionExpr`, not a pure conjunction. Nothing here may filter on
        // shape (ADR-0054 §2).
        let f = facets(vec![
            ("environment", facet(&["dev", "prod"], false)),
            ("log_level", facet(&["info", "debug"], false)),
        ]);
        let c = constraints(vec![
            ("disjunction", "environment != 'prod' || log_level != 'debug'"),
            ("negation", "!(environment == 'prod')"),
            ("cardinality", "exactly_one_of(log_level == 'info', log_level == 'debug')"),
        ]);
        assert!(validate_constraints(&c, &f).is_ok());
    }

    #[test]
    fn the_first_reported_violation_is_id_ascending() {
        // Two broken constraints; the diagnostic must be a deterministic
        // function of the model, not of HashMap iteration order.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![
            ("zeta", "region == 'pluto'"),
            ("alpha", "region == 'mars'"),
        ]);
        let err = validate_constraints(&c, &f).unwrap_err();
        assert!(format!("{err}").contains("Constraint 'alpha'"), "err: {err}");
    }

}

