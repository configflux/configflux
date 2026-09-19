// SPDX-License-Identifier: BUSL-1.1

//! The interface a chunk presents to the rest of the compile set.
//!
//! **Why this type exists** (ADR-0057 §D9, and the one design rule the
//! operator attached to "model before objects"). Every link/verify check
//! ADR-0057 introduces — a binding's catalogue existing, a `derive` source
//! being declared, duplicate catalogue and binding ids, and in
//! configflux-secb.5 requirement resolution, `accepts` validation and the
//! empty-intersection check — is written as a function over an in-memory
//! [`InterfaceSummary`], merged across the compile set into a
//! [`MergedSummary`].
//!
//! ADR-0058's **object header is this summary serialized**. When separate
//! compilation lands, the linker reads headers instead of chunks and calls the
//! same functions over the same [`MergedSummary`]. Nothing is written twice,
//! and no check has to be ported from "sees the whole model" to "sees only
//! headers" after the fact — which is the migration that would otherwise have
//! made the objects epic expensive.
//!
//! The summary is therefore deliberately **header-shaped**: it carries what a
//! linker needs to resolve a cross-unit reference (who exports what, who
//! imports what, a catalogue's entry roster, a binding's outward links, and the
//! authored clauses) and nothing that requires reading a chunk's body. Catalogue
//! *shape* validation — field types, entry completeness — is a body check and
//! deliberately lives in [`crate::link_verify::validate_catalogues`] instead.

use crate::conditions::{
    for_each_condition_operand, parse_condition_expr, ConditionOperand,
};
use crate::ir::IrChunk;
use crate::schema::{Binding, Catalogue, Component, Config, Constraint, Facet, Parameter};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The entity ids a chunk declares, by namespace.
///
/// A `BTreeSet` per namespace: an id is declared or it is not, and the merge
/// below is what turns "declared twice" into a diagnostic naming both sources.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exports {
    pub definitions: BTreeSet<String>,
    pub components: BTreeSet<String>,
    pub artifacts: BTreeSet<String>,
    pub facets: BTreeSet<String>,
    pub bindings: BTreeSet<String>,
    pub catalogues: BTreeSet<String>,
    pub constraints: BTreeSet<String>,
}

/// The entity ids a chunk *references*, by namespace, each mapped to the
/// authored sites that reference it.
///
/// The importer set is what lets a link diagnostic name the offender rather
/// than only the missing target — "binding 'line_container' names unknown
/// catalogue 'containers'" instead of "unknown catalogue 'containers'".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Imports {
    /// `depends_on` target -> the components that depend on it.
    pub components: BTreeMap<String, BTreeSet<String>>,
    /// `inherits` target -> the authored parameter paths that inherit it.
    pub definitions: BTreeMap<String, BTreeSet<String>>,
    /// Facet (or binding) name -> the conditions, constraints and `derive`
    /// tables that name it. Deliberately NOT checked against `exports.facets`:
    /// an undeclared facet referenced by a selector `condition` keeps its
    /// legacy condition-inferred domain (ADR-0047 §3), so this map is header
    /// information rather than a link obligation.
    pub facets: BTreeMap<String, BTreeSet<String>>,
    /// Binding name -> the component requirements that need it, as
    /// `components.<id>.requires.<slot>` paths (ADR-0057 §D4). Like
    /// `catalogues`, this one IS a link obligation: a requirement naming an
    /// undeclared binding is `E_REQUIRES_INVALID`.
    pub bindings: BTreeMap<String, BTreeSet<String>>,
    /// Catalogue name -> the bindings whose domain it is. Unlike `facets`,
    /// this one IS a link obligation: a binding without its catalogue has no
    /// value domain at all.
    pub catalogues: BTreeMap<String, BTreeSet<String>>,
}

/// One authored clause in canonical, id-addressed form.
///
/// Today this is exactly the `constraints` namespace — the model's only
/// authored root conjuncts (ADR-0054 §5.1). The `derive` and `accepts`
/// lowerings join it in configflux-secb.5, which is why the type is a list of
/// `(id, condition)` rather than a borrowed view of `Config::constraints`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clause {
    pub id: String,
    pub condition: String,
}

