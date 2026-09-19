// SPDX-License-Identifier: BUSL-1.1

//! Requirement delivery in the resolved snapshot (ADR-0057 §D7,
//! configflux-secb.6).
//!
//! The claim under test is the one the whole feature exists for: a service
//! reads its OWN configuration. Before this, a component that needed a shared
//! container had to know which catalogue component held it and read
//! `component.line_container.param.width_mm`; now the entry is delivered inside
//! the requiring component, at `components.<c>.requires.<slot>`.
//!
//! Three things have to hold, and the feature is worthless without any of them:
//!
//!   1. **It is delivered, per scope.** Scoping to one service must still fill
//!      that service's block — a requirement is not a `depends_on` edge, so the
//!      binding's catalogue is NOT pulled into the dependency closure and a
//!      scoped slice that dropped the catalogue would silently emit nothing.
//!   2. **It costs nothing when unused.** A component with no requirement must
//!      serialize no `requires` key at all, because `resolved_output` is the
//!      largest member of the `resolve_hash` pre-image and an always-present
//!      empty map would rotate every snapshot in existence.
//!   3. **An undecided binding fails closed, by name.** A binding a requirement
//!      needs but nothing bound is `E_RESOLVE_FACET_UNBOUND`, and the message
//!      names every `<component>.<slot>` that was waiting for it — otherwise the
//!      integrator fixes one site and rediscovers the next on the following run.
//!
//! The pack is `s_requires_delivery`: three authoring units (ADR-0057 §D1) —
//! the plant's catalogue and site facet, the line's binding, and the services.
//! `factory_c` is deliberately uncovered by the derive table, which is what
//! makes claim 3 reachable without inventing a malformed model.

use crate::loader_api::{
    canonical_selection_state, open_model, resolve_from_selection, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest, SelectionState, E_RESOLVE_FACET_UNBOUND,
};
use crate::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};
use crate::resolved_models::ResolvedConfig;
use crate::scenario_test_support::unique_temp_path;
use crate::{ir, Compiler};
use anyhow::{Context, Result};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CATALOGUE_SOURCE: &str = "scenarios/s_requires_delivery/00_catalogue.json";
const BINDINGS_SOURCE: &str = "scenarios/s_requires_delivery/10_bindings.json";
const SERVICES_SOURCE: &str = "scenarios/s_requires_delivery/20_services.json";

const CATALOGUE_CHUNK: &str = include_str!("../scenarios/s_requires_delivery/00_catalogue.json");
const BINDINGS_CHUNK: &str = include_str!("../scenarios/s_requires_delivery/10_bindings.json");
const SERVICES_CHUNK: &str = include_str!("../scenarios/s_requires_delivery/20_services.json");

/// The three chunks, as the compiler sees them.
fn chunks() -> Vec<(&'static str, &'static str)> {
    vec![
        (CATALOGUE_SOURCE, CATALOGUE_CHUNK),
        (BINDINGS_SOURCE, BINDINGS_CHUNK),
        (SERVICES_SOURCE, SERVICES_CHUNK),
    ]
}

fn emitted_cmp_dir(label: &str) -> Result<PathBuf> {
    let temp_dir = unique_temp_path("cfx-requires", label);
    std::fs::create_dir_all(&temp_dir)
        .with_context(|| format!("Failed to create temp dir '{}'", temp_dir.display()))?;

    let mut compiler = Compiler::new();
    for (source_id, chunk) in chunks() {
        compiler
            .add_chunk_auto(source_id, chunk)
            .with_context(|| format!("Failed to add chunk '{}'", source_id))?;
    }
    compiler.emit_ir(&temp_dir)?;
    Ok(temp_dir)
}

fn open_handle(cmp_dir: &Path) -> Result<ModelHandle> {
    let manifest_path = cmp_dir.join(ir::CMP_DEFAULT_MANIFEST_FILENAME);
    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });
    if result.status != OperationStatus::Ok {
        anyhow::bail!("open_model failed: {:?}", result.diagnostics.diagnostics);
    }
    result.model_handle.context("missing model_handle")
}

fn selection(model_handle: &ModelHandle, scope: &str, site: &str) -> Result<SelectionState> {
    let mut choices = BTreeMap::new();
    choices.insert("site".to_string(), site.to_string());
    canonical_selection_state(
        model_handle.model_hash.clone(),
        scope.to_string(),
        BTreeMap::new(),
        choices,
    )
}

