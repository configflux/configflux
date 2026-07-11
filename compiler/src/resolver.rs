// SPDX-License-Identifier: BUSL-1.1

use crate::conditions;
use crate::resolved_models::{ResolvedComponent, ResolvedConfig, ResolvedParameter};
use crate::schema::{self, Config, Parameter};
use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeSelector {
    Component(String),
    Platform(String),
    PlatformAll,
    All,
}

pub fn parse_scope_selectors(input: &str) -> Result<Vec<ScopeSelector>> {
    let mut selectors = Vec::new();
    for raw in input.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if token == "all" {
            selectors.push(ScopeSelector::All);
            continue;
        }
        if let Some(name) = token.strip_prefix("//") {
            ensure_snake_case_ident(name)?;
            selectors.push(ScopeSelector::Component(name.to_string()));
            continue;
        }
        if let Some(name) = token.strip_prefix("component:") {
            ensure_snake_case_ident(name)?;
            selectors.push(ScopeSelector::Component(name.to_string()));
            continue;
        }
        if let Some(name) = token.strip_prefix("components:") {
            ensure_snake_case_ident(name)?;
            selectors.push(ScopeSelector::Component(name.to_string()));
            continue;
        }
        if let Some(name) = token.strip_prefix("platform:") {
            if name == "all" {
                selectors.push(ScopeSelector::PlatformAll);
            } else {
                ensure_snake_case_ident(name)?;
                selectors.push(ScopeSelector::Platform(name.to_string()));
            }
            continue;
        }
        bail!("Unrecognized scope selector '{}'", token);
    }

    if selectors.is_empty() {
        bail!("Scope selector is empty");
    }

    if selectors.iter().any(|s| matches!(s, ScopeSelector::All)) && selectors.len() > 1 {
        bail!("Scope selector 'all' cannot be combined with other selectors");
    }

    Ok(selectors)
}

fn ensure_snake_case_ident(ident: &str) -> Result<()> {
    let bytes = ident.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
        bail!("Scope identifier must be snake_case: '{}'", ident);
    }
    let mut prev_underscore = false;
    for &b in bytes {
        if b.is_ascii_lowercase() || b.is_ascii_digit() {
            prev_underscore = false;
            continue;
        }
        if b == b'_' {
            if prev_underscore {
                bail!("Scope identifier must be snake_case (no '__'): '{}'", ident);
            }
            prev_underscore = true;
            continue;
        }
        bail!("Scope identifier must be snake_case: '{}'", ident);
    }
    Ok(())
}

/// The Runtime Context for Resolution.
/// Contains the "Tags" defining the target environment.
#[derive(Clone)]
pub struct ResolutionContext {
    pub tags: HashMap<String, String>,
}

impl ResolutionContext {
    /// Evaluates a boolean condition against the tag set using the in-crate
    /// evaluator (ADR-0008), e.g. "variant == 'heavy' && region != 'eu'".
    fn eval(&self, condition: &str) -> Result<bool> {
        conditions::eval_condition(condition, &self.tags)
    }
}

/// Main Entry Point: Transforms Raw Config -> Resolved Config
pub fn resolve(raw: Config, context: &ResolutionContext) -> Result<ResolvedConfig> {
    let mut resolved_components = HashMap::new();

    for (comp_name, comp) in raw.components {
        // 1. Component Level Filtering (The 150% -> 100% check)
        if let Some(cond) = &comp.condition {
            if !context.eval(cond)? {
                continue; // Skip this component entirely
            }
        }

        let comp_type = comp.r#type.clone().context(format!(
            "Component '{}' missing required 'type' after merge",
            comp_name
        ))?;

        // 2. Resolve Parameters
        let mut resolved_params = HashMap::new();
        for (param_name, param) in comp.params {
            // Perform the "Deep Flattening". The fully-qualified path is passed
            // down so any resolve-time diagnostic (e.g. the fail-closed
            // metadata assertion) can name the offending parameter
            // deterministically.
            let param_path = format!("components.{}.params.{}", comp_name, param_name);
            let resolved =
                resolve_parameter(param, &param_path, &raw.definitions, &raw.artifacts, context)
                    .with_context(|| {
                        format!(
                            "Error resolving parameter '{}' in component '{}'",
                            param_name, comp_name
                        )
                    })?;
            resolved_params.insert(param_name, resolved);
        }

        resolved_components.insert(
            comp_name,
            ResolvedComponent {
                r#type: comp_type,
                params: resolved_params,
            },
        );
    }

    Ok(ResolvedConfig {
        package: raw.package,
        version: raw.version,
        components: resolved_components,
    })
}

