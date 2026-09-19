// SPDX-License-Identifier: BUSL-1.1

//! `compile-object`: one unit, compiled alone against zero or more interface
//! objects (ADR-0058 §D3).
//!
//! The check set here is deliberately NARROWER than the compile path's, and
//! narrower in exactly one way: a reference the unit and its interfaces do not
//! resolve is recorded in the header's `imports` as a link obligation instead of
//! being rejected. Everything the unit CAN be checked against, it is checked
//! against, by the same functions the compile path calls — `link_verify` takes a
//! [`Scope`] rather than growing a second implementation, so a rule cannot come
//! to mean two things.
//!
//! An interface contributes only what its HEADER holds: facet domains, catalogue
//! entry rosters and binding links (ADR-0058 §A3). Its chunk files are never
//! opened, which is the whole reason compiling against an interface costs a
//! single small read. Its `exports` are not consulted, and that is not an
//! omission: under [`Scope::UnitLocal`] an unresolved `inherits` or `depends_on`
//! target is deferred whether or not an interface declares it, so knowing the
//! interface's ids would change no outcome here. The linker is where they earn
//! their keep. Its `requirements` are likewise carried for the link stage's
//! intersection check and validated nowhere here — they were already checked
//! when that unit was compiled, and re-checking them against a narrower view
//! than they were compiled under would invent failures.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::compiler_core::{Compiler, SourceChunk};
use crate::interface_summary::{self, binding_facets, InterfaceSummary};
use crate::ir::{self, IrChunk};
use crate::link_verify::{
    validate_catalogues, validate_component_dependencies_scoped, validate_constraints_scoped,
    validate_definition_inheritance_scoped, validate_facet_bindings_scoped, validate_facets,
    validate_object_summary, validate_parameter_values, Scope,
};
use crate::object::{InterfaceRef, ObjectHeader, OBJECT_FORMAT_VERSION, OBJECT_HEADER_FILENAME};
use crate::product_api::{
    map_compile_input_error, map_graph_error, Diagnostic, DiagnosticSeverity, SourceManifestEntry,
    E_COMPILE_EMIT_FAILED, E_OBJECT_UNIT_MISMATCH, PRODUCT_SCHEMA_VERSION,
};
use crate::provenance_sidecar::{hash_file, now_rfc3339_utc, ProvenanceSidecar};
use crate::schema::Facet;

/// One `compile-object` invocation.
pub struct CompileObjectRequest {
    /// The unit's chunks. Every one must carry the same `package` value
    /// (ADR-0058 §D1); that value is the unit's name.
    pub sources: Vec<SourceManifestEntry>,
    /// The interface objects' headers, already read from disk by the caller.
    /// Reading them there rather than here is what keeps "this path cannot be
    /// given a path" true: a directory that does not exist or a header that does
    /// not parse is a caller-side input error, not a compilation diagnostic.
    pub interfaces: Vec<ObjectHeader>,
    /// The object directory to write.
    pub output_dir: String,
    /// Stamp the provenance sidecar with a wall clock (ADR-0044 D1). Off keeps
    /// the object byte-stable across runs.
    pub stamp_time: bool,
}

/// Compile one unit into an object directory, returning its header.
///
/// Nothing is written unless every check passes.
pub fn compile_object(request: CompileObjectRequest) -> Result<ObjectHeader, Diagnostic> {
    let mut compiler = Compiler::new();
    for source in &request.sources {
        compiler
            .add_chunk_auto(source.source_id.clone(), &source.inline_content)
            .map_err(|err| {
                map_compile_input_error(&err, Some(source.source_id.clone()))
            })?;
    }

    let unit = unit_name(compiler.source_chunks())?;
    let external = ExternalDeclarations::from_headers(&request.interfaces);

    // Chunks in `chunk_hash` ascending order — the order the header commits to,
    // and the one thing that makes `--source` order unobservable in the bytes.
    let mut ordered: Vec<&SourceChunk> = compiler.source_chunks().iter().collect();
    ordered.sort_by(|a, b| a.chunk_hash.cmp(&b.chunk_hash));
    let summaries: Vec<InterfaceSummary> = ordered
        .iter()
        .map(|chunk| interface_summary::summarize(&chunk.config, &chunk.source_id))
        .collect();

    verify_object(&compiler, &summaries, &external).map_err(|err| map_graph_error(&err))?;

    let header = ObjectHeader::from_summaries(
        &unit,
        ordered.iter().map(|c| c.chunk_hash.clone()).collect(),
        &summaries,
        request
            .interfaces
            .iter()
            .map(ObjectHeader::as_interface_ref)
            .collect::<Vec<InterfaceRef>>(),
    );

    write_object(
        Path::new(&request.output_dir),
        &ordered,
        &header,
        request.stamp_time,
    )
    .map_err(|err| Diagnostic {
        code: E_COMPILE_EMIT_FAILED.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("{err:#}"),
        source_id: None,
        entity_path: None,
        hint: Some(
            "Choose a writable --out directory and check its ownership and mount options"
                .to_string(),
        ),
    })?;

    Ok(header)
}