/// Resolve one scope. `implied` is what `session_compose::resolve` would have
/// inferred from the derive table (ADR-0057 §D6) and hands the compiler; the
/// compiler itself stays solver-free (ADR-0003 §2), so the test supplies it the
/// same way the shared resolve wrapper does.
fn resolve_scope(
    model_handle: &ModelHandle,
    scope: &str,
    site: &str,
    implied: &[(&str, &str)],
) -> Result<crate::loader_api::ResolveResult> {
    let state = selection(model_handle, scope, site)?;
    let implied_choices: BTreeMap<String, String> = implied
        .iter()
        .map(|(facet, option)| (facet.to_string(), option.to_string()))
        .collect();
    Ok(resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: model_handle.clone(),
        scope: scope.to_string(),
        selection_state: state,
        implied_choices,
    }))
}

fn scoped_config(result: &crate::loader_api::ResolveResult, scope_root: &str) -> ResolvedConfig {
    let payload = result
        .resolved_output
        .clone()
        .expect("a successful resolve carries resolved_output");
    let scoped: BTreeMap<String, ResolvedConfig> = serde_json::from_value(payload)
        .expect("resolved_output decodes into the resolved model");
    scoped
        .get(scope_root)
        .unwrap_or_else(|| panic!("resolved_output carries no scope root '{scope_root}'"))
        .clone()
}

/// The c1 entry, as the catalogue declares it.
fn c1_fields() -> serde_json::Value {
    json!({"height_mm": 1000, "length_mm": 1200, "width_mm": 800})
}

#[test]
fn scoped_resolve_delivers_the_entry_inside_the_requiring_component() -> Result<()> {
    let cmp_dir = emitted_cmp_dir("scoped")?;
    let handle = open_handle(&cmp_dir)?;

    for service in ["vision_service", "compute_service"] {
        let scope = format!("component:{service}");
        let result = resolve_scope(&handle, &scope, "factory_a", &[("line_container", "c1")])?;
        assert_eq!(
            result.status,
            OperationStatus::Ok,
            "{service}: {:?}",
            result.diagnostics.diagnostics
        );

        let config = scoped_config(&result, service);
        let component = config
            .components
            .get(service)
            .unwrap_or_else(|| panic!("{service} missing from its own scope"));
        let requirement = component
            .requires
            .get("container")
            .unwrap_or_else(|| panic!("{service} carries no requires.container block"));

        assert_eq!(requirement.binding, "line_container");
        assert_eq!(requirement.entry, "c1");
        assert_eq!(
            serde_json::to_value(&requirement.fields)?,
            c1_fields(),
            "{service} received the wrong catalogue entry values"
        );
    }

    std::fs::remove_dir_all(&cmp_dir).ok();
    Ok(())
}

#[test]
fn scope_all_delivers_every_requiring_component() -> Result<()> {
    let cmp_dir = emitted_cmp_dir("all")?;
    let handle = open_handle(&cmp_dir)?;

    let result = resolve_scope(&handle, "all", "factory_a", &[("line_container", "c1")])?;
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "{:?}",
        result.diagnostics.diagnostics
    );

    let config = scoped_config(&result, "all");
    for service in ["vision_service", "compute_service"] {
        let requirement = config
            .components
            .get(service)
            .and_then(|component| component.requires.get("container"))
            .unwrap_or_else(|| panic!("scope all: {service} carries no requires.container block"));
        assert_eq!(requirement.entry, "c1");
        assert_eq!(serde_json::to_value(&requirement.fields)?, c1_fields());
    }

    std::fs::remove_dir_all(&cmp_dir).ok();
    Ok(())
}

#[test]
fn the_other_entry_is_delivered_when_the_site_selects_it() -> Result<()> {
    let cmp_dir = emitted_cmp_dir("factory-b")?;
    let handle = open_handle(&cmp_dir)?;

    let result = resolve_scope(
        &handle,
        "component:vision_service",
        "factory_b",
        &[("line_container", "c2")],
    )?;
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "{:?}",
        result.diagnostics.diagnostics
    );

    let requirement = scoped_config(&result, "vision_service")
        .components
        .get("vision_service")
        .and_then(|component| component.requires.get("container"))
        .cloned()
        .context("vision_service carries no requires.container block")?;
    assert_eq!(requirement.entry, "c2");
    assert_eq!(
        serde_json::to_value(&requirement.fields)?,
        json!({"height_mm": 700, "length_mm": 800, "width_mm": 600})
    );

    std::fs::remove_dir_all(&cmp_dir).ok();
    Ok(())
}

