// SPDX-License-Identifier: BUSL-1.1

//! A chunk file's name is the hash of the chunk file's content
//! (ADR-0056 Amendment 1).
//!
//! Two properties, asserted over the whole committed corpus rather than over an
//! invented fixture, because the defect these replace was invisible on any
//! single model:
//!
//! 1. **Agreement.** The address the compiler writes (computed at ingest from a
//!    `Config`) and the address a reader recomputes (from the emitted
//!    `IrChunk`) are the same value, for every chunk of every scenario pack and
//!    every shipped example. Before the amendment they could not be: the ingest
//!    preimage carried `package` and `version`, and an emitted chunk carries
//!    neither, so nothing could recompute a chunk's name from the file.
//! 2. **Tamper sensitivity.** Editing one value inside a parsed chunk moves its
//!    recomputed address. This is the primitive `link` drives through
//!    `E_LINK_OBJECT_CORRUPT` (ADR-0058 §D4 stage 3); the reproduction that
//!    forced the amendment was a chunk whose `length_mm` was edited from 1200 to
//!    9999 and which still linked clean under the same `model_hash`.
//!
//! The scenario half of the corpus is the roster
//! [`crate::scenario_byte_stability_tests::SCENARIOS`], under the coverage guard
//! that module already carries, so a new pack cannot land here uncovered
//! without failing there first.

use crate::ir::{chunk_hash_of_chunk, load_chunk, IrChunk};
use crate::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use crate::scenario_byte_stability_tests::SCENARIOS;
use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};
use crate::schema::Value as SchemaValue;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// One shipped example, embedded at compile time so the test stays hermetic.
macro_rules! example {
    ($($path:literal),+ $(,)?) => {
        &[$((concat!("examples/", $path), include_str!(concat!("../../examples/", $path)))),+]
    };
}

/// The examples half of the corpus: every model under `examples/`, compiled the
/// way its own `run.sh` compiles it.
///
/// These matter beyond the packs because they are what the documentation
/// teaches and what an evaluator runs. `06-catalogue-polyrepo` is the only
/// multi-unit model in the repository, so it is the one entry whose four
/// sources are four separate units.
#[rustfmt::skip]
const EXAMPLES: &[ExampleSpec] = &[
    ExampleSpec { key: "00-service-multi-env",  sources: example!("00-service-multi-env/00_definitions.json", "00-service-multi-env/10_components.json") },
    ExampleSpec { key: "01-hello-led",          sources: example!("01-hello-led/config.json") },
    ExampleSpec { key: "02-sensor-gateway",     sources: example!("02-sensor-gateway/00_definitions.json", "02-sensor-gateway/10_components.json") },
    ExampleSpec { key: "03-motor-controller",   sources: example!("03-motor-controller/00_definitions.json", "03-motor-controller/10_components.json") },
    ExampleSpec { key: "04-fleet-edge-node",    sources: example!("04-fleet-edge-node/00_definitions.json", "04-fleet-edge-node/10_components.json") },
    ExampleSpec { key: "05-compose-fleet",      sources: example!("05-compose-fleet/00_definitions.json", "05-compose-fleet/10_components.json") },
    ExampleSpec { key: "06-catalogue-polyrepo", sources: example!(
        "06-catalogue-polyrepo/repos/catalogue/00_catalogue.json",
        "06-catalogue-polyrepo/repos/vision/10_vision.json",
        "06-catalogue-polyrepo/repos/compute/10_compute.json",
        "06-catalogue-polyrepo/repos/sorter/20_sorter.json",
    ) },
];

#[derive(Clone, Copy)]
struct ExampleSpec {
    key: &'static str,
    sources: &'static [(&'static str, &'static str)],
}

fn manifest_for(sources: &[(&str, &str)]) -> Vec<SourceManifestEntry> {
    sources
        .iter()
        .map(|(source_id, content)| SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*content).to_string(),
        })
        .collect()
}

