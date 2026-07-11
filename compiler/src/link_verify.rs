// SPDX-License-Identifier: BUSL-1.1

use crate::conditions;
use crate::schema::{Component, Facet, Parameter};
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

/// Collect every authored `condition` string from the merged model in a stable,
/// id-sorted order: definition override chains first, then each component's own
/// activation condition and its params' override chains. Mirrors
/// `compiler_core::collect_ccm_clauses`'s harvest (that method walks the raw
/// chunks; this one walks the merged repository) so the closed-domain check
/// sees exactly the conditions the selection model will.
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

