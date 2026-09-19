// SPDX-License-Identifier: BUSL-1.1

use crate::schema::{
    Artifact, Binding, Catalogue, Component, Config, Constraint, Facet, Parameter,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

// Bumped 1 -> 2 (ADR-0047): `IrIndex.facet_index` joins the `model_hash`
// preimage (it is a field of `IrIndexContent`), so every model's `model_hash`
// rotates once this release — whether or not it declares a facet.
//
// Bumped 2 -> 3 (ADR-0054 §4/§7): `IrChunk` gains the `constraints` namespace,
// so an emitted chunk's SHAPE changed. `verify_index_integrity` compares each
// chunk's `format_version` against this constant, and that comparison is the
// ONLY barrier standing between a v4 toolchain and a pre-constraints CMP
// package: `chunk_hash` is read from the index rather than recomputed from
// chunk bytes, so a stale package is otherwise self-consistent and would load.
// Without this bump a v3 package would be silently read as constraint-free —
// precisely the `constraints`-absent fallback ADR-0054 §7 forbids, and it would
// falsify §7's guarantee that "there is no window in which the same bytes mean
// two different things". `PRODUCT_SCHEMA_VERSION` guards the REQUEST; this
// guards the PACKAGE.
//
// Bumped 3 -> 4 (ADR-0057 §D9): `IrChunk` gains the `catalogues` and
// `bindings` namespaces and `IrIndex` gains `catalogue_index` and
// `binding_index`, which join the `model_hash` preimage exactly as
// `facet_index` did — so every model's `model_hash` rotates this release
// whether or not it declares either. The two counters MOVE INDEPENDENTLY
// here: `PRODUCT_SCHEMA_VERSION` stays 4 because no REQUEST or RESULT shape
// changed (it advances in configflux-secb.6, with the snapshot's `requires`
// block). The gates are genuinely separate — `open_model` compares the
// request's `schema_version` against `PRODUCT_SCHEMA_VERSION`, while a
// package is screened by `verify_index_integrity`'s per-chunk comparison
// against THIS constant — so a v3 package is rejected on load without
// touching the request contract.
pub const IR_FORMAT_VERSION: u32 = 4;
pub const CMP_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CMP_HASH_ALGO: &str = "sha256";
// Bumped 1 -> 2 (ADR-0056 §4): this constant is precisely the counter for "the
// rule that produced `model_hash`", and that rule changed — `source_id` left
// the preimage and the chunk vector is now ordered by `chunk_hash`. Without the
// bump a package canonicalized under rule v1 falls through to the index
// recompute in `open_model` and is rejected as E_LOADER_INDEX_INVALID hinting
// "Do not mutate emitted index files; recompile instead" — a tampering
// accusation against a package that is internally consistent and was simply
// built by the previous toolchain. `validate_manifest` checks this fail-closed
// before anything else is read, so the bump makes the rejection state the true
// reason at the correct gate.
//
// IR_FORMAT_VERSION deliberately does NOT move with it: neither `IrChunk` nor
// `IrIndex` changed SHAPE, which is what that counter guards.
//
// Bumped 2 -> 3 (ADR-0056 Amendment 1 §Decision 4): the rule that produces
// `chunk_hash` — and through it `model_hash` — changed again. The preimage was
// the authored `Config`, `package` and `version` included; it is now the
// chunk's seven entity maps and nothing else, so a chunk file's name is
// recomputable from the file by any reader. Every `chunk_hash`, `model_hash`,
// `resolve_hash`, `selection_state_hash` and `bom_hash` rotates once with this
// bump, and a v2 package is refused at the manifest gate for the reason above
// rather than accused of tampering one stage later.
//
// The other three counters hold, by §4's own rule: IR_FORMAT_VERSION stays 4
// (neither `IrChunk` nor `IrIndex` changed shape), PRODUCT_SCHEMA_VERSION stays
// 5 (no request or result shape changed) and OBJECT_FORMAT_VERSION stays 1 (the
// object header is unchanged — its `chunk_hashes` VALUES rotate, its shape does
// not).
pub const CMP_CANONICALIZATION_VERSION: u32 = 3;
pub const CMP_DEFAULT_MANIFEST_FILENAME: &str = "cmp.manifest.json";
pub const CMP_DEFAULT_INDEX_REF: &str = "index.cfir.json";
pub const CMP_DEFAULT_CHUNK_SET_REF: &str = ".";

// `Clone` (configflux-8nhr): a reader that takes the seven namespaces out of a
// chunk by value needs its own copy, now that one parsed chunk is shared by
// every builder on a call instead of each builder re-reading the file for
// itself. Cloning the parsed value is what replaces that second read.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IrChunk {
    pub format_version: u32,
    pub chunk_hash: String,
    pub source_id: String,
    pub definitions: BTreeMap<String, Parameter>,
    pub components: BTreeMap<String, Component>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, Artifact>,
    // Facet declarations authored in this chunk (ADR-0047). A facet is declared
    // by at most one chunk; the cross-chunk uniqueness invariant is enforced in
    // `build_ir_index` (E_INGEST_DUPLICATE_FACET).
    #[serde(default)]
    pub facets: BTreeMap<String, Facet>,
    // Policy assertions authored in this chunk (ADR-0054 §4). Carried so
    // `load_selection_constraint_model` can read constraints on the same walk
    // that already reads `facets` / `components` / `definitions`. A `BTreeMap`
    // keeps the per-chunk walk id-ascending, which is the order
    // `SelectionConstraintModel.constraints` commits to.
    #[serde(default)]
    pub constraints: BTreeMap<String, Constraint>,
    // Typed catalogues authored in this chunk (ADR-0057 §D2). Carried so a
    // binding's value domain — the catalogue's entry ids — is reachable on
    // the same walk that already reads `facets`, without a second pass over
    // the package. A catalogue is declared by at most one chunk
    // (`E_INGEST_DUPLICATE_CATALOGUE`).
    #[serde(default)]
    pub catalogues: BTreeMap<String, Catalogue>,
    // Bindings authored in this chunk (ADR-0057 §D3), carried VERBATIM —
    // `derive` tables included. The lowering of `derive` to root conjuncts
    // lands with `requires` (configflux-secb.5); until then the table has to
    // survive the round trip, or the package would silently mean less than
    // the author wrote.
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IrChunkRef {
    pub chunk_hash: String,
    pub source_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IrIndex {
    pub format_version: u32,
    pub chunks: Vec<IrChunkRef>,
    pub component_index: BTreeMap<String, String>,
    pub definition_index: BTreeMap<String, String>,
    #[serde(default)]
    pub artifact_index: BTreeMap<String, String>,
    // Facet declaration -> owning chunk hash (ADR-0047). Enters the canonical
    // `model_hash` preimage exactly as the other entity indices do.
    #[serde(default)]
    pub facet_index: BTreeMap<String, String>,
    // Catalogue / binding declaration -> owning chunk hash (ADR-0057 §D9).
    // Both enter the canonical `model_hash` preimage exactly as the other
    // entity indices do.
    #[serde(default)]
    pub catalogue_index: BTreeMap<String, String>,
    #[serde(default)]
    pub binding_index: BTreeMap<String, String>,
    pub config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmpManifestStats {
    pub source_count: u32,
    pub chunk_count: u32,
    pub definition_count: u32,
    pub component_count: u32,
    pub artifact_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmpManifest {
    pub schema_version: u32,
    pub model_hash: String,
    pub ir_format_version: u32,
    pub index_ref: String,
    pub chunk_set_ref: String,
    pub config_hash: String,
    pub hash_algo: String,
    pub canonicalization_version: u32,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<CmpManifestStats>,
}

/// The `model_hash` preimage (ADR-0056 §1). Serialized with
/// `serde_json::to_vec` and hashed by [`hash_index_content`].
///
/// `chunks` is a `Vec<String>` of bare `chunk_hash` values, NOT the
/// [`IrChunkRef`] the index carries on disk: `IrChunkRef.source_id` is the
/// literal `--source` argument, and it was the single route by which the
/// command line reached model identity. Two checkouts of one commit disagreed
/// about the identity of one model, and moving an unedited file rotated it.
/// The standing invariant that replaces it: **no path string may enter any
/// hash preimage.**
///
/// The vector arrives sorted ascending by `chunk_hash` (§2) — sorted once in
/// `build_ir_index` and never re-sorted here, so a reordered on-disk index
/// still fails `open_model`'s recompute instead of being silently accepted.
#[derive(Serialize)]
struct IrIndexContent {
    format_version: u32,
    chunks: Vec<String>,
    component_index: BTreeMap<String, String>,
    definition_index: BTreeMap<String, String>,
    artifact_index: BTreeMap<String, String>,
    facet_index: BTreeMap<String, String>,
    catalogue_index: BTreeMap<String, String>,
    binding_index: BTreeMap<String, String>,
}

/// Project chunk refs onto the preimage's bare-hash vector, preserving order.
fn preimage_chunks(chunks: &[IrChunkRef]) -> Vec<String> {
    chunks.iter().map(|c| c.chunk_hash.clone()).collect()
}

/// The seven entity namespaces a chunk carries, projected out of a [`Config`]'s
/// `HashMap`s into the `BTreeMap`s an [`IrChunk`] holds.
///
/// This is the ONE projection from authored config to chunk content. Both the
/// emitted chunk ([`IrChunk::from_config`]) and the address computed at ingest
/// ([`chunk_hash_from_config`]) are built from it, so there is no second place
/// for the two to disagree about what a chunk contains.
struct ChunkEntities {
    definitions: BTreeMap<String, Parameter>,
    components: BTreeMap<String, Component>,
    artifacts: BTreeMap<String, Artifact>,
    facets: BTreeMap<String, Facet>,
    constraints: BTreeMap<String, Constraint>,
    catalogues: BTreeMap<String, Catalogue>,
    bindings: BTreeMap<String, Binding>,
}

impl ChunkEntities {
    fn from_config(config: &Config) -> Self {
        Self {
            definitions: config
                .definitions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            components: config
                .components
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            artifacts: config
                .artifacts
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            facets: config
                .facets
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            constraints: config
                .constraints
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            catalogues: config
                .catalogues
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            bindings: config
                .bindings
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    fn preimage(&self) -> ChunkPreimage<'_> {
        ChunkPreimage {
            definitions: &self.definitions,
            components: &self.components,
            artifacts: &self.artifacts,
            facets: &self.facets,
            constraints: &self.constraints,
            catalogues: &self.catalogues,
            bindings: &self.bindings,
        }
    }
}

/// The `chunk_hash` preimage (ADR-0056 Amendment 1): the chunk's seven entity
/// maps and NOTHING else.
///
/// Borrowed rather than owned so the two constructors — from a [`Config`] at
/// ingest, from an [`IrChunk`] at verification — share one type and one
/// encoding. A second preimage struct would be a second chance for the two
/// sides to drift, which is precisely the defect this replaces: the old
/// preimage was the authored `Config`, including `package` and `version`, and
/// an emitted chunk file carries neither — so no reader could recompute the
/// name of the file in front of it.
///
/// `package` is the UNIT's name and lives in `ObjectHeader.unit`; `version` is a
/// label that reaches no output. Neither is chunk content, so neither is here.
#[derive(Serialize)]
struct ChunkPreimage<'a> {
    definitions: &'a BTreeMap<String, Parameter>,
    components: &'a BTreeMap<String, Component>,
    artifacts: &'a BTreeMap<String, Artifact>,
    facets: &'a BTreeMap<String, Facet>,
    constraints: &'a BTreeMap<String, Constraint>,
    catalogues: &'a BTreeMap<String, Catalogue>,
    bindings: &'a BTreeMap<String, Binding>,
}

/// Canonical bytes of a chunk preimage, then SHA-256.
///
/// The recipe is `serde_json::to_value` then `to_string` — key-ascending, no
/// whitespace — so nested values canonicalize exactly as they have since
/// ADR-0021 and the same logical chunk hashes identically whether it was
/// authored as TOML or as CUE-emitted JSON.
fn hash_chunk_preimage(preimage: &ChunkPreimage<'_>) -> Result<String> {
    let value =
        serde_json::to_value(preimage).context("Failed to serialize chunk preimage for hashing")?;
    let canonical =
        serde_json::to_string(&value).context("Failed to canonicalize chunk preimage for hashing")?;
    Ok(sha256_hex(canonical.as_bytes()))
}

impl IrChunk {
    pub fn from_config(source_id: &str, chunk_hash: &str, config: &Config) -> Self {
        let ChunkEntities {
            definitions,
            components,
            artifacts,
            facets,
            constraints,
            catalogues,
            bindings,
        } = ChunkEntities::from_config(config);

        Self {
            format_version: IR_FORMAT_VERSION,
            chunk_hash: chunk_hash.to_string(),
            source_id: source_id.to_string(),
            definitions,
            components,
            artifacts,
            facets,
            constraints,
            catalogues,
            bindings,
            metadata: None,
        }
    }

    /// This chunk's preimage, over the maps the chunk itself carries.
    fn preimage(&self) -> ChunkPreimage<'_> {
        ChunkPreimage {
            definitions: &self.definitions,
            components: &self.components,
            artifacts: &self.artifacts,
            facets: &self.facets,
            constraints: &self.constraints,
            catalogues: &self.catalogues,
            bindings: &self.bindings,
        }
    }
}

impl IrIndex {
    pub fn from_parts(
        chunks: Vec<IrChunkRef>,
        component_index: BTreeMap<String, String>,
        definition_index: BTreeMap<String, String>,
        artifact_index: BTreeMap<String, String>,
        facet_index: BTreeMap<String, String>,
        catalogue_index: BTreeMap<String, String>,
        binding_index: BTreeMap<String, String>,
    ) -> Result<Self> {
        let content = IrIndexContent {
            format_version: IR_FORMAT_VERSION,
            chunks: preimage_chunks(&chunks),
            component_index,
            definition_index,
            artifact_index,
            facet_index,
            catalogue_index,
            binding_index,
        };
        let config_hash = hash_index_content(&content)?;
        Ok(Self {
            format_version: content.format_version,
            // The refs as given — `source_id` is kept on disk as provenance
            // (ADR-0056 §5) and is what ADR-0054 §6 reads to name the chunk
            // that declared a rejected constraint.
            chunks,
            component_index: content.component_index,
            definition_index: content.definition_index,
            artifact_index: content.artifact_index,
            facet_index: content.facet_index,
            catalogue_index: content.catalogue_index,
            binding_index: content.binding_index,
            config_hash,
        })
    }

    pub fn compute_config_hash(&self) -> Result<String> {
        let content = IrIndexContent {
            format_version: self.format_version,
            chunks: preimage_chunks(&self.chunks),
            component_index: self.component_index.clone(),
            definition_index: self.definition_index.clone(),
            artifact_index: self.artifact_index.clone(),
            facet_index: self.facet_index.clone(),
            catalogue_index: self.catalogue_index.clone(),
            binding_index: self.binding_index.clone(),
        };
        hash_index_content(&content)
    }
}

impl CmpManifest {
    pub fn from_index(index: &IrIndex) -> Self {
        Self {
            schema_version: CMP_MANIFEST_SCHEMA_VERSION,
            model_hash: index.config_hash.clone(),
            ir_format_version: index.format_version,
            index_ref: CMP_DEFAULT_INDEX_REF.to_string(),
            chunk_set_ref: CMP_DEFAULT_CHUNK_SET_REF.to_string(),
            config_hash: index.config_hash.clone(),
            hash_algo: CMP_HASH_ALGO.to_string(),
            canonicalization_version: CMP_CANONICALIZATION_VERSION,
            // Keep deterministic output for identical input models.
            created_at: "1970-01-01T00:00:00Z".to_string(),
            stats: Some(CmpManifestStats {
                source_count: index.chunks.len() as u32,
                chunk_count: index.chunks.len() as u32,
                definition_count: index.definition_index.len() as u32,
                component_count: index.component_index.len() as u32,
                artifact_count: index.artifact_index.len() as u32,
            }),
        }
    }
}

// Per-thread tallies of the two package reads, for the tests that PIN how many
// times one operation reads a package (configflux-8nhr).
//
// Thread-local rather than a process-wide counter: the Rust harness runs
// `#[test]` functions on parallel threads and dozens of them load packages, so
// a shared counter's delta across one measured call would be whatever the rest
// of the binary happened to be doing at the time — and a lock around the
// measured section cannot fix that, because the tests inflating it do not hold
// the lock. Every read on the load path happens on the thread that asked for
// it, so a per-thread tally is exact by construction; a future change that
// moved chunk reading onto a worker thread would read zero here and fail the
// pin loudly rather than drift back into silently repeated I/O.
#[cfg(test)]
thread_local! {
    /// Calls to [`load_index`] on this thread.
    pub(crate) static INDEX_LOADS: std::cell::Cell<u64> = std::cell::Cell::new(0);
    /// Calls to [`load_chunk`] on this thread.
    pub(crate) static CHUNK_LOADS: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

pub fn load_index(path: impl AsRef<Path>) -> Result<IrIndex> {
    #[cfg(test)]
    INDEX_LOADS.with(|count| count.set(count.get() + 1));
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read IR index '{}'", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse IR index '{}'", path.display()))
}

pub fn load_chunk(path: impl AsRef<Path>) -> Result<IrChunk> {
    #[cfg(test)]
    CHUNK_LOADS.with(|count| count.set(count.get() + 1));
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read IR chunk '{}'", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse IR chunk '{}'", path.display()))
}