pub fn resolve_scoped(
    raw: &Config,
    context: &ResolutionContext,
    scope: &str,
) -> Result<HashMap<String, ResolvedConfig>> {
    let selectors = parse_scope_selectors(scope)?;
    if selectors.len() == 1 && matches!(selectors[0], ScopeSelector::All) {
        let resolved = resolve(raw.clone(), context)?;
        let mut outputs = HashMap::new();
        outputs.insert("all".to_string(), resolved);
        return Ok(outputs);
    }

    let mut roots = Vec::new();
    for selector in selectors {
        match selector {
            ScopeSelector::Component(id) => {
                if !raw.components.contains_key(&id) {
                    bail!("Scope selector refers to unknown component '{}'", id);
                }
                roots.push(id);
            }
            ScopeSelector::Platform(id) => {
                let comp = raw.components.get(&id).with_context(|| {
                    format!("Scope selector refers to unknown platform '{}'", id)
                })?;
                if comp.r#type.as_deref() != Some("platform") {
                    bail!("Scope selector '{}' is not a platform component", id);
                }
                roots.push(id);
            }
            ScopeSelector::PlatformAll => {
                let mut found = false;
                for (id, comp) in &raw.components {
                    if comp.r#type.as_deref() == Some("platform") {
                        roots.push(id.clone());
                        found = true;
                    }
                }
                if !found {
                    bail!("Scope selector 'platform:all' matched no components");
                }
            }
            ScopeSelector::All => {}
        }
    }

    roots.sort();
    roots.dedup();

    let mut outputs = HashMap::new();
    for root in roots {
        let closure = dependency_closure(&raw.components, &root)?;
        let mut scoped_components = HashMap::new();
        for comp_id in closure {
            let comp = raw
                .components
                .get(&comp_id)
                .with_context(|| format!("Missing component '{}'", comp_id))?
                .clone();
            scoped_components.insert(comp_id, comp);
        }

        let scoped = Config {
            package: raw.package.clone(),
            version: raw.version.clone(),
            definitions: raw.definitions.clone(),
            components: scoped_components,
            artifacts: raw.artifacts.clone(),
            facets: Default::default(),
        };
        let resolved = resolve(scoped, context)?;
        outputs.insert(root, resolved);
    }

    Ok(outputs)
}

fn dependency_closure(
    components: &HashMap<String, crate::schema::Component>,
    root: &str,
) -> Result<HashSet<String>> {
    if !components.contains_key(root) {
        bail!("Scope selector refers to unknown component '{}'", root);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![root.to_string()];
    while let Some(node) = stack.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        let comp = components
            .get(&node)
            .with_context(|| format!("Missing component '{}'", node))?;
        for dep in &comp.depends_on {
            stack.push(dep.clone());
        }
    }
    Ok(seen)
}