/// The one `package` value every chunk of the call must carry (ADR-0058 §D1).
///
/// The diagnostic names BOTH values and BOTH files because neither half is
/// actionable alone: the author has to see which two sources disagree to know
/// which of them landed on the wrong command line.
fn unit_name(chunks: &[SourceChunk]) -> Result<String, Diagnostic> {
    let Some(first) = chunks.first() else {
        return Err(Diagnostic {
            code: E_OBJECT_UNIT_MISMATCH.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "compile-object needs at least one --source chunk to name a unit".to_string(),
            source_id: None,
            entity_path: None,
            hint: Some("Pass the unit's chunks with --source".to_string()),
        });
    };
    for chunk in chunks {
        if chunk.config.package == first.config.package {
            continue;
        }
        return Err(Diagnostic {
            code: E_OBJECT_UNIT_MISMATCH.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Sources belong to two different units: '{}' declares package '{}' and '{}' \
                 declares package '{}'. One object is one unit (ADR-0058), so its chunks must \
                 agree on `package`.",
                first.source_id, first.config.package, chunk.source_id, chunk.config.package
            ),
            source_id: Some(chunk.source_id.clone()),
            entity_path: None,
            hint: Some(
                "Compile each unit into its own object and pass the others with --interface"
                    .to_string(),
            ),
        });
    }
    Ok(first.config.package.clone())
}

/// What the interface headers declare, folded into one lookup view.
///
/// First header wins per id. Two interfaces declaring one id is a duplicate the
/// LINKER reports naming both units (`E_LINK_DUPLICATE_ID`); it is not this
/// step's to raise, because this step sees a subset of the objects that will be
/// linked and would be guessing at which pair the author meant.
#[derive(Default)]
struct ExternalDeclarations {
    facet_domains: BTreeMap<String, Vec<String>>,
    catalogue_entries: BTreeMap<String, Vec<String>>,
    binding_links: BTreeMap<String, crate::interface_summary::BindingLink>,
}

impl ExternalDeclarations {
    fn from_headers(headers: &[ObjectHeader]) -> Self {
        let mut out = Self::default();
        for header in headers {
            for (name, values) in &header.facet_domains {
                out.facet_domains
                    .entry(name.clone())
                    .or_insert_with(|| values.clone());
            }
            for (name, entries) in &header.catalogue_entries {
                out.catalogue_entries
                    .entry(name.clone())
                    .or_insert_with(|| entries.clone());
            }
            for (name, link) in &header.binding_links {
                out.binding_links
                    .entry(name.clone())
                    .or_insert_with(|| link.clone());
            }
        }
        out
    }

    /// The closed facets an interface contributes: its declared facets, plus the
    /// facet each of its bindings IS (ADR-0057 §D3).
    ///
    /// A header records a facet's VALUES but not its `open` flag, so an
    /// interface's facet is treated as closed here. That is what ADR-0058 §A3
    /// asks for, and it is the fail-closed direction; the linker, which holds
    /// the declaring chunk, is the authority either way.
    fn facets(&self) -> BTreeMap<String, Facet> {
        let mut out: BTreeMap<String, Facet> = BTreeMap::new();
        for (name, values) in &self.facet_domains {
            out.insert(
                name.clone(),
                Facet {
                    values: values.clone(),
                    default: None,
                    open: false,
                    doc: None,
                },
            );
        }
        for (name, link) in &self.binding_links {
            let Some(entries) = self.catalogue_entries.get(&link.catalogue) else {
                continue;
            };
            out.insert(
                name.clone(),
                Facet {
                    values: entries.clone(),
                    default: link.default.clone(),
                    open: false,
                    doc: None,
                },
            );
        }
        out
    }
}

