// SPDX-License-Identifier: BUSL-1.1

//! `link`: a set of objects becomes the package `compile` produces
//! (ADR-0058 §D4, §D8).
//!
//! Three stages, and nothing is written under `--out` unless all three pass.
//!
//! 1. **Headers only.** Unit names unique, exported ids unique across objects,
//!    every `interfaces[]` entry matching the object of that unit that was
//!    actually linked, every import provided by some object's exports, and then
//!    the ADR-0057 §D9 summary checks over the merged headers. No chunk file is
//!    opened, so the working set is proportional to the number of ids rather
//!    than to parameter volume — and a stage-1 failure is reported even when
//!    every chunk file on disk is unreadable.
//! 2. **Constraint model.** Built from the merged declarations and the headers'
//!    clauses and selectors, in the canonical order §A2 fixes: objects by unit
//!    name ascending, chunks by `chunk_hash` ascending, then the in-chunk walk.
//!    Neither `--object` order nor `--source` order can reach a byte.
//! 3. **Emit.** The chunk files copied by hash, the index built from them, the
//!    manifest, the `.ccm`, and the provenance sidecars.
//!
//! **`compile` is this module fed by in-memory objects** (§D8). It groups its
//! `--source` chunks by `package`, builds one header per unit without writing
//! it, and runs the same stages. There is one code path, so the two forms
//! cannot produce different packages — which is the byte-identity oracle
//! configflux-p0jz.2 is accepted against.
//!
//! **What is checked where, and why the two forms can still differ on a
//! DIAGNOSTIC.** Stage 1 answers from headers alone; the complete-model checks
//! ([`crate::compiler_core::verify_complete_model`]) answer from bodies. On the
//! compile path the whole model is present from the start, so the product API
//! runs the complete-model checks BEFORE stage 1 and an unresolved
//! `depends_on` keeps the code it has always had (`E_UNKNOWN_COMPONENT_DEP`).
//! On the link path the object that would have provided it may simply not have
//! been passed, so there is no body to check and stage 1 reports
//! `E_LINK_UNRESOLVED_IMPORT` instead — naming the unit and the missing id,
//! which is the information a linker has and a compiler does not. Neither form
//! accepts a model the other rejects; they differ only in which name they can
//! put on the same fault.

use std::collections::{BTreeMap, BTreeSet};

use crate::ccm_emitter::ConditionModel;
use crate::compiler_core::{condition_model_from_summary, SourceChunk};
use crate::interface_summary::{BindingLink, Clause, InterfaceSummary, MergedSummary, RequirementLink};
use crate::link_verify::validate_linked_summary;
use crate::object::ObjectHeader;
use crate::product_api::{
    map_graph_error, Diagnostic, DiagnosticSeverity, E_LINK_DUPLICATE_ID, E_LINK_DUPLICATE_UNIT,
    E_LINK_INTERFACE_MISMATCH, E_LINK_UNRESOLVED_IMPORT,
};