/// Recursive function to flatten a parameter + overrides + inheritance
fn resolve_parameter(
    mut param: Parameter,
    param_path: &str,
    definitions: &HashMap<String, Parameter>,
    artifacts: &HashMap<String, schema::Artifact>,
    context: &ResolutionContext,
) -> Result<ResolvedParameter> {
    // A. Inheritance is now resolved by CUE during whole-pack evaluation
    // (ADR-0027, Track B): gap-fill from the parent definition happens at
    // author time and is baked into the emitted chunk, so no resolve-time
    // copy is needed. The `inherits` pointer is still carried verbatim in the
    // chunk; we keep validating it resolves to a known definition (cycle and
    // unknown-target detection at link-verify is `detect_definition_cycle`).
    // The matching defence-in-depth check that the parent's *declared* metadata
    // was actually baked in (fail-closed against the silent safety-default sink,
    // configflux-ok46) is applied at step E once overrides have been merged.
    if let Some(parent_id) = &param.inherits {
        if !definitions.contains_key(parent_id) {
            bail!("Parameter inherits from unknown definition: {}", parent_id);
        }
    }

    // B. Apply Overrides (Recursive Variant Logic)
    apply_overrides_recursive(&mut param, context)?;

    // C. Validation (Ensure Mandatory Fields exist)
    let value = match param.value {
        Some(v) => v,
        None => bail!("Missing 'value' for parameter"),
    };

    let r#type = param.r#type.context("Missing 'type' for parameter")?;

    // D. Artifact validation
    if r#type == "artifact" {
        match &value {
            schema::Value::String(artifact_id) => {
                if !artifacts.contains_key(artifact_id) {
                    bail!("Artifact '{}' not found", artifact_id);
                }
            }
            _ => bail!("Artifact parameter value must be a string"),
        }
    }

    // E. Resolve safety/lifecycle/access metadata (fail-closed under inherits).
    // `param.value`/`param.r#type` were already consumed above; the remaining
    // fields are still owned by `param` and moved out here. The parent
    // definition chain (if any) is the source of truth for which metadata CUE
    // should have baked in (see `resolve_safety_metadata`). The FULL definition
    // -> definition `inherits` chain is passed, not just the immediate parent,
    // so a transitively-declared field is also caught (configflux-y11i).
    let (safety, lifecycle, access) = resolve_safety_metadata(
        param_path,
        param.inherits.as_deref(),
        definitions,
        param.safety,
        param.lifecycle,
        param.access,
    )?;

    // F. Construct Resolved Object
    Ok(ResolvedParameter {
        value,
        r#type,
        unit: param.unit,
        safety,
        lifecycle,
        access,
        req_id: param.req_id,
        doc: param.doc,
        limits: param.limits,
    })
}

