// SPDX-License-Identifier: BUSL-1.1

use crate::interface_summary::Exports;
use crate::ir;
use anyhow::Result;
use std::collections::BTreeMap;

// NOTE: the standalone public `parse_config` (a thin `toml::from_str` wrapper)
// was retired in L1 phase 8 (ADR 0021) when CUE became the canonical authoring
// surface. The 22 TOML *scenario chunks* were retired in L2 Track E (ADR 0027,
// configflux-u40u): CUE-JSON is now the sole authored scenario chunk format.
//
// In B-5 (ADR-0027 Track B, configflux-73fr) the hand-rolled Rust merge/inherit
// engine was deleted now that CUE owns `inherits` resolution and cross-file
// unification (proven byte-equal by the B-4 equivalence corpus): this file's
// `merge_components` (cross-file conflict logic) and `validate_snake_case_ids`
// (moved to `schema.cue` `#snakeId`, Decision 4) are gone. Cross-file component
// overlap is now a duplicate error (`Compiler::merge_partial`), not a merge.
//
// The TOML *parse* path (`Compiler::add_chunk_with_source`, reached via
// `add_chunk_auto`'s content router) is deliberately RETAINED: the scenario
// *mutation* error-fixtures under `scenarios/**/mutations/*.toml` (and inline
// mutation tests) are still authored in TOML and fed through `add_chunk_auto`,
// so the `toml` crate stays. Only the inline `add_chunk` convenience wrapper —
// whose sole callers were the now-deleted `lib_tests.rs` merge tests — was
// removed alongside the engine.
//
// CONSTRAINED, not deleted (configflux-qofj): authored TOML may no longer carry
// `inherits`. `add_chunk_with_source` rejects it at ingest with a deterministic
// CUE-authoring diagnostic, because the resolve-time inheritance engine is gone
// and a TOML `inherits` would silently default safety/lifecycle/access at
// resolve. The retained mutation fixtures are inherits-free, so this guard does
// not narrow the retention surface. (ADR-0027 Decision 9 erratum is finalized
// in configflux-07ot.)
/// One chunk's contribution to the package index: its content address, the
/// `source_id` the index keeps as provenance (ADR-0056 §5), and the ids it
/// declares.
///
/// The ids arrive as an [`Exports`] rather than as a parsed chunk because the
/// two producers hold different things — `compile` has the authored `Config`
/// and its summary, `link` has an emitted `IrChunk` read back off an object —
/// and the index needs only the id sets both can supply.
pub(crate) struct IndexInput<'a> {
    pub(crate) chunk_hash: &'a str,
    pub(crate) source_id: &'a str,
    pub(crate) exports: &'a Exports,
}

/// Build the emitted index from what each chunk declares.
///
/// The one index builder, reached by both forms of a compile through
/// [`crate::link_emit::write_package`], so a package written from `--source`
/// chunks and one written from `--object` directories carry the same index by
/// construction (ADR-0058 §D8).
///
/// The one-entity-one-chunk invariant this function used to enforce inline —
/// four hand-rolled roster checks, one per namespace — is
/// `link_verify::validate_link_summary` over the merged interface summaries
/// (ADR-0057 §D9), which every caller runs before reaching here. Re-running it
/// would report a fault a second time under a worse message.
pub(crate) fn build_index(inputs: &[IndexInput<'_>]) -> Result<ir::IrIndex> {
    let mut chunk_refs = Vec::new();
    let mut component_index = BTreeMap::new();
    let mut definition_index = BTreeMap::new();
    let mut artifact_index = BTreeMap::new();
    let mut facet_index = BTreeMap::new();
    let mut catalogue_index = BTreeMap::new();
    let mut binding_index = BTreeMap::new();

    for chunk in inputs {
        chunk_refs.push(ir::IrChunkRef {
            chunk_hash: chunk.chunk_hash.to_string(),
            source_id: chunk.source_id.to_string(),
        });

        for component_id in &chunk.exports.components {
            component_index.insert(component_id.clone(), chunk.chunk_hash.to_string());
        }
        for definition_id in &chunk.exports.definitions {
            definition_index.insert(definition_id.clone(), chunk.chunk_hash.to_string());
        }
        for artifact_id in &chunk.exports.artifacts {
            artifact_index.insert(artifact_id.clone(), chunk.chunk_hash.to_string());
        }
        // Facets, catalogues and bindings are pack-global namespaces declared by
        // at most one chunk (ADR-0047 §2, ADR-0057 §D2/§D3). Each index feeds
        // the `model_hash` preimage, symmetric with the three above.
        for facet_id in &chunk.exports.facets {
            facet_index.insert(facet_id.clone(), chunk.chunk_hash.to_string());
        }
        for catalogue_id in &chunk.exports.catalogues {
            catalogue_index.insert(catalogue_id.clone(), chunk.chunk_hash.to_string());
        }
        for binding_id in &chunk.exports.bindings {
            binding_index.insert(binding_id.clone(), chunk.chunk_hash.to_string());
        }
    }

    // ADR-0056 §2: order by `chunk_hash` ascending, lexicographic over the
    // lowercase hex string. The alphabet is [0-9a-f], so byte order and
    // codepoint order coincide and no locale-aware comparison is involved.
    //
    // The key USED to be `(source_id, chunk_hash)`. That normalized `--source`
    // argument order, but it made the emitted order — and therefore the
    // `model_hash` preimage's order — a function of how the sources were
    // spelled. Removing `source_id` from the preimage without re-keying this
    // sort would have left path spelling in the identity through the ordering
    // alone. No tiebreak is needed: `add_parsed_chunk` rejects a repeated
    // `chunk_hash` at ingest (§3), so the key is unique.
    //
    // Sorted exactly once, here. Both the emitted `IrIndex.chunks` and the
    // preimage vector derive from this one result.
    chunk_refs.sort_by(|a, b| a.chunk_hash.cmp(&b.chunk_hash));

    ir::IrIndex::from_parts(
        chunk_refs,
        component_index,
        definition_index,
        artifact_index,
        facet_index,
        catalogue_index,
        binding_index,
    )
}
