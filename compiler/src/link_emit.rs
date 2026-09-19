// SPDX-License-Identifier: BUSL-1.1

//! Stage 3 of the link, and the one writer both forms of a compile go through
//! (ADR-0058 §D4, §D8).
//!
//! Everything here turns a verified set of chunks into files: the chunk copies,
//! the index built from what they declare, the manifest, and the `.ccm`
//! sibling. `compile` reaches it through [`crate::compiler_core::Compiler`] and
//! `link` reaches it through the objects it loaded, so a package written from
//! `--source` chunks and one written from `--object` directories are the same
//! file set by construction rather than by two writers agreeing.
//!
//! The `.ccm` lives here rather than in the compiler because it is a LINK
//! product: an object stores no constraint model (§D2), and the model is a
//! function of the whole linked set.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ccm_emitter::{
    count_model_variables, emit_ccm_dir, emit_ccm_dir_with_budget, emit_ccm_dir_with_progress,
    ConditionModel, EmitBudgetOutcome,
};
use crate::compiler_core::Compiler;
use crate::ingest_merge::{build_index, IndexInput};
use crate::interface_summary::{Exports, InterfaceSummary};
use crate::ir;
use crate::progress::ProgressTracker;
use crate::resource_budget::{derive_knobs, ResourceBudget};

/// One chunk as the emit consumes it: the canonical file bytes, the ids it
/// declares, and the `source_id` the index records as provenance.
///
/// The bytes are carried rather than a path because both producers already have
/// them — `compile` renders them from the parsed chunk, `link` reads them off
/// the object — and because the emit must be able to fail before it writes
/// anything (§D4: nothing under `--out` unless all three stages pass).
pub(crate) struct LinkChunk {
    pub(crate) chunk_hash: String,
    pub(crate) source_id: String,
    pub(crate) exports: Exports,
    pub(crate) bytes: Vec<u8>,
}

/// The index a package built from these chunks would carry, derived without
/// writing anything.
///
/// Split out of [`write_package`] because the index IS the CMP model identity
/// (`ir::CmpManifest::from_index` reads `config_hash` straight off it), and
/// ADR-0056 Amendment 2 needs a caller that wants the identity WITHOUT the
/// package: standalone `verify` builds it in memory and reports it, so the hash
/// it reports is the hash `compile` emits by construction rather than by two
/// derivations agreeing. `compile` builds it once here and hands it to the
/// write below, so the split costs it nothing.
pub(crate) fn build_package_index(chunks: &[LinkChunk]) -> Result<ir::IrIndex> {
    let inputs: Vec<IndexInput<'_>> = chunks
        .iter()
        .map(|chunk| IndexInput {
            chunk_hash: &chunk.chunk_hash,
            source_id: &chunk.source_id,
            exports: &chunk.exports,
        })
        .collect();
    build_index(&inputs)
}