#[test]
fn a_requirement_free_component_serializes_no_requires_key() -> Result<()> {
    // The skip-if-empty guarantee, asserted on the WIRE rather than on the Rust
    // value: an empty map that still serialized would rotate the resolve_hash of
    // every model that never uses this feature.
    let cmp_dir = emitted_cmp_dir("skip-empty")?;
    let handle = open_handle(&cmp_dir)?;
    let result = resolve_scope(&handle, "all", "factory_a", &[("line_container", "c1")])?;

    let payload = result
        .resolved_output
        .clone()
        .context("a successful resolve carries resolved_output")?;
    let requires_keys = payload["all"]["components"]
        .as_object()
        .context("components is not an object")?
        .values()
        .filter(|component| component.get("requires").is_some())
        .count();
    assert_eq!(
        requires_keys, 2,
        "exactly the two requiring components may carry a requires key"
    );

    // And the negative case, from a pack with no requirement at all.
    let s1_dir = unique_temp_path("cfx-requires", "s1");
    std::fs::create_dir_all(&s1_dir)?;
    let mut compiler = Compiler::new();
    compiler.add_chunk_auto(
        "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml",
        include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json"),
    )?;
    compiler.add_chunk_auto(
        "scenarios/s1_water_pump/smoke/chunks/10_components.toml",
        include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json"),
    )?;
    compiler.emit_ir(&s1_dir)?;
    let s1_handle = open_handle(&s1_dir)?;
    let mut context = BTreeMap::new();
    for (tag, value) in [
        ("cooling_brand", "hydra"),
        ("cooling_model", "x200"),
        ("pump_type", "dual"),
        ("region", "us"),
    ] {
        context.insert(tag.to_string(), value.to_string());
    }
    let state = canonical_selection_state(
        s1_handle.model_hash.clone(),
        "component:thermal_control".to_string(),
        context,
        BTreeMap::new(),
    )?;
    let s1_result = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: s1_handle,
        scope: "component:thermal_control".to_string(),
        selection_state: state,
        implied_choices: Default::default(),
    });
    assert_eq!(s1_result.status, OperationStatus::Ok);
    let s1_payload = s1_result
        .resolved_output
        .context("a successful resolve carries resolved_output")?;
    assert!(
        !serde_json::to_string(&s1_payload)?.contains("\"requires\""),
        "a model with no requirement must not emit a requires key anywhere"
    );

    std::fs::remove_dir_all(&cmp_dir).ok();
    std::fs::remove_dir_all(&s1_dir).ok();
    Ok(())
}

#[test]
fn an_unbound_binding_names_every_requiring_component() -> Result<()> {
    // factory_c is outside the derive table, so nothing decides the container
    // and nothing defaults it. Both services are waiting on it, and the message
    // has to say so — reporting one at a time turns a single authoring mistake
    // into a sequence of failed deployments.
    let cmp_dir = emitted_cmp_dir("unbound")?;
    let handle = open_handle(&cmp_dir)?;

    let result = resolve_scope(&handle, "all", "factory_c", &[])?;
    assert_eq!(result.status, OperationStatus::Error);

    let diagnostic = result
        .diagnostics
        .diagnostics
        .first()
        .context("resolve failed with no diagnostic")?;
    assert_eq!(diagnostic.code, E_RESOLVE_FACET_UNBOUND);
    assert!(
        diagnostic.message.contains("line_container"),
        "message must name the binding: {}",
        diagnostic.message
    );
    assert!(
        diagnostic
            .message
            .contains("required by compute_service.container, vision_service.container"),
        "message must list every requiring component, sorted: {}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("[c1, c2]"),
        "message must name the binding's declared domain: {}",
        diagnostic.message
    );
    assert!(
        result.resolved_output.is_none(),
        "a rejected resolve must emit no snapshot"
    );

    std::fs::remove_dir_all(&cmp_dir).ok();
    Ok(())
}

#[test]
fn a_stale_schema_version_is_rejected_naming_the_current_one() -> Result<()> {
    // ADR-0057 §D7: no compatibility mode. A request authored against 4 is
    // refused before any payload is read, and the refusal names 5 so the caller
    // can fix it without reading the release notes.
    let cmp_dir = emitted_cmp_dir("schema-version")?;
    let handle = open_handle(&cmp_dir)?;
    let state = selection(&handle, "all", "factory_a")?;

    let result = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION - 1,
        model_handle: handle,
        scope: "all".to_string(),
        selection_state: state,
        implied_choices: Default::default(),
    });
    assert_eq!(result.status, OperationStatus::Error);
    let diagnostic = result
        .diagnostics
        .diagnostics
        .first()
        .context("rejection carried no diagnostic")?;
    assert_eq!(
        diagnostic.code,
        crate::loader_api::E_LOADER_UNSUPPORTED_SCHEMA_VERSION
    );
    assert!(
        diagnostic.message.contains("expected 5"),
        "the rejection must name the current version: {}",
        diagnostic.message
    );

    std::fs::remove_dir_all(&cmp_dir).ok();
    Ok(())
}