/// A binding's outward links, as the object header will carry them.
///
/// This is the whole of what a linker needs to resolve a binding without
/// reading the declaring chunk: which catalogue supplies its domain, which
/// entry it defaults to, and which declared facet (if any) derives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingLink {
    pub catalogue: String,
    pub default: Option<String>,
    /// The single `derive` source (a declared facet or binding). `None` when
    /// the binding declares no `derive` table.
    pub derive_source: Option<String>,
    /// `(source_value, entry_id)` pairs in source-value-ascending order. Empty
    /// when `derive_source` is `None`.
    pub derive_pairs: Vec<(String, String)>,
    /// How many sources the authored `derive` table named. v1 admits exactly
    /// one; the count is carried so the validator can name the real fault
    /// instead of silently reading the first.
    pub derive_source_count: usize,
}

/// One component requirement, as the object header will carry it
/// (ADR-0057 §D4).
///
/// Header-shaped for the same reason [`BindingLink`] is: everything
/// `link_verify::validate_requirements` needs to resolve a requirement without
/// reading the declaring chunk's body. The component's `condition` rides along
/// because the `accepts` lowering is GUARDED by it (ADR-0054 §3 — a component
/// that is not included asserts nothing), so the conjunct cannot be built from
/// the requirement alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementLink {
    pub component: String,
    /// The authored slot key. Unique per component (the authored form is a
    /// map), so `(component, slot)` is the requirement's primary key and is
    /// what the `accepts:` attribution id is built from.
    pub slot: String,
    pub binding: String,
    /// The accepted entry ids in AUTHORED order — the order the lowered
    /// `any_of(...)` commits to. `None` means every entry of the binding's
    /// catalogue is acceptable.
    pub accepts: Option<Vec<String>>,
    /// The declaring component's inclusion `condition`, verbatim.
    pub condition: Option<String>,
}

/// One chunk's interface. See the module docs for why this type exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSummary {
    /// The authoring unit this chunk belongs to — the chunk's `package` field
    /// (ADR-0057 §D1). Empty when summarizing an emitted IR chunk, which does
    /// not carry `package`; no check in this ADR reads it, and ADR-0058's
    /// object header is where it becomes load-bearing.
    pub unit: String,
    pub source_id: String,
    pub exports: Exports,
    pub imports: Imports,
    /// Declared facet -> its declared value domain, in declared order. Bindings
    /// are absent here: a binding's domain lives in its catalogue, which may be
    /// in another chunk, so it is resolved at merge time.
    pub facet_domains: BTreeMap<String, Vec<String>>,
    /// Of [`Self::facet_domains`], the facets declared `open: true`.
    ///
    /// Carried beside the domain rather than folded into it because the two
    /// answer different questions and only one of them is a domain. It is not
    /// decoration: ADR-0054 §5.2's cardinality channel emits
    /// `exactly_one_of(...)` for a CLOSED facet and pairwise at-most-one for an
    /// OPEN one, so a linker that could not tell them apart would assert
    /// at-least-one over an extensible domain — unsound, and a silently
    /// different `.ccm` from the one `compile` produces for the same model.
    /// A binding is never in here: a binding IS a closed facet (ADR-0057 §D3).
    pub open_facets: BTreeSet<String>,
    /// Catalogue -> its entry ids in id-ascending order (the order a binding's
    /// value domain commits to; see [`crate::schema::Catalogue`]).
    pub catalogue_entries: BTreeMap<String, Vec<String>>,
    pub binding_links: BTreeMap<String, BindingLink>,
    /// Component requirements, component-then-slot ascending (ADR-0057 §D4).
    pub requirements: Vec<RequirementLink>,
    pub clauses: Vec<Clause>,
    /// The chunk's inclusion SELECTORS: every component activation `condition`
    /// and every parameter-override branch `condition`, keyed by the authored
    /// path that carries it, in the canonical in-chunk walk order (ADR-0058
    /// §A2) — definitions by id, then components by id, and within each the
    /// override chain in authored order.
    ///
    /// Separate from [`Self::clauses`] because the two are different channels,
    /// not two spellings of one (ADR-0054 §5.1): a constraint is an ASSERTED
    /// root conjunct, while a selector asserts nothing and contributes only the
    /// `(facet, value)` symbols it names. They are also ordered differently —
    /// clauses id-ascending, selectors in walk order — and the walk order is
    /// what the `.ccm` clause channel commits to.
    ///
    /// This is the field that makes ADR-0058 §D4 stage 2 possible at all: the
    /// linker builds the constraint model from HEADERS, and without the
    /// selectors a header cannot say which symbols the unit introduces. §D2
    /// listed them ("conditions with owner path"); the §A1 reconciliation
    /// shrank the sketch to the ADR-0057 §D9 shapes and dropped them, and
    /// stage 2 cannot be header-only without them.
    ///
    /// The `id` is an ENTITY path (`components.<id>`,
    /// `definitions.<id>.overrides[0]`), never a file path, so it may enter the
    /// object header without breaking ADR-0056's no-path-in-an-identity rule.
    pub selectors: Vec<Clause>,
}