/// The CMP half of the emit: one `chunk-<hash>.cfir` per chunk, the index built
/// from what those chunks declare, and the manifest.
///
/// Shared with [`Compiler::emit_ir`] so a package written by `compile` and one
/// written by `link` are the same file set by construction rather than by two
/// writers agreeing.
///
/// `index` must be [`build_package_index`] over the same `chunks`. It is taken
/// rather than rebuilt so a caller that already needed the identity does not
/// derive it twice; `build_index` is pure, so passing it in cannot move a
/// written byte.
pub(crate) fn write_package(
    output_dir: &Path,
    chunks: &[LinkChunk],
    index: &ir::IrIndex,
) -> Result<()> {
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create IR output dir '{}'", output_dir.display()))?;

    for chunk in chunks {
        let path = output_dir.join(format!("chunk-{}.cfir", chunk.chunk_hash));
        std::fs::write(&path, &chunk.bytes)
            .with_context(|| format!("Failed to write IR chunk '{}'", path.display()))?;
    }

    let index_path = output_dir.join("index.cfir.json");
    let file = std::fs::File::create(&index_path)
        .with_context(|| format!("Failed to create IR index '{}'", index_path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, index)
        .with_context(|| format!("Failed to write IR index '{}'", index_path.display()))?;
    drop(writer);

    ir::verify_index_integrity(index, output_dir)?;
    let manifest = ir::CmpManifest::from_index(index);
    ir::write_cmp_manifest(&output_dir.join(ir::CMP_DEFAULT_MANIFEST_FILENAME), &manifest)?;
    Ok(())
}

/// This compiler's chunks as link inputs (ADR-0058 §D8).
///
/// `summaries` must be the per-chunk summaries in chunk order — the pairing
/// [`Compiler::emit_ir`] already builds — so each chunk carries the ids it
/// declares without a second walk.
pub(crate) fn link_chunks_of(
    compiler: &Compiler,
    summaries: &[InterfaceSummary],
) -> Result<Vec<LinkChunk>> {
    let chunks = compiler.source_chunks();
    debug_assert_eq!(chunks.len(), summaries.len());
    chunks
        .iter()
        .zip(summaries)
        .map(|(chunk, summary)| {
            let ir_chunk =
                ir::IrChunk::from_config(&chunk.source_id, &chunk.chunk_hash, &chunk.config);
            Ok(LinkChunk {
                chunk_hash: chunk.chunk_hash.clone(),
                source_id: chunk.source_id.clone(),
                exports: summary.exports.clone(),
                bytes: ir::chunk_file_bytes(&ir_chunk)?,
            })
        })
        .collect()
}

/// The `.ccm` sibling emit, with the ADR-0039 soft budget and the ADR-0039 §7
/// progress stream threaded exactly as the compile path has always threaded
/// them. Moved here from `compiler_core` with `link`: the artifact is a LINK
/// product (ADR-0058 §D2 — an object stores no constraint model).
pub(crate) fn emit_ccm_sibling(
    cmp_output_dir: &Path,
    model: &ConditionModel,
    cluster_size: Option<usize>,
    budget: Option<&ResourceBudget>,
    progress: Option<&mut ProgressTracker>,
) -> Result<(PathBuf, EmitBudgetOutcome)> {
    let ccm_dir = cmp_output_dir.join("ccm");

    // With no budget, `memo_cap` stays `None` (byte-identical default apply
    // memos) and no `cluster_size` is derived, so an explicit value flows
    // through unchanged. ADR-0012 Amendment 1: explicit always wins.
    let (effective_cluster_size, memo_cap) = match budget {
        Some(budget) => {
            let hint = count_model_variables(model);
            let derived = derive_knobs(budget, Some(hint));
            (cluster_size.or(derived.cluster_size), Some(derived.memo_cap))
        }
        None => (cluster_size, None),
    };

    // ADR-0039 §5: the operator-facing MiB target becomes the KiB unit
    // `proc_rss` samples. `None` ⇒ no sampling, no shrink, byte-identical.
    let rss_budget_kib = budget
        .and_then(|b| b.max_rss_mb)
        .map(|mb| mb.saturating_mul(1024));

    let mut outcome = match (progress, effective_cluster_size, memo_cap) {
        (Some(tracker), cluster, cap) => emit_ccm_dir_with_progress(
            model,
            &ccm_dir,
            "facet-name-ascending",
            "in-crate",
            cluster.unwrap_or(usize::MAX),
            cap,
            rss_budget_kib,
            tracker,
        ),
        // No progress, no partitioning override, no memo cap, no RSS budget:
        // the exact pre-budget single-partition path (byte-identical).
        (None, None, None) if rss_budget_kib.is_none() => {
            emit_ccm_dir(model, &ccm_dir).map(|()| EmitBudgetOutcome::default())
        }
        (None, cluster, cap) => emit_ccm_dir_with_budget(
            model,
            &ccm_dir,
            "facet-name-ascending",
            "in-crate",
            cluster.unwrap_or(usize::MAX),
            cap,
            rss_budget_kib,
        ),
    }
    .with_context(|| format!("Failed to emit .ccm artifact at '{}'", ccm_dir.display()))?;
    // Recorded only when a budget was in play; the unbudgeted path leaves the
    // outcome zeroed.
    if budget.is_some() {
        outcome.effective_cluster_size = Some(effective_cluster_size.unwrap_or(usize::MAX));
    }
    Ok((ccm_dir, outcome))
}