#[test]
fn inspect_component_reports_its_requirements() -> Result<()> {
    // ADR-0057 §D7: `configflux-compiler inspect component <id>` is where an
    // author checks what a component declares, so a requirement that does not
    // appear there is invisible until a resolve fails.
    use crate::product_api::{
        inspect_model, InspectModelRequest, InspectQuery, InspectionItem, SourceManifestEntry,
    };

    let result = inspect_model(InspectModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: chunks()
            .into_iter()
            .map(|(source_id, content)| SourceManifestEntry {
                source_id: source_id.to_string(),
                inline_content: content.to_string(),
            })
            .collect(),
        query: InspectQuery::Component {
            component_id: "compute_service".to_string(),
        },
    });
    assert_eq!(result.status, OperationStatus::Ok);

    match result.item {
        Some(InspectionItem::Component { requires, .. }) => {
            let requirement = requires
                .get("container")
                .context("inspect reported no container requirement")?;
            assert_eq!(requirement.binding, "line_container");
            assert_eq!(
                requirement.accepts.as_deref(),
                Some(["c1".to_string(), "c2".to_string()].as_slice())
            );
        }
        other => panic!("expected a component item, got {other:?}"),
    }
    Ok(())
}

#[test]
fn a_second_undecided_binding_is_named_in_the_same_message() {
    // The accumulator collects every unbound binding, and the message says so:
    // one binding is reported in full (every site, and its domain once the
    // loader's mapper has run) and the rest are named after it. Reporting only
    // the first would send the integrator round the loop once per binding.
    //
    // Built as an in-memory `Config` rather than a scenario pack on purpose: a
    // second binding in the committed pack would rotate its model_hash and every
    // fixture derived from it, to pin a message shape that needs no package at
    // all.
    use crate::resolver::{resolve_scoped, ResolutionContext};
    use crate::schema::{
        Binding, Catalogue, CatalogueField, CatalogueFieldType, Component, Config, Requirement,
    };
    use std::collections::HashMap;

    fn catalogue(field: &str) -> Catalogue {
        let mut fields = BTreeMap::new();
        fields.insert(
            field.to_string(),
            CatalogueField {
                r#type: CatalogueFieldType::Integer,
                unit: None,
                doc: None,
            },
        );
        let mut entry = BTreeMap::new();
        entry.insert(field.to_string(), crate::schema::Value::Integer(1));
        let mut entries = BTreeMap::new();
        entries.insert("e1".to_string(), entry);
        Catalogue {
            fields,
            entries,
            doc: None,
        }
    }

    fn requiring(slot: &str, binding: &str) -> Component {
        let mut requires = BTreeMap::new();
        requires.insert(
            slot.to_string(),
            Requirement {
                binding: binding.to_string(),
                accepts: None,
            },
        );
        Component {
            r#type: Some("service".to_string()),
            condition: None,
            depends_on: Vec::new(),
            requires,
            params: HashMap::new(),
        }
    }

    let mut catalogues = HashMap::new();
    catalogues.insert("cat_a".to_string(), catalogue("width_mm"));
    catalogues.insert("cat_b".to_string(), catalogue("height_mm"));

    let mut bindings = HashMap::new();
    for (id, cat) in [("alpha_binding", "cat_a"), ("beta_binding", "cat_b")] {
        bindings.insert(
            id.to_string(),
            Binding {
                catalogue: cat.to_string(),
                default: None,
                derive: None,
                doc: None,
            },
        );
    }

    let mut components = HashMap::new();
    components.insert("one_service".to_string(), requiring("slot_a", "alpha_binding"));
    components.insert("two_service".to_string(), requiring("slot_b", "beta_binding"));

    let config = Config {
        package: "multi".to_string(),
        version: "1.0.0".to_string(),
        definitions: HashMap::new(),
        components,
        artifacts: HashMap::new(),
        facets: HashMap::new(),
        constraints: HashMap::new(),
        catalogues,
        bindings,
    };

    let context = ResolutionContext {
        tags: HashMap::new(),
    };
    let err = resolve_scoped(&config, &context, "all")
        .expect_err("two undecided bindings must fail the resolve");
    let message = format!("{err:#}");

    // Binding-id-ascending, so the message is byte-stable whatever order the
    // component HashMap walked in.
    assert!(
        message.contains("'alpha_binding' is unbound; required by one_service.slot_a"),
        "the first binding must be reported in full: {message}"
    );
    assert!(
        message.contains("also unbound: beta_binding"),
        "the second binding must be named in the same message: {message}"
    );
}
