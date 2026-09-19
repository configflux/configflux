// SPDX-License-Identifier: BUSL-1.1

//! Reading objects off disk for the link (ADR-0058 §D4).
//!
//! Two reads, deliberately separate. [`read_headers`] is what stage 1 needs and
//! all it needs: one small file per object, so the whole graph check runs in a
//! working set proportional to the number of ids. [`load_chunks`] is stage 3,
//! and it is the first moment a chunk file is opened — which is why a stage-1
//! fault is reported even when every chunk in every object is unreadable.
//!
//! **What `E_LINK_OBJECT_CORRUPT` means.** An object is a build product, and
//! its header is tamper-evident: `object_hash` covers every other field, so a
//! header that parses and recomputes is the unit's interface as SOME writer
//! wrote it. This module checks the chunk files AGAINST that header — every
//! named chunk present, parsing, at this build's IR format version, carrying
//! its own name as `chunk_hash` — and then checks the HEADER against those
//! chunk files, by rebuilding it from them and comparing `object_hash`
//! ([`check_header_matches_bodies`]). It also checks each chunk file against
//! ITSELF: a chunk's address is the hash of the seven entity maps the chunk
//! carries (ADR-0056 Amendment 1), so [`ir::chunk_hash_of_chunk`] recomputes it
//! from the file alone and a body that no longer hashes to its own name is
//! refused.
//!
//! No one of the three subsumes another. The recomputed address is what catches
//! an edit to a value INSIDE a chunk, which moves neither the file's
//! `chunk_hash` field nor its name and leaves the header perfectly consistent.
//! The header is what says WHICH chunks the unit exported, which no chunk can
//! attest to on its own. And the header rebuild is what catches a header
//! rewritten WITH a fresh `object_hash` — self-consistent, and saying something
//! its own chunk files do not say (ADR-0063 Amendment 1 §2). The address covers
//! entity content only — `format_version`, `source_id` and `metadata` sit
//! outside the preimage — so it is not a whole-file checksum and nothing here
//! claims one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::interface_summary::{summarize_ir_chunk, Exports, InterfaceSummary};
use crate::ir::{self, IrChunk};
use crate::link_emit::LinkChunk;
use crate::object::{ObjectHeader, OBJECT_HEADER_FILENAME};
use crate::product_api::{Diagnostic, DiagnosticSeverity, E_LINK_OBJECT_CORRUPT};
use crate::schema::Config;

/// Every linked object's header, in the order the directories were given.
///
/// A directory that is not a readable, self-consistent object is
/// `E_LINK_OBJECT_CORRUPT` rather than a CLI input error: `--object` names a
/// build product, not a file the author wrote, so "it is not an object" is a
/// fault in the thing being linked.
pub(crate) fn read_headers(dirs: &[String]) -> Result<Vec<ObjectHeader>, Diagnostic> {
    dirs.iter()
        .map(|dir| {
            ObjectHeader::read_from_dir(Path::new(dir)).map_err(|err| {
                corrupt(
                    format!("{err:#}"),
                    &format!(
                        "Recompile the object with `compile-object`, or check that '{dir}' is an \
                         object directory holding {OBJECT_HEADER_FILENAME}"
                    ),
                )
            })
        })
        .collect()
}

/// The linked model, materialized: the chunk files to copy, the merged model
/// the complete-model checks read, and one summary per chunk.
pub(crate) struct LinkedModel {
    pub(crate) chunks: Vec<LinkChunk>,
    pub(crate) repository: Config,
    pub(crate) summaries: Vec<InterfaceSummary>,
}

/// Stage 3's read: every chunk of every object, checked against the header that
/// names it and against its own content address (see the module doc for why
/// neither check subsumes the other).
///
/// `dirs` and `headers` are parallel — the header at index `i` was read from the
/// directory at index `i`.
pub(crate) fn load_chunks(
    dirs: &[String],
    headers: &[ObjectHeader],
) -> Result<LinkedModel, Diagnostic> {
    let mut model = LinkedModel {
        chunks: Vec::new(),
        // `package` and `version` are authoring labels that the emitted IR
        // chunk does not carry and no check below reads (ADR-0056 removed the
        // last reader when `source_id` left the identity). The unit names live
        // in the headers, where the link messages take them from.
        repository: Config {
            package: String::new(),
            version: String::new(),
            definitions: Default::default(),
            components: Default::default(),
            artifacts: Default::default(),
            facets: Default::default(),
            constraints: Default::default(),
            catalogues: Default::default(),
            bindings: Default::default(),
        },
        summaries: Vec::new(),
    };

    for (dir, header) in dirs.iter().zip(headers) {
        // Where this object's summaries start. `load_chunks` accumulates one
        // flat list across every object, and the header rebuild below needs
        // exactly this unit's slice of it, in `chunk_hashes` order.
        let unit_summaries_from = model.summaries.len();
        for chunk_hash in &header.chunk_hashes {
            let chunk = read_chunk(Path::new(dir), &header.unit, chunk_hash)?;
            model.summaries.push(summarize_ir_chunk(&chunk.parsed));
            model.chunks.push(LinkChunk {
                chunk_hash: chunk_hash.clone(),
                source_id: chunk.parsed.source_id.clone(),
                exports: chunk_exports(&chunk.parsed),
                bytes: chunk.bytes,
            });
            // Moved last: `IrChunk` is not `Clone`, and everything above reads
            // it by reference, so the fold consumes it without a copy of the
            // whole chunk body.
            merge_into(&mut model.repository, chunk.parsed);
        }
        // The object's header says exactly what its chunk files say. Checked
        // per object, because an object is what a header describes and what
        // the remedy — recompile this one — acts on.
        check_header_matches_bodies(dir, header, &model.summaries[unit_summaries_from..])?;
    }
    Ok(model)
}

