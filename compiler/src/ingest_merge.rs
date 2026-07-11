// SPDX-License-Identifier: BUSL-1.1

use crate::compiler_core::SourceChunk;
use crate::ir;
use anyhow::{bail, Result};
use std::collections::{BTreeMap, HashMap};

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
pub(crate) fn build_ir_index(chunks: &[SourceChunk]) -> Result<ir::IrIndex> {
    let mut chunk_refs = Vec::new();
    let mut component_index = BTreeMap::new();
    let mut definition_index = BTreeMap::new();
    let mut artifact_index = BTreeMap::new();
    let mut facet_index = BTreeMap::new();
    let mut component_sources: HashMap<String, String> = HashMap::new();
    let mut definition_sources: HashMap<String, String> = HashMap::new();
    let mut artifact_sources: HashMap<String, String> = HashMap::new();
    let mut facet_sources: HashMap<String, String> = HashMap::new();

    for chunk in chunks {
        chunk_refs.push(ir::IrChunkRef {
            chunk_hash: chunk.chunk_hash.clone(),
            source_id: chunk.source_id.clone(),
        });

        for component_id in chunk.config.components.keys() {
            if let Some(existing) =
                component_sources.insert(component_id.clone(), chunk.source_id.clone())
            {
                bail!(
                    "Component '{}' appears in multiple chunks: '{}' and '{}'",
                    component_id,
                    existing,
                    chunk.source_id
                );
            }
            component_index.insert(component_id.clone(), chunk.chunk_hash.clone());
        }

        for definition_id in chunk.config.definitions.keys() {
            if let Some(existing) =
                definition_sources.insert(definition_id.clone(), chunk.source_id.clone())
            {
                bail!(
                    "Definition '{}' appears in multiple chunks: '{}' and '{}'",
                    definition_id,
                    existing,
                    chunk.source_id
                );
            }
            definition_index.insert(definition_id.clone(), chunk.chunk_hash.clone());
        }

        for artifact_id in chunk.config.artifacts.keys() {
            if let Some(existing) =
                artifact_sources.insert(artifact_id.clone(), chunk.source_id.clone())
            {
                bail!(
                    "Artifact '{}' appears in multiple chunks: '{}' and '{}'",
                    artifact_id,
                    existing,
                    chunk.source_id
                );
            }
            artifact_index.insert(artifact_id.clone(), chunk.chunk_hash.clone());
        }

        // Facets are a pack-global namespace declared by at most one chunk
        // (ADR-0047 §2). The cross-chunk uniqueness invariant is enforced at
        // ingest merge (E_INGEST_DUPLICATE_FACET); this loop is the structural
        // one-facet-one-chunk guarantee that feeds `facet_index` into the
        // `model_hash` preimage, symmetric with the other entity indices.
        for facet_id in chunk.config.facets.keys() {
            if let Some(existing) = facet_sources.insert(facet_id.clone(), chunk.source_id.clone()) {
                bail!(
                    "Facet '{}' is declared in more than one chunk: '{}' and '{}'",
                    facet_id,
                    existing,
                    chunk.source_id
                );
            }
            facet_index.insert(facet_id.clone(), chunk.chunk_hash.clone());
        }
    }

    chunk_refs.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then_with(|| a.chunk_hash.cmp(&b.chunk_hash))
    });

    ir::IrIndex::from_parts(
        chunk_refs,
        component_index,
        definition_index,
        artifact_index,
        facet_index,
    )
}