/// Resolve the safety/lifecycle/access triple, applying the
/// QM/Runtime/Technician defaults — but FAILING CLOSED when a parameter that
/// carries `inherits` drops a metadata field any definition in its inheritance
/// chain declared (configflux-ok46 for the immediate parent; configflux-y11i
/// for the transitive case; defence-in-depth for ADR-0027 Decision 8).
///
/// Post-Track-B, `inherits` gap-fill happens only in CUE whole-pack export,
/// which copies the parent definition's declared safety/lifecycle/access into
/// the emitted chunk. So for any legitimate CUE-origin chunk every field the
/// parent declares is also present on the child, and the defaults below are
/// reached only for fields the parent itself omits (e.g. an `artifact` slot
/// like `driver_slot` declares lifecycle+access but no safety — its inheritor
/// legitimately has `safety: None` → QM default). The assertion therefore
/// changes no `resolved_output` for any real chunk and keeps the byte-stability
/// / cross-path equivalence corpora green. It trips only on a HAND-crafted
/// (non-CUE) chunk inheriting from a `safety: sil2` definition yet omitting
/// that field — the same silent safety-level downgrade configflux-qofj closed
/// for TOML, here closed format-agnostically at the resolve sink. A
/// standalone/root parameter (no `inherits`) keeps the defaults. Evaluated at
/// the sink, i.e. AFTER `apply_overrides_recursive` merges override payloads.
///
/// configflux-y11i: definitions may themselves carry `inherits`
/// (definition→definition chains are structurally valid — CUE accepts even a
/// cyclic `a -> b -> a`, see configflux-0zql). The one-level check ok46
/// installed missed a transitive drop: grandparent def declares `safety=sil3`;
/// intermediate def `{inherits: grandparent}` omits it; child param
/// `{inherits: intermediate}` omits it — the intermediate's `safety` is `None`,
/// so the immediate-parent-only guard silently defaulted the child to QM. We
/// therefore flatten the chain: a field is "declared by the chain" if ANY
/// reachable ancestor definition declares it. For the real CUE corpus (zero
/// definition-level `inherits`; every chunk's immediate parent is a flat root
/// definition) this is a strict no-op — the field is seen at the immediate
/// parent exactly as before.
fn resolve_safety_metadata(
    param_path: &str,
    inherits: Option<&str>,
    definitions: &HashMap<String, Parameter>,
    safety: Option<schema::SafetyLevel>,
    lifecycle: Option<schema::Lifecycle>,
    access: Option<schema::Role>,
) -> Result<(schema::SafetyLevel, schema::Lifecycle, schema::Role)> {
    if let Some(parent_id) = inherits {
        let declared = collect_chain_declared_metadata(parent_id, definitions);

        // A field is "dropped" when some ancestor definition declares it
        // (`Some`) but the resolved child does not carry it (`None`). CUE
        // gap-fill never drops a declared field, so this can only happen on a
        // hand-crafted chunk. Collect dropped fields in a fixed order so the
        // diagnostic is deterministic (no HashMap iteration, no addresses, no
        // timestamps).
        let mut missing: Vec<&str> = Vec::new();
        if declared.safety && safety.is_none() {
            missing.push("safety");
        }
        if declared.lifecycle && lifecycle.is_none() {
            missing.push("lifecycle");
        }
        if declared.access && access.is_none() {
            missing.push("access");
        }
        if !missing.is_empty() {
            // Name the full chain (param -> immediate parent -> ... ->
            // ancestor) so the author can see WHERE the rating originates. The
            // chain is built in deterministic traversal order and rendered as
            // prose; the canonical `[..]` slot stays reserved for the
            // missing-fields list (the most safety-critical content, and the
            // contract the fail-closed tests parse).
            bail!(
                "Parameter '{path}' inherits via chain {chain} but is missing \
                 baked-in metadata [{fields}] that its inheritance chain \
                 declares; refusing to silently default to safety=QM / \
                 lifecycle=Runtime / access=Technician. CUE whole-pack export \
                 bakes the parent's safety/lifecycle/access into every chunk \
                 (ADR-0027); author this chunk in CUE so the inherited metadata \
                 is resolved, rather than hand-crafting a chunk that carries \
                 `inherits` without it.",
                path = param_path,
                chain = declared.chain.join(" -> "),
                fields = missing.join(", "),
            );
        }
    }

    Ok((
        safety.unwrap_or(schema::SafetyLevel::QM),
        lifecycle.unwrap_or(schema::Lifecycle::Runtime),
        access.unwrap_or(schema::Role::Technician),
    ))
}

/// Which metadata fields are declared anywhere along a parameter's definition
/// `inherits` chain, plus the chain of definition ids walked (for diagnostics).
struct ChainDeclared {
    safety: bool,
    lifecycle: bool,
    access: bool,
    /// Definition ids visited, in traversal order (immediate parent first).
    chain: Vec<String>,
}