struct ReadChunk {
    parsed: IrChunk,
    bytes: Vec<u8>,
}

fn read_chunk(dir: &Path, unit: &str, chunk_hash: &str) -> Result<ReadChunk, Diagnostic> {
    let path: PathBuf = dir.join(format!("chunk-{chunk_hash}.cfir"));
    let bytes = std::fs::read(&path).map_err(|err| {
        corrupt(
            format!(
                "Object for unit '{}' names chunk {} but '{}' could not be read: {}",
                unit,
                chunk_hash,
                path.display(),
                err
            ),
            "Recompile the object with `compile-object`",
        )
    })?;
    let parsed: IrChunk = serde_json::from_slice(&bytes).map_err(|err| {
        corrupt(
            format!(
                "Object for unit '{}' holds a chunk file '{}' that does not parse: {}",
                unit,
                path.display(),
                err
            ),
            "Recompile the object with `compile-object`",
        )
    })?;
    if parsed.format_version != ir::IR_FORMAT_VERSION {
        return Err(corrupt(
            format!(
                "Object for unit '{}' holds chunk {} at IR format version {}, but this build \
                 reads {}",
                unit,
                chunk_hash,
                parsed.format_version,
                ir::IR_FORMAT_VERSION
            ),
            "Recompile the object with a matching build of `compile-object`",
        ));
    }
    if parsed.chunk_hash != chunk_hash {
        return Err(corrupt(
            format!(
                "Object for unit '{}' holds a file named chunk-{}.cfir whose content records \
                 chunk_hash {}",
                unit, chunk_hash, parsed.chunk_hash
            ),
            "Recompile the object with `compile-object`; a chunk file is named by its content \
             and is never renamed by hand",
        ));
    }
    // The content check the two above cannot make. Both of them read a value
    // the file declares about itself, so an edit to what the chunk HOLDS —
    // which touches neither the `chunk_hash` field nor the name — passes both.
    // This recomputes the address from the entity maps in front of us
    // (ADR-0056 Amendment 1) and compares it to the name the header asked for.
    let recomputed = ir::chunk_hash_of_chunk(&parsed).map_err(|err| {
        corrupt(
            format!(
                "Object for unit '{}' holds chunk-{}.cfir whose content address could not be \
                 recomputed: {:#}",
                unit, chunk_hash, err
            ),
            "Recompile the object with `compile-object`",
        )
    })?;
    if recomputed != chunk_hash {
        return Err(corrupt(
            format!(
                "Object for unit '{}' holds chunk-{}.cfir whose content hashes to {}",
                unit, chunk_hash, recomputed
            ),
            "Recompile the object with `compile-object`; a chunk file is named by its content, \
             so an edit to what it holds moves its name",
        ));
    }
    Ok(ReadChunk { parsed, bytes })
}

/// The ids one emitted chunk declares.
fn chunk_exports(chunk: &IrChunk) -> Exports {
    let mut exports = Exports::default();
    union(&mut exports, chunk);
    exports
}

fn union(into: &mut Exports, chunk: &IrChunk) {
    into.definitions.extend(chunk.definitions.keys().cloned());
    into.components.extend(chunk.components.keys().cloned());
    into.artifacts.extend(chunk.artifacts.keys().cloned());
    into.facets.extend(chunk.facets.keys().cloned());
    into.constraints.extend(chunk.constraints.keys().cloned());
    into.catalogues.extend(chunk.catalogues.keys().cloned());
    into.bindings.extend(chunk.bindings.keys().cloned());
}