/// Compile one corpus member and return the directory the package was written
/// to, together with the guard that removes it.
fn compile_corpus_member(label: &str, sources: &[(&str, &str)]) -> Result<TempDirGuard> {
    let out_dir = unique_temp_dir("configflux-chunk-address", label)?;
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest_for(sources),
        output_dir: Some(out_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    if result.status != OperationStatus::Ok {
        return Err(anyhow!(
            "[{label}] compile_model failed: {:?}",
            result.verify_report.diagnostics.diagnostics
        ));
    }
    Ok(out_dir)
}

/// Every `chunk-<hash>.cfir` the package directory holds, in name order.
fn emitted_chunk_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read package dir '{}'", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("chunk-") && name.ends_with(".cfir"))
        })
        .collect();
    paths.sort();
    Ok(paths)
}

/// The hash a chunk file's NAME claims: `chunk-<hash>.cfir`.
fn hash_in_filename(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("Chunk path '{}' has no usable file name", path.display()))?;
    name.strip_prefix("chunk-")
        .and_then(|rest| rest.strip_suffix(".cfir"))
        .map(str::to_string)
        .with_context(|| format!("Chunk file '{name}' is not named chunk-<hash>.cfir"))
}

/// Assert the three-way agreement for one package directory, accumulating
/// failures so one run reports every drifted chunk in the corpus rather than
/// only the first.
fn check_package(label: &str, dir: &Path, failures: &mut Vec<String>) -> Result<usize> {
    let chunk_paths = emitted_chunk_files(dir)?;
    if chunk_paths.is_empty() {
        return Err(anyhow!("[{label}] package emitted no chunk files"));
    }
    for path in &chunk_paths {
        let named = hash_in_filename(path)?;
        let chunk: IrChunk = load_chunk(path)
            .with_context(|| format!("[{label}] failed to parse '{}'", path.display()))?;
        let recomputed = chunk_hash_of_chunk(&chunk)
            .with_context(|| format!("[{label}] failed to recompute '{}'", path.display()))?;

        if recomputed != named {
            failures.push(format!(
                "[{label}] recomputed address does not match the file name\n  \
                 file name: {named}\n  recomputed: {recomputed}"
            ));
        }
        if chunk.chunk_hash != named {
            failures.push(format!(
                "[{label}] embedded chunk_hash does not match the file name\n  \
                 file name: {named}\n  embedded:  {}",
                chunk.chunk_hash
            ));
        }
    }
    Ok(chunk_paths.len())
}