/// Walk the definition→definition `inherits` chain starting at `start_id`,
/// recording which of safety/lifecycle/access ANY visited definition declares.
///
/// CYCLE SAFETY (configflux-y11i): definition cycles are structurally possible
/// and are NOT pre-screened on the resolve path — `detect_definition_cycle`
/// lives in `link_verify` and is not guaranteed to have run before `resolve`
/// (CUE itself accepts `a -> b -> a`; see configflux-0zql). This walk MUST
/// terminate. We use a visited-set and bounded iteration: a node already seen
/// stops the walk (bounded-stop). This is safe — field-presence accumulation is
/// monotonic, so revisiting a node can declare nothing new; a dropped declared
/// field still trips the fail-closed assertion, and a cycle whose definitions
/// declare nothing simply yields the documented defaults. A cycle therefore
/// cannot produce a silent safety downgrade, while termination is guaranteed.
/// Cycle *rejection* remains `detect_definition_cycle`'s job at link-verify;
/// the resolver's contract here is only "never silently downgrade, always
/// terminate". A missing/unknown id mid-chain also stops the walk (the
/// immediate-parent-unknown case is already a hard error earlier in
/// `resolve_parameter`; a deeper dangling link cannot mask a declared field).
fn collect_chain_declared_metadata(
    start_id: &str,
    definitions: &HashMap<String, Parameter>,
) -> ChainDeclared {
    let mut declared = ChainDeclared {
        safety: false,
        lifecycle: false,
        access: false,
        chain: Vec::new(),
    };
    let mut visited: HashSet<&str> = HashSet::new();
    let mut current = Some(start_id);

    while let Some(id) = current {
        if !visited.insert(id) {
            break; // cycle / already-seen: bounded-stop (see fn doc).
        }
        declared.chain.push(id.to_string());

        let def = match definitions.get(id) {
            Some(def) => def,
            None => break, // dangling link: stop (cannot mask a declared field).
        };
        declared.safety |= def.safety.is_some();
        declared.lifecycle |= def.lifecycle.is_some();
        declared.access |= def.access.is_some();

        current = def.inherits.as_deref();
    }

    declared
}

/// Depth-First Override Resolution
/// 1. Evaluates conditions.
/// 2. If true, recurses into the payload (to handle nested overrides).
/// 3. Merges the result into the base parameter.
fn apply_overrides_recursive(param: &mut Parameter, context: &ResolutionContext) -> Result<()> {
    // We take the overrides out of the struct to avoid mutable borrow conflicts
    // while iterating.
    let overrides = std::mem::take(&mut param.overrides);

    for block in overrides {
        if context.eval(&block.condition)? {
            // 1. Prepare the payload
            // The payload is a Box<Parameter>. We dereference it to get the struct.
            let mut active_payload = *block.payload;

            // 2. Recurse!
            // The payload itself might have `overrides` (nested conditions).
            // We must resolve those BEFORE merging upwards.
            apply_overrides_recursive(&mut active_payload, context)?;

            // 3. Merge the resolved payload into the current param
            merge_params(param, active_payload)?;
        }
    }

    Ok(())
}

// --- Helpers for Merging ---

// `merge_params` is retained: it is the conflict-aware merge used by the
// override/late-binding path (`apply_overrides_recursive`), which stays
// Rust-owned per ADR-0027 Decision 6 (`overrides` are opaque pass-through,
// late-bound against runtime tags). The hand-rolled `apply_inheritance`
// gap-fill engine it used to sit beside was deleted in B-5 (ADR-0027
// Decision 5) once CUE became the semantic owner of `inherits` resolution.
fn merge_params(base: &mut Parameter, patch: Parameter) -> Result<()> {
    // Sanity Check: Prevent overwriting strict physical properties with conflicting ones
    if let (Some(base_unit), Some(patch_unit)) = (&base.unit, &patch.unit) {
        if base_unit != patch_unit {
            bail!(
                "Unit Conflict: Cannot merge patch with unit '{}' into base with unit '{}'",
                patch_unit,
                base_unit
            );
        }
    }

    if let (Some(base_type), Some(patch_type)) = (&base.r#type, &patch.r#type) {
        if base_type != patch_type {
            bail!(
                "Type Conflict: Cannot merge patch with type '{}' into base with type '{}'",
                patch_type,
                base_type
            );
        }
    }

    // Inheritance: allow filling gaps; error on conflicting inherit targets.
    match (&base.inherits, &patch.inherits) {
        (None, Some(parent)) => base.inherits = Some(parent.clone()),
        (Some(existing), Some(incoming)) if existing != incoming => {
            bail!(
                "Inheritance Conflict: param inherits '{}' but patch specifies '{}'",
                existing,
                incoming
            );
        }
        _ => {}
    }

    // Overwrite fields if the patch has them
    if let Some(v) = patch.value {
        base.value = Some(v);
    }
    if let Some(v) = patch.r#type {
        base.r#type = Some(v);
    }
    if let Some(v) = patch.unit {
        base.unit = Some(v);
    }
    if let Some(v) = patch.safety {
        base.safety = Some(v);
    }
    if let Some(v) = patch.lifecycle {
        base.lifecycle = Some(v);
    }
    if let Some(v) = patch.access {
        base.access = Some(v);
    }
    if let Some(v) = patch.limits {
        base.limits = Some(v);
    }
    if let Some(v) = patch.req_id {
        base.req_id = Some(v);
    }
    if let Some(v) = patch.doc {
        base.doc = Some(v);
    }

    // Merge overrides by appending; they will be evaluated later.
    if !patch.overrides.is_empty() {
        base.overrides.extend(patch.overrides);
    }

    Ok(())
}

