// SPDX-License-Identifier: BUSL-1.1

use crate::schema::{Artifact, Component, Config, Constraint, Facet, Parameter};
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
// guards the PACKAGE. Both must move together.
pub const IR_FORMAT_VERSION: u32 = 3;
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
pub const CMP_CANONICALIZATION_VERSION: u32 = 2;
pub const CMP_DEFAULT_MANIFEST_FILENAME: &str = "cmp.manifest.json";
pub const CMP_DEFAULT_INDEX_REF: &str = "index.cfir.json";
pub const CMP_DEFAULT_CHUNK_SET_REF: &str = ".";

#[derive(Debug, Serialize, Deserialize)]
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
}

/// Project chunk refs onto the preimage's bare-hash vector, preserving order.
fn preimage_chunks(chunks: &[IrChunkRef]) -> Vec<String> {
    chunks.iter().map(|c| c.chunk_hash.clone()).collect()
}

impl IrChunk {
    pub fn from_config(source_id: &str, chunk_hash: &str, config: &Config) -> Self {
        let definitions = config
            .definitions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let components = config
            .components
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let artifacts = config
            .artifacts
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let facets = config
            .facets
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let constraints = config
            .constraints
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Self {
            format_version: IR_FORMAT_VERSION,
            chunk_hash: chunk_hash.to_string(),
            source_id: source_id.to_string(),
            definitions,
            components,
            artifacts,
            facets,
            constraints,
            metadata: None,
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
    ) -> Result<Self> {
        let content = IrIndexContent {
            format_version: IR_FORMAT_VERSION,
            chunks: preimage_chunks(&chunks),
            component_index,
            definition_index,
            artifact_index,
            facet_index,
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

pub fn load_index(path: impl AsRef<Path>) -> Result<IrIndex> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read IR index '{}'", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse IR index '{}'", path.display()))
}

pub fn load_chunk(path: impl AsRef<Path>) -> Result<IrChunk> {
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

pub fn verify_index_integrity(index: &IrIndex, chunk_dir: impl AsRef<Path>) -> Result<()> {
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

        let chunk_components: HashSet<&str> = chunk.components.keys().map(|k| k.as_str()).collect();
        let chunk_definitions: HashSet<&str> =
            chunk.definitions.keys().map(|k| k.as_str()).collect();
        let chunk_artifacts: HashSet<&str> = chunk.artifacts.keys().map(|k| k.as_str()).collect();
        let chunk_facets: HashSet<&str> = chunk.facets.keys().map(|k| k.as_str()).collect();

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
    }

    Ok(())
}

/// Model-derived chunk hash: SHA-256 over the canonical (sorted-key) JSON
/// encoding of the parsed `Config`. Because `serde_json::Value` keys are a
/// BTreeMap (preserve_order is off), the encoding is deterministic and
/// independent of source field/map order — so the same logical model hashes
/// identically whether it was authored as TOML or as CUE-emitted JSON. This
/// keeps the CUE front-end out of the hash *computation* while making
/// cross-format CMP byte-identical (ADR 0021).
pub fn chunk_hash_from_config(config: &Config) -> Result<String> {
    let value =
        serde_json::to_value(config).context("Failed to serialize config for hashing")?;
    let canonical =
        serde_json::to_string(&value).context("Failed to canonicalize config for hashing")?;
    Ok(sha256_hex(canonical.as_bytes()))
}

fn hash_index_content(content: &IrIndexContent) -> Result<String> {
    let json = serde_json::to_vec(content).context("Failed to serialize IR index content")?;
    Ok(sha256_hex(&json))
}

fn sha256_hex(bytes: &[u8]) -> String {
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
    use crate::schema::{Artifact, Component, Config, Parameter, Value};
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
            overrides: Vec::new(),
        }
    }

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
        // Different value -> different hash.
        let c: Config = toml::from_str(
            "package = \"p1\"\nversion = \"2.0\"\n\n[definitions.speed]\ntype = \"float\"\n",
        )
        .unwrap();

        let hash_a = chunk_hash_from_config(&a).unwrap();
        let hash_b = chunk_hash_from_config(&b).unwrap();
        let hash_c = chunk_hash_from_config(&c).unwrap();

        assert_eq!(hash_a, hash_b);
        assert_ne!(hash_a, hash_c);
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
        };

        let chunk_hash = "hash_ok";
        let chunk = IrChunk::from_config("source.toml", chunk_hash, &config);

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
        )
        .unwrap();

        let temp_dir = unique_temp_path("configflux-ir", "missing");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let err = verify_index_integrity(&index, &temp_dir).unwrap_err();
        assert!(format!("{err}").contains("Missing IR chunk"), "err: {err}");
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn ir_format_version_is_three_for_constraint_bearing_chunks() {
        // ADR-0047 took this 1 -> 2 (the facet_index preimage change).
        // ADR-0054 §4/§7 takes it 2 -> 3: `IrChunk` gained `constraints`, and
        // this constant is the ONLY barrier against a v4 toolchain silently
        // reading a pre-constraints package as constraint-free. model_hash
        // rotates globally as a result, exactly as it did for ADR-0047.
        assert_eq!(IR_FORMAT_VERSION, 3);
    }

    #[test]
    fn a_pre_constraints_chunk_is_rejected_by_format_version() {
        // THE test for ADR-0054 §7's package-side guarantee. The IR bump's
        // whole justification is that this comparison is the only barrier
        // between a v4 toolchain and a pre-constraints CMP — `chunk_hash` is
        // read from the index, never recomputed from chunk bytes, so a stale
        // package is otherwise self-consistent and would load and be read as
        // constraint-free. Assert the barrier actually bites.
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
        )
        .unwrap();
        assert_ne!(without.config_hash, with.config_hash);
        assert_eq!(with.compute_config_hash().unwrap(), with.config_hash);
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
        };
        let chunk_hash = "hf";
        let chunk = IrChunk::from_config("source.toml", chunk_hash, &config);

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