pub fn load_cmp_manifest(path: impl AsRef<Path>) -> Result<CmpManifest> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read CMP manifest '{}'", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse CMP manifest '{}'", path.display()))
}

pub fn write_cmp_manifest(path: impl AsRef<Path>, manifest: &CmpManifest) -> Result<()> {
    let path = path.as_ref();
    let file = std::fs::File::create(path)
        .with_context(|| format!("Failed to create CMP manifest '{}'", path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, manifest)
        .with_context(|| format!("Failed to write CMP manifest '{}'", path.display()))
}

/// A chunk file whose content does not hash to the address the package names it
/// by (ADR-0056 Amendment 1).
///
/// Carried as a typed error rather than as a bare message so a caller can offer
/// the remedy that fits. Every other integrity failure is a package that is
/// incomplete or disagrees with itself, and "restore the missing file" is not
/// the same advice as "recompile". The diagnostic CODE is unaffected:
/// `E_LOADER_INDEX_INVALID` is the package-integrity code and this is a
/// package-integrity failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkContentAddressMismatch {
    /// The address the package stores for the chunk — its file name, which the
    /// `chunk_hash` field inside the file agrees with.
    pub stored: String,
    /// The address the chunk's own entity maps hash to.
    pub recomputed: String,
}

impl std::fmt::Display for ChunkContentAddressMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Chunk 'chunk-{}.cfir' does not hash to the address it is stored \
             under: stored '{}', recomputed '{}'",
            self.stored, self.stored, self.recomputed
        )
    }
}