/// Every id a namespace declares across the compile set, mapped to the
/// `source_id`s that declared it. A vector of length > 1 is a duplicate.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergedExports {
    pub definitions: BTreeMap<String, Vec<String>>,
    pub components: BTreeMap<String, Vec<String>>,
    pub artifacts: BTreeMap<String, Vec<String>>,
    pub facets: BTreeMap<String, Vec<String>>,
    pub bindings: BTreeMap<String, Vec<String>>,
    pub catalogues: BTreeMap<String, Vec<String>>,
    pub constraints: BTreeMap<String, Vec<String>>,
}

/// The compile set's interfaces, merged. Every ADR-0057 link check is a
/// function of this value alone.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergedSummary {
    /// Every distinct `package` value in the compile set (ADR-0057 §D1). This
    /// ADR adds no agreement check — grouping by value IS the definition of a
    /// unit — and ADR-0058 checks agreement per `compile-object` call.
    pub units: BTreeSet<String>,
    pub exports: MergedExports,
    pub imports: Imports,
    pub facet_domains: BTreeMap<String, Vec<String>>,
    /// Of [`Self::facet_domains`], the facets declared `open: true`. See the
    /// field of the same name on [`InterfaceSummary`].
    pub open_facets: BTreeSet<String>,
    pub catalogue_entries: BTreeMap<String, Vec<String>>,
    pub binding_links: BTreeMap<String, BindingLink>,
    /// Every chunk's requirements, in a deterministic order: chunk visit order,
    /// then component-then-slot ascending within a chunk. Sorted once at the
    /// end of [`merge`] so the order is a property of the compile SET, not of
    /// the order its chunks happened to arrive in.
    pub requirements: Vec<RequirementLink>,
    pub clauses: Vec<Clause>,
    /// Every chunk's selectors, concatenated in the order the summaries were
    /// given and NOT re-sorted. The caller owns that order because it IS the
    /// canonical clause order of ADR-0058 §A2 — objects by unit name ascending,
    /// then chunks by `chunk_hash` ascending — and a sort here would erase it.
    pub selectors: Vec<Clause>,
}

impl MergedSummary {
    /// The effective value domain of a declared facet OR binding.
    ///
    /// A binding is a declared closed facet whose values are its catalogue's
    /// entries (ADR-0057 §D3), so a `derive` source may be either kind and this
    /// is the one lookup that answers for both.
    pub fn declared_domain(&self, name: &str) -> Option<&[String]> {
        if let Some(values) = self.facet_domains.get(name) {
            return Some(values.as_slice());
        }
        let link = self.binding_links.get(name)?;
        self.catalogue_entries
            .get(&link.catalogue)
            .map(Vec::as_slice)
    }
}