// ----------------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        Component, ConditionalBlock, Lifecycle, Parameter, Role, SafetyLevel, Value,
    };
    use std::collections::HashMap;

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

    fn param_with_type_value(r#type: &str, value: Value) -> Parameter {
        let mut param = empty_param();
        param.r#type = Some(r#type.to_string());
        param.value = Some(value);
        param
    }

    fn config_with_component(comp_name: &str, comp: Component) -> Config {
        let mut components = HashMap::new();
        components.insert(comp_name.to_string(), comp);
        Config {
            package: "test".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components,
            artifacts: HashMap::new(),
            facets: Default::default(),
        }
    }

    #[test]
    fn eval_accepts_single_quotes_and_complex_logic() {
        let mut tags = HashMap::new();
        tags.insert("variant".to_string(), "heavy".to_string());
        tags.insert("region".to_string(), "us".to_string());
        let ctx = ResolutionContext { tags };

        let result = ctx.eval("variant == 'heavy' && region != 'eu'").unwrap();
        assert!(result);
    }

    #[test]
    fn eval_errors_on_missing_tag() {
        let ctx = ResolutionContext {
            tags: HashMap::new(),
        };
        let err = ctx.eval("variant == 'heavy'").unwrap_err();
        assert!(
            format!("{err}").contains("Failed to evaluate condition"),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_applies_default_metadata() {
        let mut params = HashMap::new();
        params.insert(
            "speed".to_string(),
            param_with_type_value("float", Value::Float(1.0)),
        );

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };
        let config = config_with_component("motor", comp);

        let resolved = resolve(
            config,
            &ResolutionContext {
                tags: HashMap::new(),
            },
        )
        .unwrap();
        let param = &resolved.components["motor"].params["speed"];

        assert_eq!(param.safety, SafetyLevel::QM);
        assert_eq!(param.lifecycle, Lifecycle::Runtime);
        assert_eq!(param.access, Role::Technician);
    }

    #[test]
    fn resolve_errors_on_override_unit_conflict() {
        let mut param = param_with_type_value("float", Value::Float(1.0));
        param.unit = Some("m/s".to_string());
        param.overrides.push(ConditionalBlock {
            condition: "variant == 'heavy'".to_string(),
            payload: Box::new({
                let mut payload = empty_param();
                payload.unit = Some("km/h".to_string());
                payload
            }),
        });

        let mut params = HashMap::new();
        params.insert("speed".to_string(), param);

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };
        let config = config_with_component("motor", comp);

        let mut tags = HashMap::new();
        tags.insert("variant".to_string(), "heavy".to_string());
        let ctx = ResolutionContext { tags };

        let err = resolve(config, &ctx).unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("Unit Conflict")),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_errors_on_override_type_conflict() {
        let mut param = param_with_type_value("float", Value::Float(1.0));
        param.overrides.push(ConditionalBlock {
            condition: "variant == 'heavy'".to_string(),
            payload: Box::new({
                let mut payload = empty_param();
                payload.r#type = Some("string".to_string());
                payload.value = Some(Value::String("slow".to_string()));
                payload
            }),
        });

        let mut params = HashMap::new();
        params.insert("speed".to_string(), param);

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };
        let config = config_with_component("motor", comp);

        let mut tags = HashMap::new();
        tags.insert("variant".to_string(), "heavy".to_string());
        let ctx = ResolutionContext { tags };

        let err = resolve(config, &ctx).unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("Type Conflict")),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_errors_on_override_missing_tag() {
        let mut param = param_with_type_value("float", Value::Float(1.0));
        param.overrides.push(ConditionalBlock {
            condition: "variant == 'heavy'".to_string(),
            payload: Box::new({
                let mut payload = empty_param();
                payload.value = Some(Value::Float(2.0));
                payload
            }),
        });

        let mut params = HashMap::new();
        params.insert("speed".to_string(), param);

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };
        let config = config_with_component("motor", comp);
        let ctx = ResolutionContext {
            tags: HashMap::new(),
        };

        let err = resolve(config, &ctx).unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("Failed to evaluate condition")),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_does_not_inherit_definition_value() {
        let mut def = empty_param();
        def.r#type = Some("float".to_string());
        def.value = Some(Value::Float(2.0));

        let mut child = empty_param();
        child.inherits = Some("base_speed".to_string());

        let mut params = HashMap::new();
        params.insert("speed".to_string(), child);

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };

        let mut definitions = HashMap::new();
        definitions.insert("base_speed".to_string(), def);

        let mut components = HashMap::new();
        components.insert("motor".to_string(), comp);

        let config = Config {
            package: "test".to_string(),
            version: "1.0".to_string(),
            definitions,
            components,
            artifacts: HashMap::new(),
            facets: Default::default(),
        };

        let err = resolve(
            config,
            &ResolutionContext {
                tags: HashMap::new(),
            },
        )
        .unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("Missing 'value' for parameter")),
            "err: {err}"
        );
    }

    #[test]
    fn parse_scope_selectors_accepts_variants() {
        let selectors = parse_scope_selectors("//motor, platform:all, component:sensor").unwrap();
        assert_eq!(selectors.len(), 3);
        assert!(selectors.contains(&ScopeSelector::Component("motor".to_string())));
        assert!(selectors.contains(&ScopeSelector::PlatformAll));
        assert!(selectors.contains(&ScopeSelector::Component("sensor".to_string())));
    }

    #[test]
    fn parse_scope_selectors_rejects_invalid_ident() {
        let err = parse_scope_selectors("component:Bad_Name").unwrap_err();
        assert!(
            format!("{err}").contains("Scope identifier must be snake_case"),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_scoped_component_closure() {
        let mut defs = HashMap::new();
        let mut def = empty_param();
        def.r#type = Some("float".to_string());
        def.value = Some(Value::Float(1.0));
        defs.insert("speed".to_string(), def);

        let mut root_params = HashMap::new();
        root_params.insert(
            "speed".to_string(),
            param_with_type_value("float", Value::Float(1.0)),
        );

        let mut child_params = HashMap::new();
        child_params.insert(
            "speed".to_string(),
            param_with_type_value("float", Value::Float(2.0)),
        );

        let mut components = HashMap::new();
        components.insert(
            "root".to_string(),
            Component {
                r#type: Some("actuator".to_string()),
                condition: None,
                depends_on: vec!["child".to_string()],
                params: root_params,
            },
        );
        components.insert(
            "child".to_string(),
            Component {
                r#type: Some("sensor".to_string()),
                condition: None,
                depends_on: Vec::new(),
                params: child_params,
            },
        );
        components.insert(
            "unrelated".to_string(),
            Component {
                r#type: Some("sensor".to_string()),
                condition: None,
                depends_on: Vec::new(),
                params: HashMap::new(),
            },
        );

        let raw = Config {
            package: "pkg".to_string(),
            version: "1.0".to_string(),
            definitions: defs,
            components,
            artifacts: HashMap::new(),
            facets: Default::default(),
        };

        let ctx = ResolutionContext {
            tags: HashMap::new(),
        };

        let scoped = resolve_scoped(&raw, &ctx, "//root").unwrap();
        let resolved = scoped.get("root").expect("missing root");
        assert!(resolved.components.contains_key("root"));
        assert!(resolved.components.contains_key("child"));
        assert!(!resolved.components.contains_key("unrelated"));
    }

    #[test]
    fn resolve_scoped_platform_all() {
        let mut components = HashMap::new();
        components.insert(
            "platform_a".to_string(),
            Component {
                r#type: Some("platform".to_string()),
                condition: None,
                depends_on: Vec::new(),
                params: HashMap::new(),
            },
        );
        components.insert(
            "platform_b".to_string(),
            Component {
                r#type: Some("platform".to_string()),
                condition: None,
                depends_on: Vec::new(),
                params: HashMap::new(),
            },
        );
        components.insert(
            "sensor".to_string(),
            Component {
                r#type: Some("sensor".to_string()),
                condition: None,
                depends_on: Vec::new(),
                params: HashMap::new(),
            },
        );

        let raw = Config {
            package: "pkg".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components,
            artifacts: HashMap::new(),
            facets: Default::default(),
        };

        let ctx = ResolutionContext {
            tags: HashMap::new(),
        };

        let scoped = resolve_scoped(&raw, &ctx, "platform:all").unwrap();
        assert!(scoped.contains_key("platform_a"));
        assert!(scoped.contains_key("platform_b"));
    }

    #[test]
    fn resolve_artifact_parameter_ok() {
        let mut params = HashMap::new();
        params.insert(
            "driver".to_string(),
            param_with_type_value("artifact", Value::String("motor_driver".to_string())),
        );

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };

        let mut artifacts = HashMap::new();
        artifacts.insert(
            "motor_driver".to_string(),
            schema::Artifact {
                name: "motor_driver".to_string(),
                version: None,
                hash: None,
                source: Some("artifact://motor_driver".to_string()),
                target: None,
                doc: None,
            },
        );

        let config = Config {
            package: "test".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: {
                let mut map = HashMap::new();
                map.insert("motor".to_string(), comp);
                map
            },
            artifacts,
            facets: Default::default(),
        };

        let resolved = resolve(
            config,
            &ResolutionContext {
                tags: HashMap::new(),
            },
        )
        .unwrap();
        assert!(resolved.components["motor"].params.contains_key("driver"));
    }

    #[test]
    fn resolve_artifact_parameter_missing_id() {
        let mut params = HashMap::new();
        params.insert(
            "driver".to_string(),
            param_with_type_value("artifact", Value::String("missing".to_string())),
        );

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };

        let config = Config {
            package: "test".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: {
                let mut map = HashMap::new();
                map.insert("motor".to_string(), comp);
                map
            },
            artifacts: HashMap::new(),
            facets: Default::default(),
        };

        let err = resolve(
            config,
            &ResolutionContext {
                tags: HashMap::new(),
            },
        )
        .unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("Artifact 'missing' not found")),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_artifact_parameter_non_string() {
        let mut params = HashMap::new();
        params.insert(
            "driver".to_string(),
            param_with_type_value("artifact", Value::Integer(5)),
        );

        let comp = Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            params,
        };

        let config = Config {
            package: "test".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: {
                let mut map = HashMap::new();
                map.insert("motor".to_string(), comp);
                map
            },
            artifacts: HashMap::new(),
            facets: Default::default(),
        };

        let err = resolve(
            config,
            &ResolutionContext {
                tags: HashMap::new(),
            },
        )
        .unwrap_err();
        assert!(
            err.chain().any(|cause| cause
                .to_string()
                .contains("Artifact parameter value must be a string")),
            "err: {err}"
        );
    }
}