/// Every object-time check, in the order the compile path runs them.
///
/// The order is not cosmetic: a catalogue's SHAPE is validated before any rule
/// reads its entry roster, and a binding is resolved before a requirement's
/// `accepts` list is checked against the catalogue it names.
fn verify_object(
    compiler: &Compiler,
    summaries: &[InterfaceSummary],
    external: &ExternalDeclarations,
) -> Result<()> {
    let repo = compiler.get_repo();

    validate_definition_inheritance_scoped(&repo.definitions, Scope::UnitLocal)?;
    validate_component_dependencies_scoped(&repo.components, Scope::UnitLocal)?;
    validate_catalogues(&repo.catalogues)?;
    // configflux-2yiq. Unscoped on purpose: a non-finite float is unrepresentable
    // in the canonical bytes wherever it is authored, so unlike the reference
    // rules around it there is nothing for the linker to resolve later. A unit
    // that carries one must not become an object at all — the object's chunk
    // files are the package's chunk files, copied by hash.
    validate_parameter_values(&repo.definitions, &repo.components)?;

    let mut merged = interface_summary::merge(summaries);
    for (name, values) in &external.facet_domains {
        merged
            .facet_domains
            .entry(name.clone())
            .or_insert_with(|| values.clone());
    }
    for (name, entries) in &external.catalogue_entries {
        merged
            .catalogue_entries
            .entry(name.clone())
            .or_insert_with(|| entries.clone());
    }
    for (name, link) in &external.binding_links {
        merged
            .binding_links
            .entry(name.clone())
            .or_insert_with(|| link.clone());
    }
    validate_object_summary(&merged)?;

    // The unit's own facets, the facet each of its bindings is, and what the
    // interfaces declare — the unit's own always winning, because a collision
    // between the two is a duplicate declaration the LINKER names with both
    // units, and shadowing it here would only change which message the author
    // gets first.
    let mut facets = external.facets();
    facets.extend(binding_facets(&repo.catalogues, &repo.bindings));
    for (id, facet) in &repo.facets {
        facets.insert(id.clone(), facet.clone());
    }
    let facets: std::collections::HashMap<String, Facet> = facets.into_iter().collect();

    validate_facets(&facets, &repo.components, &repo.definitions)?;
    validate_constraints_scoped(&repo.constraints, &facets, Scope::UnitLocal)?;
    // ADR-0064 D2 at unit scope: the three per-parameter rules hold over one
    // unit, while the whole-model one-handle-per-facet rule is a LINK
    // obligation — two units each binding one facet is exactly what `link`
    // exists to catch.
    validate_facet_bindings_scoped(&repo.components, &facets, Scope::UnitLocal)
}

/// Write the object directory: the unit's chunk files exactly as the package
/// stores them, the header, and the deterministic provenance sidecar.
///
/// The chunk files go through the SAME `IrChunk::from_config` the package emit
/// uses, with the same compact serializer, which is what makes an object's copy
/// byte-identical to the package's and therefore copyable by hash at link time.
fn write_object(
    dir: &Path,
    chunks: &[&SourceChunk],
    header: &ObjectHeader,
    stamp_time: bool,
) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create object dir '{}'", dir.display()))?;

    for chunk in chunks {
        let ir_chunk = IrChunk::from_config(&chunk.source_id, &chunk.chunk_hash, &chunk.config);
        let path: PathBuf = dir.join(format!("chunk-{}.cfir", chunk.chunk_hash));
        std::fs::write(&path, ir::chunk_file_bytes(&ir_chunk)?)
            .with_context(|| format!("Failed to write object chunk '{}'", path.display()))?;
    }

    header.write_to_dir(dir)?;

    // The sidecar records the header alone. The header names every chunk by
    // content hash, so a chunk file's integrity is checkable from it — the same
    // rule the package sidecar follows in recording only its manifest.
    let mut schema_versions = BTreeMap::new();
    schema_versions.insert("ir_chunk".to_string(), ir::IR_FORMAT_VERSION);
    schema_versions.insert("object_header".to_string(), OBJECT_FORMAT_VERSION);
    schema_versions.insert("product".to_string(), PRODUCT_SCHEMA_VERSION);
    let mut artifacts = BTreeMap::new();
    artifacts.insert(
        OBJECT_HEADER_FILENAME.to_string(),
        hash_file(&dir.join(OBJECT_HEADER_FILENAME))?,
    );
    let stamped_at = if stamp_time {
        Some(now_rfc3339_utc())
    } else {
        None
    };
    ProvenanceSidecar::new(schema_versions, artifacts, stamped_at).write_to_dir(dir)
}

/// One-line human summary of a written object (the CLI's default output).
///
/// Namespaces are printed in HEADER order rather than alphabetically, so the
/// line reads in the same order as the file it describes.
pub fn object_summary_line(header: &ObjectHeader, output_dir: &str) -> String {
    let exports = [
        ("definitions", header.exports.definitions.len()),
        ("components", header.exports.components.len()),
        ("artifacts", header.exports.artifacts.len()),
        ("facets", header.exports.facets.len()),
        ("bindings", header.exports.bindings.len()),
        ("catalogues", header.exports.catalogues.len()),
        ("constraints", header.exports.constraints.len()),
    ];
    let imports = [
        ("components", header.imports.components.len()),
        ("definitions", header.imports.definitions.len()),
        ("facets", header.imports.facets.len()),
        ("bindings", header.imports.bindings.len()),
        ("catalogues", header.imports.catalogues.len()),
    ];
    format!(
        "object: {}  unit: {}  object_hash: {}  chunks: {}  exports: {}  imports: {}",
        output_dir,
        header.unit,
        header.object_hash,
        header.chunk_hashes.len(),
        render_counts(&exports),
        render_counts(&imports),
    )
}

fn render_counts(counts: &[(&str, usize)]) -> String {
    counts
        .iter()
        .map(|(name, count)| format!("{name}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
}