/// The in-memory objects a one-shot `compile` links (ADR-0058 §D8).
///
/// `compile` groups its `--source` chunks by `package` into units and builds
/// one header per unit through the SAME merge `compile_object` writes, without
/// writing anything. The linker then runs over those headers exactly as it runs
/// over headers read off disk, which is what makes the two forms one code path.
///
/// `interfaces` is empty for every header, and that is the honest value rather
/// than a shortcut: an in-memory object is compiled against the other units of
/// the same compile, whose hashes would each depend on the others' — mutually,
/// for any two units that reference each other. There is nothing to mismatch
/// (`E_LINK_INTERFACE_MISMATCH` is vacuous when every object was built in one
/// process from one input set), so the field records nothing.
///
/// Total: grouping by `package` IS the definition of a unit (ADR-0057 §D1), so
/// unlike `compile-object` there is no agreement to check and nothing to
/// refuse. `summaries` must be the per-chunk summaries in chunk order.
pub(crate) fn in_memory_headers(
    chunks: &[SourceChunk],
    summaries: &[InterfaceSummary],
) -> Vec<ObjectHeader> {
    // Chunk-hash ascending WITHIN each unit — the order the header commits to
    // (ADR-0058 §D2) and the inner level of the §A2 canonical clause order.
    let mut by_unit: BTreeMap<&str, Vec<(&str, &InterfaceSummary)>> = BTreeMap::new();
    for (chunk, summary) in chunks.iter().zip(summaries) {
        by_unit
            .entry(chunk.config.package.as_str())
            .or_default()
            .push((chunk.chunk_hash.as_str(), summary));
    }
    by_unit
        .into_iter()
        .map(|(unit, mut rows)| {
            rows.sort_by(|a, b| a.0.cmp(b.0));
            let hashes: Vec<String> = rows.iter().map(|(hash, _)| (*hash).to_string()).collect();
            let unit_summaries: Vec<InterfaceSummary> =
                rows.into_iter().map(|(_, summary)| summary.clone()).collect();
            ObjectHeader::from_summaries(unit, hashes, &unit_summaries, Vec::new())
        })
        .collect()
}

/// Stage 1 (ADR-0058 §D4): every check that can be answered from headers, in
/// the order that puts the most structural fault first.
///
/// Returns the merged summary the later stages read, with objects visited by
/// unit name ascending — the outer level of the §A2 canonical clause order.
pub(crate) fn link_stage_headers(headers: &[ObjectHeader]) -> Result<MergedSummary, Diagnostic> {
    // The order ADR-0058 §D4 lists them in, and it is the order of decreasing
    // structural reach: a unit claimed twice makes every later answer
    // ambiguous, an id exported twice makes one reference ambiguous, an import
    // nothing provides makes one reference unanswerable, and an interface whose
    // hash moved leaves the model well formed but checked against something
    // else.
    let ordered = order_objects(headers)?;
    let merged = merge_headers(&ordered);
    check_duplicate_ids(&merged)?;
    check_unresolved_imports(&merged)?;
    check_interface_hashes(&ordered)?;
    // The ADR-0057 §D9 checks — binding links, requirement resolution,
    // `accepts` membership, the empty-intersection rule, and the unique-export
    // backstop — over exactly the same type the compile path passes them, and
    // through the same error-to-code mapping, so a binding fault carries
    // `E_BINDING_INVALID` here exactly as it does in a one-shot compile.
    validate_linked_summary(&merged).map_err(|err| map_graph_error(&err))?;
    Ok(merged)
}

/// Objects by unit name ascending, refusing two objects that claim one unit.
fn order_objects(headers: &[ObjectHeader]) -> Result<Vec<&ObjectHeader>, Diagnostic> {
    let mut ordered: Vec<&ObjectHeader> = headers.iter().collect();
    ordered.sort_by(|a, b| a.unit.cmp(&b.unit));
    for pair in ordered.windows(2) {
        if pair[0].unit != pair[1].unit {
            continue;
        }
        return Err(diagnostic(
            E_LINK_DUPLICATE_UNIT,
            format!(
                "Two linked objects declare unit '{}' (object_hash {} and {}); a unit names \
                 exactly one object in a link",
                pair[0].unit, pair[0].object_hash, pair[1].object_hash
            ),
            "Link one object per unit: drop the stale copy, or recompile the unit once",
        ));
    }
    Ok(ordered)
}