/// Summarize one authored chunk.
pub fn summarize(config: &Config, source_id: &str) -> InterfaceSummary {
    let mut summary = InterfaceSummary {
        unit: config.package.clone(),
        source_id: source_id.to_string(),
        exports: Exports::default(),
        imports: Imports::default(),
        facet_domains: BTreeMap::new(),
        open_facets: BTreeSet::new(),
        catalogue_entries: BTreeMap::new(),
        binding_links: BTreeMap::new(),
        requirements: Vec::new(),
        clauses: Vec::new(),
        selectors: Vec::new(),
    };

    // Definitions first, then components, each id-ascending: the canonical
    // in-chunk walk ADR-0058 §A2 fixes, and the order `selectors` commits to.
    // Both namespaces are `HashMap`s, so an unsorted walk would make the
    // selector order a function of a hash seed — which is exactly the class of
    // defect ADR-0005 §10 G1 rules out for anything that reaches emitted bytes.
    let mut definition_ids: Vec<&String> = config.definitions.keys().collect();
    definition_ids.sort();
    for id in definition_ids {
        summary.exports.definitions.insert(id.clone());
        collect_parameter_links(&config.definitions[id], &format!("definitions.{id}"), &mut summary);
    }
    for id in config.artifacts.keys() {
        summary.exports.artifacts.insert(id.clone());
    }
    let mut component_ids: Vec<&String> = config.components.keys().collect();
    component_ids.sort();
    for id in component_ids {
        summary.exports.components.insert(id.clone());
        collect_component_links(id, &config.components[id], &mut summary);
    }
    for (id, facet) in &config.facets {
        summary.exports.facets.insert(id.clone());
        summary.facet_domains.insert(id.clone(), facet.values.clone());
        if facet.open {
            summary.open_facets.insert(id.clone());
        }
    }
    for (id, constraint) in &config.constraints {
        summary.exports.constraints.insert(id.clone());
        collect_constraint_links(id, constraint, &mut summary);
    }
    for (id, catalogue) in &config.catalogues {
        summary.exports.catalogues.insert(id.clone());
        summary
            .catalogue_entries
            .insert(id.clone(), catalogue.entries.keys().cloned().collect());
    }
    for (id, binding) in &config.bindings {
        summary.exports.bindings.insert(id.clone());
        collect_binding_links(id, binding, &mut summary);
    }

    // `config.components` is a `HashMap`, so the requirement walk order is
    // seed-dependent until it is sorted. `(component, slot)` is the
    // requirement's primary key, and this is the order the `accepts:` conjuncts
    // are folded into the CCM root in — so it must be a property of the model,
    // never of a hash seed.
    summary
        .requirements
        .sort_by(|a, b| a.component.cmp(&b.component).then_with(|| a.slot.cmp(&b.slot)));
    summary.clauses.sort_by(|a, b| a.id.cmp(&b.id));
    summary
}