/// The object's header, rebuilt from the chunk bodies just read, must be the
/// header on disk (ADR-0063 Amendment 1 §2).
///
/// [`ObjectHeader::from_summaries`] is a pure function of (unit, chunk hashes,
/// per-chunk summaries, interfaces) — the same constructor `compile-object`
/// built the written header with, over summaries of the same chunks — so
/// rebuilding it here and comparing `object_hash` asks ONE question that covers
/// every field the `.ccm` is later synthesized from: the exported id sets, the
/// facet domains and which of them are open, the catalogue entry rosters, the
/// binding links, the requirements, the clauses and the selectors.
///
/// That is the check the charset rule needs to reach the headers at all. `link`
/// validates the chunk BODIES it loaded (`verify_complete_model`), but merges
/// the `.ccm`'s facet domains and entry rosters from the on-disk HEADERS; the
/// predecessor of this function compared id SETS only, so a header carrying an
/// injected facet value or catalogue entry passed it with a clean body. Headers
/// now equal bodies, and bodies are validated, so the rosters are covered
/// transitively rather than by a second charset pass over the merge.
///
/// `read_from_dir` has already proved the header hashes to what it records.
/// That is a different question: it says the header was not edited after it was
/// written, and this says that what it says is what its chunk files say. A
/// header rewritten WITH a fresh `object_hash` — which is all a version skew or
/// an edit-and-rehash amounts to — passes the first and fails this one.
///
/// `chunk_hashes` and `interfaces` are taken FROM the header rather than
/// rediscovered, so they cannot differ here: the chunk list is what the loop
/// above read, and the interfaces are an attestation about other objects that
/// no chunk of this one carries. `check_interface_hashes` is what checks those
/// against the linked set.
fn check_header_matches_bodies(
    dir: &str,
    header: &ObjectHeader,
    summaries: &[InterfaceSummary],
) -> Result<(), Diagnostic> {
    let expected = ObjectHeader::from_summaries(
        &header.unit,
        header.chunk_hashes.clone(),
        summaries,
        header.interfaces.clone(),
    );
    if expected.object_hash == header.object_hash {
        return Ok(());
    }
    let unit = &header.unit;
    let field = expected.first_difference(header);
    // `exports` keeps the wording it carried when the id sets were the whole
    // check: a set difference has an ID to name, and "holds component 'x' that
    // its header does not export" is what an author can act on. No other field
    // has a comparable handle — a domain or a roster differs as a whole — so
    // the rest name the field and stop.
    if field == Some("exports") {
        check_declares(unit, dir, &expected.exports, &header.exports)?;
    }
    Err(corrupt(
        match field {
            Some(field) => format!(
                "Object '{dir}' (unit '{unit}') has a header that does not match its chunk \
                 files: {field} differs"
            ),
            // Unreachable while `first_difference` mirrors the `object_hash`
            // preimage: equal preimage fields hash equally. Kept total rather
            // than panicking, and worded so the refusal still stands if the
            // two ever drift.
            None => format!(
                "Object '{dir}' (unit '{unit}') has a header that does not match its chunk files"
            ),
        },
        "Recompile the object with `compile-object`",
    ))
}

fn check_declares(
    unit: &str,
    dir: &str,
    declared: &Exports,
    header: &Exports,
) -> Result<(), Diagnostic> {
    let namespaces = [
        ("definition", &declared.definitions, &header.definitions),
        ("component", &declared.components, &header.components),
        ("artifact", &declared.artifacts, &header.artifacts),
        ("facet", &declared.facets, &header.facets),
        ("constraint", &declared.constraints, &header.constraints),
        ("catalogue", &declared.catalogues, &header.catalogues),
        ("binding", &declared.bindings, &header.bindings),
    ];
    for (noun, found, listed) in namespaces {
        if let Some(id) = found.difference(listed).next() {
            return Err(corrupt(
                format!(
                    "Object '{dir}' (unit '{unit}') holds {noun} '{id}' in its chunk files, but \
                     its header does not export it"
                ),
                "Recompile the object with `compile-object`",
            ));
        }
        if let Some(id) = listed.difference(found).next() {
            return Err(corrupt(
                format!(
                    "Object '{dir}' (unit '{unit}') exports {noun} '{id}' in its header, but no \
                     chunk file declares it"
                ),
                "Recompile the object with `compile-object`",
            ));
        }
    }
    Ok(())
}

/// Fold one emitted chunk into the linked model.
///
/// A plain insert per namespace: an id declared by two chunks of one object is
/// impossible (ingest rejected it when the object was compiled) and an id
/// declared by two OBJECTS is `E_LINK_DUPLICATE_ID`, raised in stage 1 before
/// this function runs.
fn merge_into(repository: &mut Config, chunk: IrChunk) {
    extend(&mut repository.definitions, chunk.definitions);
    extend(&mut repository.components, chunk.components);
    extend(&mut repository.artifacts, chunk.artifacts);
    extend(&mut repository.facets, chunk.facets);
    extend(&mut repository.constraints, chunk.constraints);
    extend(&mut repository.catalogues, chunk.catalogues);
    extend(&mut repository.bindings, chunk.bindings);
}

fn extend<V>(into: &mut std::collections::HashMap<String, V>, from: BTreeMap<String, V>) {
    for (id, value) in from {
        into.insert(id, value);
    }
}

fn corrupt(message: String, hint: &str) -> Diagnostic {
    Diagnostic {
        code: E_LINK_OBJECT_CORRUPT.to_string(),
        severity: DiagnosticSeverity::Error,
        message,
        source_id: None,
        entity_path: None,
        hint: Some(hint.to_string()),
    }
}