/// Every `interfaces[]` entry names an object of that unit that was linked, with
/// the same `object_hash` it was compiled against (§D4 stage 1).
fn check_interface_hashes(ordered: &[&ObjectHeader]) -> Result<(), Diagnostic> {
    let linked: BTreeMap<&str, &str> = ordered
        .iter()
        .map(|header| (header.unit.as_str(), header.object_hash.as_str()))
        .collect();
    for header in ordered {
        for reference in &header.interfaces {
            match linked.get(reference.unit.as_str()) {
                // An interface that was not linked at all is not this rule's
                // fault to report: whatever the unit actually NEEDS from it is
                // an unresolved import, and that message names the missing id
                // rather than only the missing object.
                None => continue,
                Some(hash) if *hash == reference.object_hash => continue,
                Some(hash) => {
                    return Err(diagnostic(
                        E_LINK_INTERFACE_MISMATCH,
                        format!(
                            "Unit '{}' was compiled against unit '{}' at object_hash {}, but the \
                             '{}' object linked here is {}",
                            header.unit, reference.unit, reference.object_hash, reference.unit, hash
                        ),
                        "Recompile the dependent unit against this interface object, or link the \
                         interface object it was compiled against",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The linked set's merged summary, built from headers alone.
///
/// The "source" recorded against every exported id, and the "importer" recorded
/// against every imported one, is the UNIT name — not a path and not the
/// authored site. A link diagnostic names objects; a header carries no path by
/// construction (ADR-0056, ADR-0058 §D2) and drops the per-site importer set
/// that a per-chunk summary keeps. Nothing in
/// [`validate_linked_summary`] reads `imports`, so the two populations of the
/// same type never meet.
fn merge_headers(ordered: &[&ObjectHeader]) -> MergedSummary {
    let mut merged = MergedSummary::default();
    for header in ordered {
        merged.units.insert(header.unit.clone());
        let unit = header.unit.as_str();
        declare(&mut merged.exports.definitions, &header.exports.definitions, unit);
        declare(&mut merged.exports.components, &header.exports.components, unit);
        declare(&mut merged.exports.artifacts, &header.exports.artifacts, unit);
        declare(&mut merged.exports.facets, &header.exports.facets, unit);
        declare(&mut merged.exports.bindings, &header.exports.bindings, unit);
        declare(&mut merged.exports.catalogues, &header.exports.catalogues, unit);
        declare(&mut merged.exports.constraints, &header.exports.constraints, unit);

        import(&mut merged.imports.components, &header.imports.components, unit);
        import(&mut merged.imports.definitions, &header.imports.definitions, unit);
        import(&mut merged.imports.facets, &header.imports.facets, unit);
        import(&mut merged.imports.bindings, &header.imports.bindings, unit);
        import(&mut merged.imports.catalogues, &header.imports.catalogues, unit);

        first_wins(&mut merged.facet_domains, &header.facet_domains);
        merged.open_facets.extend(header.open_facets.iter().cloned());
        first_wins(&mut merged.catalogue_entries, &header.catalogue_entries);
        first_wins_links(&mut merged.binding_links, &header.binding_links);

        merged.requirements.extend(header.requirements.iter().cloned());
        merged.clauses.extend(header.clauses.iter().cloned());
        // NOT sorted: unit-ascending then the header's own chunk-hash order IS
        // the canonical clause order (ADR-0058 §A2).
        merged.selectors.extend(header.selectors.iter().cloned());
    }
    // Properties of the linked SET rather than of the order its objects
    // arrived in, exactly as `interface_summary::merge` ends.
    merged
        .requirements
        .sort_by(|a: &RequirementLink, b| a.component.cmp(&b.component).then_with(|| a.slot.cmp(&b.slot)));
    merged.clauses.sort_by(|a: &Clause, b| a.id.cmp(&b.id));
    merged
}

/// One exported id, one object (`E_LINK_DUPLICATE_ID`, naming both units).
fn check_duplicate_ids(merged: &MergedSummary) -> Result<(), Diagnostic> {
    let namespaces: [(&str, &BTreeMap<String, Vec<String>>); 7] = [
        ("definition", &merged.exports.definitions),
        ("component", &merged.exports.components),
        ("artifact", &merged.exports.artifacts),
        ("facet", &merged.exports.facets),
        ("binding", &merged.exports.bindings),
        ("catalogue", &merged.exports.catalogues),
        ("constraint", &merged.exports.constraints),
    ];
    for (noun, index) in namespaces {
        for (id, units) in index {
            if units.len() < 2 {
                continue;
            }
            return Err(diagnostic(
                E_LINK_DUPLICATE_ID,
                format!(
                    "Units '{}' and '{}' both export {} '{}'; an id names one declaration across \
                     the whole linked set",
                    units[0], units[1], noun, id
                ),
                "Rename one of the two declarations, or link only the unit that owns the id",
            ));
        }
    }
    Ok(())
}

/// Every import is provided by some object's exports (`E_LINK_UNRESOLVED_IMPORT`).
///
/// The facet namespace is deliberately absent: a condition naming a facet
/// nothing declares keeps its legacy condition-inferred domain (ADR-0047 §3),
/// so it is header information rather than an obligation.
fn check_unresolved_imports(merged: &MergedSummary) -> Result<(), Diagnostic> {
    // Namespaces in the header's own declaration order, so the message an
    // author meets first is the one the header lists first.
    type Obligation<'a> = (
        &'a str,
        &'a BTreeMap<String, BTreeSet<String>>,
        &'a BTreeMap<String, Vec<String>>,
        &'a str,
    );
    let obligations: [Obligation<'_>; 4] = [
        (
            "component",
            &merged.imports.components,
            &merged.exports.components,
            "depends_on",
        ),
        (
            "definition",
            &merged.imports.definitions,
            &merged.exports.definitions,
            "inherits",
        ),
        (
            "binding",
            &merged.imports.bindings,
            &merged.exports.bindings,
            "requires",
        ),
        (
            "catalogue",
            &merged.imports.catalogues,
            &merged.exports.catalogues,
            "binds to",
        ),
    ];
    for (noun, imports, exports, verb) in obligations {
        for (id, importers) in imports {
            if exports.contains_key(id) {
                continue;
            }
            // Importers are UNIT names here (see `merge_headers`); the first is
            // the alphabetically smallest, so the message is deterministic.
            let unit = importers
                .iter()
                .next()
                .map(String::as_str)
                .unwrap_or("<unknown>");
            return Err(diagnostic(
                E_LINK_UNRESOLVED_IMPORT,
                format!("unit '{unit}' {verb} {noun} '{id}'; no linked object declares it"),
                "Link the object whose unit declares it, or drop the reference",
            ));
        }
    }
    Ok(())
}

/// Stage 2's model, bound to the package identity stage 3 produced.
///
/// A one-line re-export of [`condition_model_from_summary`] under the name the
/// stage sequence uses, so a reader following §D4 finds the stage where the ADR
/// says it is. `model_hash` must be the CMP's own (`IrIndex::config_hash`),
/// which `open_model` byte-compares against `CmpManifest::model_hash`.
pub(crate) fn link_stage_model(model_hash: &str, merged: &MergedSummary) -> ConditionModel {
    condition_model_from_summary(model_hash, merged)
}

// ----------------------------------------------------------------------------
// Internals
// ----------------------------------------------------------------------------

fn declare(index: &mut BTreeMap<String, Vec<String>>, ids: &BTreeSet<String>, unit: &str) {
    for id in ids {
        index.entry(id.clone()).or_default().push(unit.to_string());
    }
}

fn import(
    index: &mut BTreeMap<String, BTreeSet<String>>,
    ids: &BTreeSet<String>,
    unit: &str,
) {
    for id in ids {
        index.entry(id.clone()).or_default().insert(unit.to_string());
    }
}

fn first_wins(into: &mut BTreeMap<String, Vec<String>>, from: &BTreeMap<String, Vec<String>>) {
    for (key, values) in from {
        into.entry(key.clone()).or_insert_with(|| values.clone());
    }
}

fn first_wins_links(
    into: &mut BTreeMap<String, BindingLink>,
    from: &BTreeMap<String, BindingLink>,
) {
    for (key, link) in from {
        into.entry(key.clone()).or_insert_with(|| link.clone());
    }
}

fn diagnostic(code: &str, message: String, hint: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: DiagnosticSeverity::Error,
        message,
        source_id: None,
        entity_path: None,
        hint: Some(hint.to_string()),
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod link_tests;