/// Summarize an emitted IR chunk, so the re-verification path
/// ([`crate::verify_ir_dir`]) can run the summary-driven link checks over the
/// same type the compile path builds from an authored `Config`.
///
/// `unit` is empty: `IrChunk` does not carry the authored `package` field, and
/// nothing in ADR-0057 reads the unit name. ADR-0058's object header is the
/// change that puts it in the package.
pub fn summarize_ir_chunk(chunk: &IrChunk) -> InterfaceSummary {
    let config = Config {
        package: String::new(),
        version: String::new(),
        definitions: chunk
            .definitions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        components: chunk
            .components
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        artifacts: chunk
            .artifacts
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        facets: chunk
            .facets
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        constraints: chunk
            .constraints
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        catalogues: chunk
            .catalogues
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        bindings: chunk
            .bindings
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    summarize(&config, &chunk.source_id)
}

/// Merge the compile set's summaries. Total: duplicates are recorded rather
/// than rejected, so the caller decides which diagnostic to raise (and so the
/// merge itself stays usable by a linker that wants to report all of them).
pub fn merge(summaries: &[InterfaceSummary]) -> MergedSummary {
    let mut merged = MergedSummary::default();

    for summary in summaries {
        if !summary.unit.is_empty() {
            merged.units.insert(summary.unit.clone());
        }
        merge_exports(
            &summary.exports.definitions,
            &summary.source_id,
            &mut merged.exports.definitions,
        );
        merge_exports(
            &summary.exports.components,
            &summary.source_id,
            &mut merged.exports.components,
        );
        merge_exports(
            &summary.exports.artifacts,
            &summary.source_id,
            &mut merged.exports.artifacts,
        );
        merge_exports(
            &summary.exports.facets,
            &summary.source_id,
            &mut merged.exports.facets,
        );
        merge_exports(
            &summary.exports.bindings,
            &summary.source_id,
            &mut merged.exports.bindings,
        );
        merge_exports(
            &summary.exports.catalogues,
            &summary.source_id,
            &mut merged.exports.catalogues,
        );
        merge_exports(
            &summary.exports.constraints,
            &summary.source_id,
            &mut merged.exports.constraints,
        );

        merge_imports(&summary.imports.components, &mut merged.imports.components);
        merge_imports(
            &summary.imports.definitions,
            &mut merged.imports.definitions,
        );
        merge_imports(&summary.imports.facets, &mut merged.imports.facets);
        merge_imports(&summary.imports.bindings, &mut merged.imports.bindings);
        merge_imports(&summary.imports.catalogues, &mut merged.imports.catalogues);

        for (name, values) in &summary.facet_domains {
            merged
                .facet_domains
                .entry(name.clone())
                .or_insert_with(|| values.clone());
        }
        merged.open_facets.extend(summary.open_facets.iter().cloned());
        for (name, entries) in &summary.catalogue_entries {
            merged
                .catalogue_entries
                .entry(name.clone())
                .or_insert_with(|| entries.clone());
        }
        for (name, link) in &summary.binding_links {
            merged
                .binding_links
                .entry(name.clone())
                .or_insert_with(|| link.clone());
        }
        merged
            .requirements
            .extend(summary.requirements.iter().cloned());
        merged.clauses.extend(summary.clauses.iter().cloned());
        // Concatenated, never sorted: the caller's summary order IS the
        // canonical clause order (see the field's doc).
        merged.selectors.extend(summary.selectors.iter().cloned());
    }

    merged
        .requirements
        .sort_by(|a, b| a.component.cmp(&b.component).then_with(|| a.slot.cmp(&b.slot)));
    merged.clauses.sort_by(|a, b| a.id.cmp(&b.id));
    merged
}

/// Project every binding onto the closed facet it IS (ADR-0057 §D3): domain =
/// the catalogue's entry ids in id-ascending order, `default` = the binding's
/// declared default, `open` = false.
///
/// This is the single place the projection is written. Every downstream reader
/// — the CCM's symbol and cardinality synthesis, the selection model's facet
/// domains, the resolve model's declared defaults — goes through it, which is
/// what makes "nothing downstream special-cases a binding" true by construction
/// rather than by four consistent implementations.
///
/// A binding whose catalogue is missing is skipped: that is
/// `E_BINDING_INVALID`, raised at link time, and this projection runs on models
/// that already passed it.
pub fn binding_facets<'a, C, B>(catalogues: C, bindings: B) -> BTreeMap<String, Facet>
where
    C: IntoIterator<Item = (&'a String, &'a Catalogue)>,
    B: IntoIterator<Item = (&'a String, &'a Binding)>,
{
    let by_id: BTreeMap<&str, &Catalogue> = catalogues
        .into_iter()
        .map(|(id, catalogue)| (id.as_str(), catalogue))
        .collect();
    let mut out = BTreeMap::new();
    for (id, binding) in bindings {
        let Some(catalogue) = by_id.get(binding.catalogue.as_str()) else {
            continue;
        };
        out.insert(id.clone(), binding_facet(binding, catalogue));
    }
    out
}

/// The closed facet one binding is. See [`binding_facets`].
pub fn binding_facet(binding: &Binding, catalogue: &Catalogue) -> Facet {
    Facet {
        values: catalogue.entries.keys().cloned().collect(),
        default: binding.default.clone(),
        open: false,
        doc: binding.doc.clone(),
    }
}

// ----------------------------------------------------------------------------
// Internals
// ----------------------------------------------------------------------------

fn merge_exports(
    ids: &BTreeSet<String>,
    source_id: &str,
    out: &mut BTreeMap<String, Vec<String>>,
) {
    for id in ids {
        out.entry(id.clone()).or_default().push(source_id.to_string());
    }
}

fn merge_imports(
    from: &BTreeMap<String, BTreeSet<String>>,
    out: &mut BTreeMap<String, BTreeSet<String>>,
) {
    for (name, importers) in from {
        out.entry(name.clone())
            .or_default()
            .extend(importers.iter().cloned());
    }
}

fn import(map: &mut BTreeMap<String, BTreeSet<String>>, name: &str, importer: &str) {
    map.entry(name.to_string())
        .or_default()
        .insert(importer.to_string());
}

fn collect_component_links(id: &str, component: &Component, summary: &mut InterfaceSummary) {
    for dep in &component.depends_on {
        import(&mut summary.imports.components, dep, id);
    }
    if let Some(condition) = component.condition.as_deref() {
        let path = format!("components.{id}");
        collect_condition_facets(condition, &path, summary);
        record_selector(summary, path, condition);
    }
    // ADR-0057 §D4. A requirement is an import of a BINDING, not of a
    // component: it names a shared choice, so it is deliberately kept out of
    // `imports.components` and out of the dependency closure the graph checks
    // walk.
    for (slot, requirement) in &component.requires {
        import(
            &mut summary.imports.bindings,
            &requirement.binding,
            &format!("components.{id}.requires.{slot}"),
        );
        summary.requirements.push(RequirementLink {
            component: id.to_string(),
            slot: slot.clone(),
            binding: requirement.binding.clone(),
            accepts: requirement.accepts.clone(),
            condition: component.condition.clone(),
        });
    }
    // Params id-ascending for the reason the definition/component walks are
    // (see `summarize`): `Component::params` is a `HashMap` and its override
    // conditions land in `selectors`.
    let mut param_keys: Vec<&String> = component.params.keys().collect();
    param_keys.sort();
    for key in param_keys {
        collect_parameter_links(
            &component.params[key],
            &format!("components.{id}.params.{key}"),
            summary,
        );
    }
}

fn collect_parameter_links(param: &Parameter, path: &str, summary: &mut InterfaceSummary) {
    if let Some(target) = param.inherits.as_deref() {
        import(&mut summary.imports.definitions, target, path);
    }
    for (idx, block) in param.overrides.iter().enumerate() {
        let branch = format!("{path}.overrides[{idx}]");
        collect_condition_facets(&block.condition, &branch, summary);
        record_selector(summary, branch.clone(), &block.condition);
        // The condition BEFORE the nested payload, so a nested override's
        // selector follows its parent's — the order
        // `compiler_core::collect_parameter_conditions` has always walked.
        collect_parameter_links(block.payload.as_ref(), &branch, summary);
    }
}

fn collect_constraint_links(id: &str, constraint: &Constraint, summary: &mut InterfaceSummary) {
    summary.clauses.push(Clause {
        id: id.to_string(),
        condition: constraint.condition.clone(),
    });
    collect_condition_facets(&constraint.condition, &format!("constraints.{id}"), summary);
}

fn collect_binding_links(id: &str, binding: &Binding, summary: &mut InterfaceSummary) {
    import(&mut summary.imports.catalogues, &binding.catalogue, id);

    let mut derive_source = None;
    let mut derive_pairs = Vec::new();
    let mut derive_source_count = 0;
    if let Some(table) = &binding.derive {
        derive_source_count = table.len();
        // v1 admits exactly one source; the validator rejects anything else.
        // Reading the first (id-ascending) keeps this projection total so the
        // diagnostic can still name the binding.
        if let Some((source, pairs)) = table.iter().next() {
            derive_source = Some(source.clone());
            derive_pairs = pairs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            import(&mut summary.imports.facets, source, &format!("bindings.{id}"));
        }
    }

    summary.binding_links.insert(
        id.to_string(),
        BindingLink {
            catalogue: binding.catalogue.clone(),
            default: binding.default.clone(),
            derive_source,
            derive_pairs,
            derive_source_count,
        },
    );
}

/// Record one inclusion selector under the authored path that carries it.
///
/// Blank conditions are dropped here rather than downstream so the header
/// carries no entry that contributes nothing: `collect_ccm_clauses` has always
/// skipped an empty condition, and a selector list that mirrors the emitted
/// clause channel is easier to read against a `.ccm` than one that does not.
/// An UNPARSEABLE condition is kept, because whether it parses is the reader's
/// judgement (the clause builder skips it, `validate_facets` skips it) and a
/// header that silently dropped it would hide what the author wrote.
fn record_selector(summary: &mut InterfaceSummary, id: String, condition: &str) {
    if condition.trim().is_empty() {
        return;
    }
    summary.selectors.push(Clause {
        id,
        condition: condition.to_string(),
    });
}

/// Record every facet name a condition or constraint expression references.
///
/// An expression that does not parse contributes nothing, mirroring
/// `compiler_core::collect_ccm_clauses` and `link_verify::validate_facets`: a
/// selector that cannot be understood widens no domain, so it imports nothing
/// either. (A `constraints` entry that does not parse is a hard ingest error
/// raised separately in `link_verify::validate_constraints`, so nothing is
/// silently lost.)
fn collect_condition_facets(condition: &str, importer: &str, summary: &mut InterfaceSummary) {
    let trimmed = condition.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(expr) = parse_condition_expr(trimmed) else {
        return;
    };
    let mut names: BTreeSet<String> = BTreeSet::new();
    for_each_condition_operand(&expr, |operand| match operand {
        ConditionOperand::Symbol(tag, _) => {
            names.insert(tag.to_string());
        }
        ConditionOperand::ComparisonLeft(tag) | ConditionOperand::ComparisonRight(tag) => {
            names.insert(tag.to_string());
        }
    });
    for name in names {
        import(&mut summary.imports.facets, &name, importer);
    }
}

#[cfg(test)]
#[path = "interface_summary_tests.rs"]
mod interface_summary_tests;
