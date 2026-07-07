// SPDX-License-Identifier: BUSL-1.1

use crate::conditions;
use crate::schema::{Component, Parameter};
use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};

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

    for root in &roots {
        let mut seen = HashSet::new();
        detect_diamond(root, root, &edges, &mut seen, 0)?;
    }

    Ok(())
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

fn detect_diamond(
    root: &str,
    node: &str,
    edges: &HashMap<String, Vec<String>>,
    seen: &mut HashSet<String>,
    depth: usize,
) -> Result<()> {
    // Explicit depth (not `seen.len()`): `seen` counts every reachable node,
    // which on a wide-but-legitimate tree far exceeds the path depth this
    // ceiling is bounding (configflux-xowl.5).
    if depth > MAX_CHAIN_DEPTH {
        bail!(
            "Component dependency chain exceeds the maximum supported depth {} at '{}'",
            MAX_CHAIN_DEPTH,
            node
        );
    }
    if !seen.insert(node.to_string()) {
        bail!(
            "Diamond dependency detected from '{}' reaching '{}' via multiple paths",
            root,
            node
        );
    }

    if let Some(deps) = edges.get(node) {
        for dep in deps {
            detect_diamond(root, dep, edges, seen, depth + 1)?;
        }
    }

    Ok(())
}