impl std::error::Error for ChunkContentAddressMismatch {}

/// Integrity-check a package and discard what the walk read.
///
/// Every check, every order, and every message is
/// [`verify_index_integrity_loading`]'s — this is that function with its result
/// dropped, for the callers that only want the verdict.
pub fn verify_index_integrity(index: &IrIndex, chunk_dir: impl AsRef<Path>) -> Result<()> {
    verify_index_integrity_loading(index, chunk_dir).map(|_| ())
}

/// Integrity-check a package and KEEP the chunks the walk parsed, in
/// `index.chunks` order (configflux-8nhr).
///
/// The walk already opens and parses every chunk file to check it, so a caller
/// that then wants those chunks was reading each file a second time for bytes
/// this function had in hand. Returning them is the whole difference: the
/// checks below are unchanged, and [`verify_index_integrity`] is this function
/// with the vector dropped, so no caller can observe a different verdict or a
/// different message than it did before.
pub fn verify_index_integrity_loading(
    index: &IrIndex,
    chunk_dir: impl AsRef<Path>,
) -> Result<Vec<IrChunk>> {
    let chunk_dir = chunk_dir.as_ref();
    let mut chunk_refs: HashMap<&str, &IrChunkRef> = HashMap::new();
    for chunk in &index.chunks {
        if chunk_refs
            .insert(chunk.chunk_hash.as_str(), chunk)
            .is_some()
        {
            bail!("Duplicate chunk hash '{}' in index", chunk.chunk_hash);
        }
    }

    for (component_id, chunk_hash) in &index.component_index {
        if !chunk_refs.contains_key(chunk_hash.as_str()) {
            bail!(
                "Component '{}' references unknown chunk '{}'",
                component_id,
                chunk_hash
            );
        }
    }
    for (definition_id, chunk_hash) in &index.definition_index {
        if !chunk_refs.contains_key(chunk_hash.as_str()) {
            bail!(
                "Definition '{}' references unknown chunk '{}'",
                definition_id,
                chunk_hash
            );
        }
    }
    for (artifact_id, chunk_hash) in &index.artifact_index {
        if !chunk_refs.contains_key(chunk_hash.as_str()) {
            bail!(
                "Artifact '{}' references unknown chunk '{}'",
                artifact_id,
                chunk_hash
            );
        }
    }
    for (facet_id, chunk_hash) in &index.facet_index {
        if !chunk_refs.contains_key(chunk_hash.as_str()) {
            bail!(
                "Facet '{}' references unknown chunk '{}'",
                facet_id,
                chunk_hash
            );
        }
    }
    for (catalogue_id, chunk_hash) in &index.catalogue_index {
        if !chunk_refs.contains_key(chunk_hash.as_str()) {
            bail!(
                "Catalogue '{}' references unknown chunk '{}'",
                catalogue_id,
                chunk_hash
            );
        }
    }
    for (binding_id, chunk_hash) in &index.binding_index {
        if !chunk_refs.contains_key(chunk_hash.as_str()) {
            bail!(
                "Binding '{}' references unknown chunk '{}'",
                binding_id,
                chunk_hash
            );
        }
    }

    let mut components_by_chunk: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (component_id, chunk_hash) in &index.component_index {
        components_by_chunk
            .entry(chunk_hash.as_str())
            .or_default()
            .insert(component_id.as_str());
    }

    let mut definitions_by_chunk: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (definition_id, chunk_hash) in &index.definition_index {
        definitions_by_chunk
            .entry(chunk_hash.as_str())
            .or_default()
            .insert(definition_id.as_str());
    }

    let mut artifacts_by_chunk: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (artifact_id, chunk_hash) in &index.artifact_index {
        artifacts_by_chunk
            .entry(chunk_hash.as_str())
            .or_default()
            .insert(artifact_id.as_str());
    }

    let mut facets_by_chunk: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (facet_id, chunk_hash) in &index.facet_index {
        facets_by_chunk
            .entry(chunk_hash.as_str())
            .or_default()
            .insert(facet_id.as_str());
    }

    let mut catalogues_by_chunk: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (catalogue_id, chunk_hash) in &index.catalogue_index {
        catalogues_by_chunk
            .entry(chunk_hash.as_str())
            .or_default()
            .insert(catalogue_id.as_str());
    }

    let mut bindings_by_chunk: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (binding_id, chunk_hash) in &index.binding_index {
        bindings_by_chunk
            .entry(chunk_hash.as_str())
            .or_default()
            .insert(binding_id.as_str());
    }

    // The chunks this walk parses, kept in `index.chunks` order so a caller can
    // iterate them alongside the index entries they were checked against.
    let mut chunks = Vec::with_capacity(index.chunks.len());
    for chunk_ref in &index.chunks {
        let path = chunk_dir.join(format!("chunk-{}.cfir", chunk_ref.chunk_hash));
        if !path.exists() {
            bail!(
                "Missing IR chunk for hash '{}' at '{}'",
                chunk_ref.chunk_hash,
                path.display()
            );
        }
        let chunk = load_chunk(&path)?;
        if chunk.format_version != IR_FORMAT_VERSION {
            bail!(
                "Chunk '{}' has unsupported format version {}",
                chunk_ref.chunk_hash,
                chunk.format_version
            );
        }
        if chunk.chunk_hash != chunk_ref.chunk_hash {
            bail!(
                "Chunk hash mismatch: index '{}' vs chunk '{}'",
                chunk_ref.chunk_hash,
                chunk.chunk_hash
            );
        }
        if chunk.source_id != chunk_ref.source_id {
            bail!(
                "Chunk source mismatch for '{}': index '{}' vs chunk '{}'",
                chunk_ref.chunk_hash,
                chunk_ref.source_id,
                chunk.source_id
            );
        }
        // The content check none of the three checks above can make. Each of
        // them compares a value the file DECLARES about itself — its name, its
        // `chunk_hash` field, its `source_id` — so an edit to what the chunk
        // HOLDS passes all three: the ids stay put, the field stays put, the
        // file keeps its name. Since ADR-0056 Amendment 1 the address is a
        // function of the seven entity maps and nothing else, so it can be
        // recomputed from the file in front of us. `link` makes the same check
        // over an object's chunks (ADR-0058 §D4 stage 3, E_LINK_OBJECT_CORRUPT);
        // this is its package-side twin, and it shares the one preimage
        // function so the two cannot disagree about what a chunk hashes to.
        let recomputed = chunk_hash_of_chunk(&chunk).with_context(|| {
            format!(
                "Failed to recompute the content address of chunk '{}'",
                chunk_ref.chunk_hash
            )
        })?;
        if recomputed != chunk_ref.chunk_hash {
            return Err(anyhow::Error::new(ChunkContentAddressMismatch {
                stored: chunk_ref.chunk_hash.clone(),
                recomputed,
            }));
        }

        let chunk_components: HashSet<&str> = chunk.components.keys().map(|k| k.as_str()).collect();
        let chunk_definitions: HashSet<&str> =
            chunk.definitions.keys().map(|k| k.as_str()).collect();
        let chunk_artifacts: HashSet<&str> = chunk.artifacts.keys().map(|k| k.as_str()).collect();
        let chunk_facets: HashSet<&str> = chunk.facets.keys().map(|k| k.as_str()).collect();
        let chunk_catalogues: HashSet<&str> =
            chunk.catalogues.keys().map(|k| k.as_str()).collect();
        let chunk_bindings: HashSet<&str> = chunk.bindings.keys().map(|k| k.as_str()).collect();

        for component_id in &chunk_components {
            match index.component_index.get(*component_id) {
                Some(hash) if hash == &chunk_ref.chunk_hash => {}
                Some(hash) => bail!(
                    "Component '{}' expected in chunk '{}' but index points to '{}'",
                    component_id,
                    chunk_ref.chunk_hash,
                    hash
                ),
                None => bail!(
                    "Component '{}' is present in chunk '{}' but missing from index",
                    component_id,
                    chunk_ref.chunk_hash
                ),
            }
        }

        for definition_id in &chunk_definitions {
            match index.definition_index.get(*definition_id) {
                Some(hash) if hash == &chunk_ref.chunk_hash => {}
                Some(hash) => bail!(
                    "Definition '{}' expected in chunk '{}' but index points to '{}'",
                    definition_id,
                    chunk_ref.chunk_hash,
                    hash
                ),
                None => bail!(
                    "Definition '{}' is present in chunk '{}' but missing from index",
                    definition_id,
                    chunk_ref.chunk_hash
                ),
            }
        }

        for artifact_id in &chunk_artifacts {
            match index.artifact_index.get(*artifact_id) {
                Some(hash) if hash == &chunk_ref.chunk_hash => {}
                Some(hash) => bail!(
                    "Artifact '{}' expected in chunk '{}' but index points to '{}'",
                    artifact_id,
                    chunk_ref.chunk_hash,
                    hash
                ),
                None => bail!(
                    "Artifact '{}' is present in chunk '{}' but missing from index",
                    artifact_id,
                    chunk_ref.chunk_hash
                ),
            }
        }

        for facet_id in &chunk_facets {
            match index.facet_index.get(*facet_id) {
                Some(hash) if hash == &chunk_ref.chunk_hash => {}
                Some(hash) => bail!(
                    "Facet '{}' expected in chunk '{}' but index points to '{}'",
                    facet_id,
                    chunk_ref.chunk_hash,
                    hash
                ),
                None => bail!(
                    "Facet '{}' is present in chunk '{}' but missing from index",
                    facet_id,
                    chunk_ref.chunk_hash
                ),
            }
        }

        for catalogue_id in &chunk_catalogues {
            match index.catalogue_index.get(*catalogue_id) {
                Some(hash) if hash == &chunk_ref.chunk_hash => {}
                Some(hash) => bail!(
                    "Catalogue '{}' expected in chunk '{}' but index points to '{}'",
                    catalogue_id,
                    chunk_ref.chunk_hash,
                    hash
                ),
                None => bail!(
                    "Catalogue '{}' is present in chunk '{}' but missing from index",
                    catalogue_id,
                    chunk_ref.chunk_hash
                ),
            }
        }

        for binding_id in &chunk_bindings {
            match index.binding_index.get(*binding_id) {
                Some(hash) if hash == &chunk_ref.chunk_hash => {}
                Some(hash) => bail!(
                    "Binding '{}' expected in chunk '{}' but index points to '{}'",
                    binding_id,
                    chunk_ref.chunk_hash,
                    hash
                ),
                None => bail!(
                    "Binding '{}' is present in chunk '{}' but missing from index",
                    binding_id,
                    chunk_ref.chunk_hash
                ),
            }
        }

        if let Some(expected) = components_by_chunk.get(chunk_ref.chunk_hash.as_str()) {
            for component_id in expected {
                if !chunk_components.contains(component_id) {
                    bail!(
                        "Component '{}' listed in index for chunk '{}' but not in chunk data",
                        component_id,
                        chunk_ref.chunk_hash
                    );
                }
            }
        }
        if let Some(expected) = definitions_by_chunk.get(chunk_ref.chunk_hash.as_str()) {
            for definition_id in expected {
                if !chunk_definitions.contains(definition_id) {
                    bail!(
                        "Definition '{}' listed in index for chunk '{}' but not in chunk data",
                        definition_id,
                        chunk_ref.chunk_hash
                    );
                }
            }
        }
        if let Some(expected) = artifacts_by_chunk.get(chunk_ref.chunk_hash.as_str()) {
            for artifact_id in expected {
                if !chunk_artifacts.contains(artifact_id) {
                    bail!(
                        "Artifact '{}' listed in index for chunk '{}' but not in chunk data",
                        artifact_id,
                        chunk_ref.chunk_hash
                    );
                }
            }
        }
        if let Some(expected) = facets_by_chunk.get(chunk_ref.chunk_hash.as_str()) {
            for facet_id in expected {
                if !chunk_facets.contains(facet_id) {
                    bail!(
                        "Facet '{}' listed in index for chunk '{}' but not in chunk data",
                        facet_id,
                        chunk_ref.chunk_hash
                    );
                }
            }
        }
        if let Some(expected) = catalogues_by_chunk.get(chunk_ref.chunk_hash.as_str()) {
            for catalogue_id in expected {
                if !chunk_catalogues.contains(catalogue_id) {
                    bail!(
                        "Catalogue '{}' listed in index for chunk '{}' but not in chunk data",
                        catalogue_id,
                        chunk_ref.chunk_hash
                    );
                }
            }
        }
        if let Some(expected) = bindings_by_chunk.get(chunk_ref.chunk_hash.as_str()) {
            for binding_id in expected {
                if !chunk_bindings.contains(binding_id) {
                    bail!(
                        "Binding '{}' listed in index for chunk '{}' but not in chunk data",
                        binding_id,
                        chunk_ref.chunk_hash
                    );
                }
            }
        }

        // Last statement in the body: every check above reads `chunk`, and the
        // borrows they take end here.
        chunks.push(chunk);
    }

    Ok(chunks)
}