/// T2. For every chunk of every scenario pack and every shipped example, the
/// address recomputed from the chunk file equals the name the compiler gave it
/// and equals the value embedded inside it.
#[test]
fn every_emitted_chunk_hashes_to_its_own_name() -> Result<()> {
    let mut failures: Vec<String> = Vec::new();
    let mut chunks_checked = 0usize;

    for spec in SCENARIOS {
        let guard = compile_corpus_member(spec.key, spec.chunks)?;
        chunks_checked += check_package(spec.key, &guard.path, &mut failures)?;
    }
    for spec in EXAMPLES {
        let guard = compile_corpus_member(spec.key, spec.sources)?;
        chunks_checked += check_package(spec.key, &guard.path, &mut failures)?;
    }

    assert!(
        chunks_checked >= SCENARIOS.len() + EXAMPLES.len(),
        "corpus walk checked only {chunks_checked} chunks — the roster or the \
         emitted layout changed"
    );
    assert!(
        failures.is_empty(),
        "{} chunk file(s) do not hash to their own name. A chunk's address is \
         its content's SHA-256 (ADR-0056 Amendment 1), so a mismatch means the \
         ingest-side and verification-side preimages have diverged.\n\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

/// T2, second half. The two constructors agree by construction — they share one
/// preimage type — and this pins that they still do on real content, so a
/// future edit that reintroduces a second projection fails here.
#[test]
fn the_ingest_address_and_the_verified_address_agree() -> Result<()> {
    let spec = SCENARIOS
        .iter()
        .find(|s| s.key == "s1-smoke")
        .context("s1-smoke missing from the scenario roster")?;
    let guard = compile_corpus_member("agreement", spec.chunks)?;

    for path in emitted_chunk_files(&guard.path)? {
        let chunk = load_chunk(&path)?;
        // `chunk.chunk_hash` is what `chunk_hash_from_config` produced at
        // ingest; `chunk_hash_of_chunk` is the verification-side constructor.
        assert_eq!(
            chunk_hash_of_chunk(&chunk)?,
            chunk.chunk_hash,
            "ingest and verification disagree on '{}'",
            path.display()
        );
    }
    Ok(())
}

/// T3. The tamper primitive: change one parameter value inside a parsed chunk
/// and its recomputed address moves. `link` drives this through
/// `E_LINK_OBJECT_CORRUPT` (configflux-p0jz.2); here it is asserted on the
/// primitive alone, so a regression is attributed to the address rather than to
/// the linker.
#[test]
fn editing_a_value_inside_a_chunk_moves_its_address() -> Result<()> {
    let spec = SCENARIOS
        .iter()
        .find(|s| s.key == "s1-smoke")
        .context("s1-smoke missing from the scenario roster")?;
    let guard = compile_corpus_member("tamper", spec.chunks)?;

    let mut tampered = 0usize;
    for path in emitted_chunk_files(&guard.path)? {
        let mut chunk = load_chunk(&path)?;
        let before = chunk_hash_of_chunk(&chunk)?;
        assert_eq!(before, chunk.chunk_hash, "fixture chunk is already corrupt");

        // Edit ONE authored value, leaving `chunk_hash`, `source_id` and the
        // file name exactly as they were — the shape of the real tamper.
        let Some((_, definition)) = chunk.definitions.iter_mut().next() else {
            continue;
        };
        // Chosen against what is there, so the edit is a change whatever the
        // fixture happens to declare.
        let edited = match definition.value.as_ref() {
            Some(SchemaValue::Integer(9999)) => SchemaValue::Integer(1200),
            _ => SchemaValue::Integer(9999),
        };
        assert_ne!(
            definition.value.as_ref(),
            Some(&edited),
            "the tamper must change the value it replaces"
        );
        definition.value = Some(edited);

        let after = chunk_hash_of_chunk(&chunk)?;
        assert_ne!(
            after, before,
            "an edited parameter value left the address unchanged in '{}'",
            path.display()
        );
        assert_ne!(
            after, chunk.chunk_hash,
            "an edited chunk still hashes to the address it claims in '{}'",
            path.display()
        );
        tampered += 1;
    }

    assert!(
        tampered > 0,
        "no chunk in the fixture carried a definition to edit"
    );
    Ok(())
}

/// Coverage guard for the examples half of the corpus, mirroring
/// `scenario_byte_stability_every_pack_under_scenarios_is_covered` for the packs
/// half. A new model under `examples/` must be added to EXAMPLES and to this
/// ledger in the same change, or the chunk-address invariant silently stops
/// covering what the documentation teaches.
#[test]
fn every_example_source_under_examples_is_covered() {
    #[rustfmt::skip]
    const EXPECTED_EXAMPLE_SOURCES: &[&str] = &[
        "examples/00-service-multi-env/00_definitions.json",
        "examples/00-service-multi-env/10_components.json",
        "examples/01-hello-led/config.json",
        "examples/02-sensor-gateway/00_definitions.json",
        "examples/02-sensor-gateway/10_components.json",
        "examples/03-motor-controller/00_definitions.json",
        "examples/03-motor-controller/10_components.json",
        "examples/04-fleet-edge-node/00_definitions.json",
        "examples/04-fleet-edge-node/10_components.json",
        "examples/05-compose-fleet/00_definitions.json",
        "examples/05-compose-fleet/10_components.json",
        "examples/06-catalogue-polyrepo/repos/catalogue/00_catalogue.json",
        "examples/06-catalogue-polyrepo/repos/compute/10_compute.json",
        "examples/06-catalogue-polyrepo/repos/sorter/20_sorter.json",
        "examples/06-catalogue-polyrepo/repos/vision/10_vision.json",
    ];

    let mut declared: Vec<&str> = EXAMPLES
        .iter()
        .flat_map(|spec| spec.sources.iter().map(|(source_id, _)| *source_id))
        .collect();
    declared.sort_unstable();

    assert_eq!(
        declared, EXPECTED_EXAMPLE_SOURCES,
        "EXAMPLES drifted from the expected ledger. If a model was added under \
         examples/, add it to both EXAMPLES and EXPECTED_EXAMPLE_SOURCES in the \
         same change."
    );
}