/// The canonical bytes of one emitted `chunk-<hash>.cfir` file.
///
/// Compact JSON rendered through `serde_json::Value`, whose object keys are a
/// `BTreeMap` (`preserve_order` is off), so every map in the document — at any
/// depth, whatever Rust type produced it — is written in key order.
///
/// Serializing an `IrChunk` directly is NOT equivalent, and the difference is a
/// defect rather than a nicety. `Component::params` is a `HashMap`, so a direct
/// rendering emits it in one HashMap instance's iteration order, and two
/// compiles of one model write different chunk bytes. `model_hash` never
/// noticed — it is computed over [`chunk_hash_from_config`], which already
/// renders through `Value` — so the packages verified as identical while their
/// files differed, and no two builds of one model were comparable byte for byte.
/// Found by configflux-p0jz.1's object-vs-package chunk identity test; ADR-0058
/// §D4's "link produces the package `compile` produces" oracle cannot hold
/// without it.
///
/// The chunk SHAPE is unchanged, so [`IR_FORMAT_VERSION`] does not move: every
/// reader parses JSON and no reader depends on key order. Bumping it would
/// rotate the `model_hash` of every model in existence for a change that alters
/// no meaning.
pub(crate) fn chunk_file_bytes(chunk: &IrChunk) -> Result<Vec<u8>> {
    let value =
        serde_json::to_value(chunk).context("Failed to serialize IR chunk for emission")?;
    serde_json::to_vec(&value).context("Failed to canonicalize IR chunk for emission")
}

/// The content address a parsed chunk will be written under: SHA-256 over the
/// canonical (sorted-key) JSON encoding of its seven entity maps.
///
/// Because `serde_json::Value` keys are a BTreeMap (preserve_order is off), the
/// encoding is deterministic and independent of source field/map order — so the
/// same logical chunk hashes identically whether it was authored as TOML or as
/// CUE-emitted JSON. This keeps the CUE front-end out of the hash *computation*
/// while making cross-format CMP byte-identical (ADR-0021). ADR-0056
/// Amendment 1 strengthened that equivalence rather than weakening it: two
/// authorings now share an address even under different `package` and `version`
/// labels, because those labels are outside the preimage entirely.
///
/// The ingest-side constructor. [`chunk_hash_of_chunk`] is the verification-side
/// one, over an [`IrChunk`] read back from disk; the two share
/// [`ChunkPreimage`], so they cannot disagree.
pub fn chunk_hash_from_config(config: &Config) -> Result<String> {
    hash_chunk_preimage(&ChunkEntities::from_config(config).preimage())
}

/// Recompute a chunk's content address from the chunk itself.
///
/// This is what makes a `chunk-<hash>.cfir` file self-verifying: its name is a
/// function of its own bytes, so `link` (ADR-0058 §D4 stage 3), a package
/// checker, a lockfile checker or a reader with a JSON parser can all confirm
/// that the content in front of them is the content the address names. Under
/// the previous preimage — the authored `Config`, `package` and `version`
/// included — none of them could: an edited value inside a chunk file left the
/// embedded `chunk_hash` and the filename untouched and no check could see it.
///
/// The address covers entity content only. `source_id` is on-disk provenance
/// (ADR-0056 §5) and `format_version` is the shape discriminator; neither is
/// content, and neither is hashed.
pub fn chunk_hash_of_chunk(chunk: &IrChunk) -> Result<String> {
    hash_chunk_preimage(&chunk.preimage())
}

fn hash_index_content(content: &IrIndexContent) -> Result<String> {
    let json = serde_json::to_vec(content).context("Failed to serialize IR index content")?;
    Ok(sha256_hex(&json))
}

/// SHA-256 of `bytes` as lowercase hex.
///
/// `pub(crate)` so the object header (ADR-0058 §D2) hashes its preimage with
/// the SAME function the chunk and index hashes use; a second implementation
/// is a second place for the hex alphabet to disagree.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push(hex_char(b >> 4));
        out.push(hex_char(b & 0x0f));
    }
    out
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => '?',
    }
}

// ----------------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario_test_support::unique_temp_path;
    use crate::schema::{
        Artifact, Binding, Catalogue, CatalogueField, CatalogueFieldType, Component, Config,
        Parameter, Value,
    };
    use std::collections::{BTreeMap, HashMap};

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
            facet: None,
            overrides: Vec::new(),
        }
    }

    /// The chunk address is a function of the chunk's entity maps and of
    /// nothing else (ADR-0056 Amendment 1). Source field order never mattered;
    /// what changed is that the two LABELS an emitted chunk file does not carry
    /// — `package` and `version` — no longer reach the address, so the address
    /// a reader recomputes from the file equals the one the compiler wrote.
    #[test]
    fn chunk_hash_from_config_is_canonical() {
        // Same logical model; source field/map order must not affect the hash.
        let a: Config = toml::from_str(
            "package = \"p1\"\nversion = \"1.0\"\n\n[definitions.speed]\ntype = \"float\"\n",
        )
        .unwrap();
        let b: Config = toml::from_str(
            "version = \"1.0\"\npackage = \"p1\"\n\n[definitions.speed]\ntype = \"float\"\n",
        )
        .unwrap();
        // Same entity maps under a different `version` label. `version` reaches
        // no output — a loaded package is rebuilt as version "0.0.0" — so it is
        // outside every identity preimage but the source digest.
        let c: Config = toml::from_str(
            "package = \"p1\"\nversion = \"2.0\"\n\n[definitions.speed]\ntype = \"float\"\n",
        )
        .unwrap();
        // Same entity maps under a different `package` label. `package` is the
        // UNIT's name and lives in `ObjectHeader.unit`, never in a chunk
        // address.
        let d: Config = toml::from_str(
            "package = \"p2\"\nversion = \"1.0\"\n\n[definitions.speed]\ntype = \"float\"\n",
        )
        .unwrap();
        // A changed entity map still moves the address.
        let e: Config = toml::from_str(
            "package = \"p1\"\nversion = \"1.0\"\n\n[definitions.speed]\ntype = \"integer\"\n",
        )
        .unwrap();

        let hash_a = chunk_hash_from_config(&a).unwrap();
        let hash_b = chunk_hash_from_config(&b).unwrap();
        let hash_c = chunk_hash_from_config(&c).unwrap();
        let hash_d = chunk_hash_from_config(&d).unwrap();
        let hash_e = chunk_hash_from_config(&e).unwrap();

        assert_eq!(hash_a, hash_b);
        assert_eq!(
            hash_a, hash_c,
            "a `version` label must not change the chunk address"
        );
        assert_eq!(
            hash_a, hash_d,
            "a `package` label must not change the chunk address"
        );
        assert_ne!(
            hash_a, hash_e,
            "a changed entity map must change the chunk address"
        );
    }

    #[test]
    fn chunk_hash_is_format_agnostic() {
        // The same model authored as TOML vs CUE-emitted JSON must hash
        // identically — the byte-identical-CMP property the CUE front-end
        // relies on (ADR 0021).
        let toml_src = "package = \"p\"\nversion = \"1.0.0\"\n\n[definitions.speed]\ntype = \"float\"\nvalue = 0.08\nlifecycle = \"runtime\"\n";
        let json_src = r#"{"package":"p","version":"1.0.0","definitions":{"speed":{"type":"float","value":0.08,"lifecycle":"runtime"}}}"#;

        let from_toml: Config = toml::from_str(toml_src).unwrap();
        let from_json: Config = serde_json::from_str(json_src).unwrap();

        assert_eq!(
            chunk_hash_from_config(&from_toml).unwrap(),
            chunk_hash_from_config(&from_json).unwrap()
        );
    }

    #[test]
    fn sha256_hex_matches_known_value() {
        let hash = sha256_hex(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn ir_chunk_from_config_copies_fields() {
        let mut definitions = HashMap::new();
        let mut def = empty_param();
        def.r#type = Some("float".to_string());
        definitions.insert("speed".to_string(), def);

        let mut params = HashMap::new();
        let mut param = empty_param();
        param.r#type = Some("float".to_string());
        param.value = Some(Value::Float(1.0));
        params.insert("value".to_string(), param);

        let mut components = HashMap::new();
        components.insert(
            "motor".to_string(),
            Component {
                r#type: Some("actuator".to_string()),
                condition: None,
                depends_on: Vec::new(),
                requires: Default::default(),
                params,
            },
        );

        let mut artifacts = HashMap::new();
        artifacts.insert(
            "motor_driver".to_string(),
            Artifact {
                name: "motor_driver".to_string(),
                version: Some("1.2.3".to_string()),
                hash: None,
                source: Some("artifact://motor_driver".to_string()),
                target: Some("/opt/drivers/motor".to_string()),
                doc: None,
            },
        );

        let config = Config {
            package: "pkg".to_string(),
            version: "1.0".to_string(),
            definitions,
            components,
            artifacts,
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };

        let chunk = IrChunk::from_config("src/config.toml", "abc", &config);
        assert_eq!(chunk.format_version, IR_FORMAT_VERSION);
        assert_eq!(chunk.chunk_hash, "abc");
        assert_eq!(chunk.source_id, "src/config.toml");
        assert_eq!(chunk.definitions.len(), 1);
        assert_eq!(chunk.components.len(), 1);
        assert_eq!(chunk.artifacts.len(), 1);
    }

    #[test]
    fn ir_index_hash_matches_compute() {
        let chunks = vec![IrChunkRef {
            chunk_hash: "hash1".to_string(),
            source_id: "source1".to_string(),
        }];
        let index = IrIndex::from_parts(
            chunks,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(index.compute_config_hash().unwrap(), index.config_hash);
    }

    #[test]
    fn verify_index_integrity_ok() {
        let mut definitions = HashMap::new();
        let mut def = empty_param();
        def.r#type = Some("float".to_string());
        definitions.insert("speed".to_string(), def);

        let mut params = HashMap::new();
        let mut param = empty_param();
        param.r#type = Some("float".to_string());
        param.value = Some(Value::Float(1.0));
        params.insert("value".to_string(), param);

        let mut components = HashMap::new();
        components.insert(
            "motor".to_string(),
            Component {
                r#type: Some("actuator".to_string()),
                condition: None,
                depends_on: Vec::new(),
                requires: Default::default(),
                params,
            },
        );

        let mut artifacts = HashMap::new();
        artifacts.insert(
            "motor_driver".to_string(),
            Artifact {
                name: "motor_driver".to_string(),
                version: None,
                hash: None,
                source: Some("artifact://motor_driver".to_string()),
                target: None,
                doc: None,
            },
        );

        let config = Config {
            package: "pkg".to_string(),
            version: "1.0".to_string(),
            definitions,
            components,
            artifacts,
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };

        // The chunk's REAL address: the walk recomputes it from the emitted
        // content, so a fixture stamped with an invented hash is not a package
        // any reader would accept.
        let chunk_hash = chunk_hash_from_config(&config).unwrap();
        let chunk = IrChunk::from_config("source.toml", &chunk_hash, &config);

        let temp_dir = unique_temp_path("configflux-ir", "verify");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let chunk_path = temp_dir.join(format!("chunk-{}.cfir", chunk_hash));
        std::fs::write(&chunk_path, serde_json::to_vec(&chunk).unwrap()).unwrap();

        let mut component_index = BTreeMap::new();
        component_index.insert("motor".to_string(), chunk_hash.to_string());
        let mut definition_index = BTreeMap::new();
        definition_index.insert("speed".to_string(), chunk_hash.to_string());
        let mut artifact_index = BTreeMap::new();
        artifact_index.insert("motor_driver".to_string(), chunk_hash.to_string());

        let index = IrIndex::from_parts(
            vec![IrChunkRef {
                chunk_hash: chunk_hash.to_string(),
                source_id: "source.toml".to_string(),
            }],
            component_index,
            definition_index,
            artifact_index,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        verify_index_integrity(&index, &temp_dir).unwrap();

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn verify_index_integrity_missing_chunk() {
        let index = IrIndex::from_parts(
            vec![IrChunkRef {
                chunk_hash: "missing".to_string(),
                source_id: "source.toml".to_string(),
            }],
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        let temp_dir = unique_temp_path("configflux-ir", "missing");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let err = verify_index_integrity(&index, &temp_dir).unwrap_err();
        assert!(format!("{err}").contains("Missing IR chunk"), "err: {err}");
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn ir_format_version_is_four_for_catalogue_bearing_chunks() {
        // ADR-0047 took this 1 -> 2 (the facet_index preimage change).
        // ADR-0054 §4/§7 took it 2 -> 3 (`IrChunk` gained `constraints`).
        // ADR-0057 §D9 takes it 3 -> 4: `IrChunk` gained `catalogues` and
        // `bindings`, and `IrIndex` gained the two matching indices, which join
        // the `model_hash` preimage. This constant is the ONLY barrier against
        // a v4 toolchain silently reading a pre-catalogue package as
        // catalogue-free. model_hash rotates globally as a result, exactly as
        // it did for the two bumps before it.
        assert_eq!(IR_FORMAT_VERSION, 4);
    }

    #[test]
    fn a_previous_format_chunk_is_rejected_by_format_version() {
        // THE test for the package-side guarantee (ADR-0054 §7, extended by
        // ADR-0057 §D9). The IR bump's whole justification is that this
        // comparison is the only barrier between the current toolchain and a
        // stale CMP — `chunk_hash` is read from the index, never recomputed
        // from chunk bytes, so a stale package is otherwise self-consistent and
        // would load and be read as if the missing namespaces were simply
        // absent. Assert the barrier actually bites.
        let temp_dir = unique_temp_path("configflux-ir", "stale-format");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let config = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        let chunk_hash = chunk_hash_from_config(&config).unwrap();
        let mut chunk = IrChunk::from_config("s.json", &chunk_hash, &config);
        // Stamp the PREVIOUS format version: a chunk emitted before the
        // `constraints` namespace existed.
        chunk.format_version = IR_FORMAT_VERSION - 1;
        let chunk_path = temp_dir.join(format!("chunk-{chunk_hash}.cfir"));
        std::fs::write(&chunk_path, serde_json::to_vec(&chunk).unwrap()).unwrap();

        let index = IrIndex::from_parts(
            vec![IrChunkRef {
                chunk_hash: chunk_hash.clone(),
                source_id: "s.json".to_string(),
            }],
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        let err = verify_index_integrity(&index, &temp_dir).unwrap_err();
        assert!(
            format!("{err}").contains("unsupported format version"),
            "a pre-constraints chunk must be rejected, got: {err}"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn from_config_carries_constraints() {
        let mut constraints = HashMap::new();
        constraints.insert(
            "prod_forbids_debug".to_string(),
            Constraint {
                condition: "environment != 'prod' || log_level != 'debug'".to_string(),
                doc: Some("Debug logging is not permitted in production.".to_string()),
            },
        );
        let config = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints,
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };

        let chunk = IrChunk::from_config("s.json", "abc", &config);
        assert_eq!(chunk.constraints.len(), 1);
        let declared = chunk
            .constraints
            .get("prod_forbids_debug")
            .expect("constraint carried into the chunk IR");
        assert_eq!(
            declared.condition,
            "environment != 'prod' || log_level != 'debug'"
        );
        assert_eq!(
            declared.doc.as_deref(),
            Some("Debug logging is not permitted in production.")
        );
    }

    #[test]
    fn chunk_hash_changes_when_a_constraint_is_declared() {
        // A constraint is authored policy and part of the model's content
        // address — declaring one must move `chunk_hash`, or two models with
        // different policies would share a `model_hash`.
        let bare = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        let mut constraints = HashMap::new();
        constraints.insert(
            "eu_needs_tls".to_string(),
            Constraint {
                condition: "region != 'eu' || tls_mode == 'strict'".to_string(),
                doc: None,
            },
        );
        let with_policy = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints,
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };

        assert_ne!(
            chunk_hash_from_config(&bare).unwrap(),
            chunk_hash_from_config(&with_policy).unwrap()
        );
    }

    #[test]
    fn from_config_carries_facets() {
        let mut facets = HashMap::new();
        facets.insert(
            "region".to_string(),
            Facet {
                values: vec!["eu".to_string(), "us".to_string(), "apac".to_string()],
                default: Some("eu".to_string()),
                open: false,
                doc: None,
            },
        );
        let config = Config {
            package: "pkg".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets,
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        let chunk = IrChunk::from_config("src", "h", &config);
        assert_eq!(chunk.facets.len(), 1);
        let region = chunk.facets.get("region").expect("facet carried");
        assert_eq!(region.values, vec!["eu", "us", "apac"]);
        assert_eq!(region.default.as_deref(), Some("eu"));
        assert!(!region.open);
    }

    #[test]
    fn facet_index_enters_model_hash_preimage() {
        // A model that declares a facet must not collide with the same model
        // without the declaration: facet_index is in the hash preimage.
        let chunks = vec![IrChunkRef {
            chunk_hash: "h".to_string(),
            source_id: "s".to_string(),
        }];
        let without = IrIndex::from_parts(
            chunks.clone(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        let mut facet_index = BTreeMap::new();
        facet_index.insert("region".to_string(), "h".to_string());
        let with = IrIndex::from_parts(
            chunks,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            facet_index,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_ne!(without.config_hash, with.config_hash);
        assert_eq!(with.compute_config_hash().unwrap(), with.config_hash);
    }

    fn container_catalogue() -> Catalogue {
        let mut fields = BTreeMap::new();
        fields.insert(
            "width_mm".to_string(),
            CatalogueField {
                r#type: CatalogueFieldType::Integer,
                unit: Some("mm".to_string()),
                doc: None,
            },
        );
        let mut c1 = BTreeMap::new();
        c1.insert("width_mm".to_string(), Value::Integer(800));
        let mut c2 = BTreeMap::new();
        c2.insert("width_mm".to_string(), Value::Integer(600));
        let mut entries = BTreeMap::new();
        entries.insert("c1".to_string(), c1);
        entries.insert("c2".to_string(), c2);
        Catalogue {
            fields,
            entries,
            doc: None,
        }
    }

    fn derived_binding() -> Binding {
        let mut pairs = BTreeMap::new();
        pairs.insert("factory_a".to_string(), "c1".to_string());
        let mut table = BTreeMap::new();
        table.insert("site".to_string(), pairs);
        Binding {
            catalogue: "containers".to_string(),
            default: None,
            derive: Some(table),
            doc: None,
        }
    }

    #[test]
    fn from_config_carries_catalogues_and_bindings_verbatim() {
        // ADR-0057 §D3: `derive` lowers to root conjuncts in configflux-secb.5.
        // Until then the table has to survive the round trip untouched, or the
        // compiled package would silently mean less than the author wrote.
        let mut config = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        config
            .catalogues
            .insert("containers".to_string(), container_catalogue());
        config
            .bindings
            .insert("line_container".to_string(), derived_binding());

        let chunk = IrChunk::from_config("s.json", "abc", &config);

        let catalogue = chunk.catalogues.get("containers").expect("catalogue carried");
        assert_eq!(
            catalogue.entries.keys().cloned().collect::<Vec<_>>(),
            vec!["c1".to_string(), "c2".to_string()]
        );
        assert_eq!(
            catalogue.fields["width_mm"].r#type,
            CatalogueFieldType::Integer
        );
        let binding = chunk.bindings.get("line_container").expect("binding carried");
        assert_eq!(binding.catalogue, "containers");
        assert_eq!(
            binding.derive.as_ref().unwrap()["site"]["factory_a"],
            "c1".to_string()
        );
    }

    #[test]
    fn chunk_hash_changes_when_a_catalogue_or_binding_is_declared() {
        // A catalogue is authored data and a binding is an authored decision;
        // both are part of the model's content address, or two models with
        // different tables would share a `model_hash`.
        let bare = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        let mut with_catalogue = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        with_catalogue
            .catalogues
            .insert("containers".to_string(), container_catalogue());
        let mut with_binding = Config {
            package: "p".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        with_binding
            .catalogues
            .insert("containers".to_string(), container_catalogue());
        with_binding
            .bindings
            .insert("line_container".to_string(), derived_binding());

        let bare_hash = chunk_hash_from_config(&bare).unwrap();
        let catalogue_hash = chunk_hash_from_config(&with_catalogue).unwrap();
        let binding_hash = chunk_hash_from_config(&with_binding).unwrap();
        assert_ne!(bare_hash, catalogue_hash);
        assert_ne!(catalogue_hash, binding_hash);
    }

    #[test]
    fn catalogue_index_and_binding_index_enter_model_hash_preimage() {
        // ADR-0057 §D9: both join the preimage exactly as `facet_index` did, so
        // a model that declares a table cannot collide with the same model
        // without it.
        let chunks = vec![IrChunkRef {
            chunk_hash: "h".to_string(),
            source_id: "s".to_string(),
        }];
        let empty = || BTreeMap::new();
        let without = IrIndex::from_parts(
            chunks.clone(),
            empty(),
            empty(),
            empty(),
            empty(),
            empty(),
            empty(),
        )
        .unwrap();

        let mut catalogue_index = BTreeMap::new();
        catalogue_index.insert("containers".to_string(), "h".to_string());
        let with_catalogue = IrIndex::from_parts(
            chunks.clone(),
            empty(),
            empty(),
            empty(),
            empty(),
            catalogue_index.clone(),
            empty(),
        )
        .unwrap();

        let mut binding_index = BTreeMap::new();
        binding_index.insert("line_container".to_string(), "h".to_string());
        let with_binding = IrIndex::from_parts(
            chunks,
            empty(),
            empty(),
            empty(),
            empty(),
            catalogue_index,
            binding_index,
        )
        .unwrap();

        assert_ne!(without.config_hash, with_catalogue.config_hash);
        assert_ne!(with_catalogue.config_hash, with_binding.config_hash);
        assert_eq!(
            with_binding.compute_config_hash().unwrap(),
            with_binding.config_hash
        );
    }

    #[test]
    fn verify_index_integrity_rejects_catalogue_and_binding_index_drift() {
        // Symmetric with the definition/component/artifact/facet checks: an id
        // listed in the index but absent from the chunk data must fail closed.
        let config = Config {
            package: "pkg".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        let chunk_hash = chunk_hash_from_config(&config).unwrap();
        let chunk = IrChunk::from_config("source.toml", &chunk_hash, &config);

        let temp_dir = unique_temp_path("configflux-ir", "catalogue");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let chunk_path = temp_dir.join(format!("chunk-{}.cfir", chunk_hash));
        std::fs::write(&chunk_path, serde_json::to_vec(&chunk).unwrap()).unwrap();

        let refs = || {
            vec![IrChunkRef {
                chunk_hash: chunk_hash.to_string(),
                source_id: "source.toml".to_string(),
            }]
        };
        let empty = || BTreeMap::new();

        let mut catalogue_index = BTreeMap::new();
        catalogue_index.insert("containers".to_string(), chunk_hash.to_string());
        let index = IrIndex::from_parts(
            refs(),
            empty(),
            empty(),
            empty(),
            empty(),
            catalogue_index,
            empty(),
        )
        .unwrap();
        let err = verify_index_integrity(&index, &temp_dir).unwrap_err();
        assert!(format!("{err}").contains("Catalogue 'containers'"), "err: {err}");

        let mut binding_index = BTreeMap::new();
        binding_index.insert("line_container".to_string(), chunk_hash.to_string());
        let index = IrIndex::from_parts(
            refs(),
            empty(),
            empty(),
            empty(),
            empty(),
            empty(),
            binding_index,
        )
        .unwrap();
        let err = verify_index_integrity(&index, &temp_dir).unwrap_err();
        assert!(
            format!("{err}").contains("Binding 'line_container'"),
            "err: {err}"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn verify_index_integrity_rejects_facet_index_missing_from_chunk() {
        // A facet listed in the index but absent from the chunk data must fail
        // closed, mirroring the definition/component/artifact symmetry checks.
        let config = Config {
            package: "pkg".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: HashMap::new(),
            constraints: HashMap::new(),
            catalogues: HashMap::new(),
            bindings: HashMap::new(),
        };
        let chunk_hash = chunk_hash_from_config(&config).unwrap();
        let chunk = IrChunk::from_config("source.toml", &chunk_hash, &config);

        let temp_dir = unique_temp_path("configflux-ir", "facet");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let chunk_path = temp_dir.join(format!("chunk-{}.cfir", chunk_hash));
        std::fs::write(&chunk_path, serde_json::to_vec(&chunk).unwrap()).unwrap();

        let mut facet_index = BTreeMap::new();
        facet_index.insert("region".to_string(), chunk_hash.to_string());
        let index = IrIndex::from_parts(
            vec![IrChunkRef {
                chunk_hash: chunk_hash.to_string(),
                source_id: "source.toml".to_string(),
            }],
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            facet_index,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        let err = verify_index_integrity(&index, &temp_dir).unwrap_err();
        assert!(
            format!("{err}").contains("Facet 'region'"),
            "err: {err}"
        );
        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
