// SPDX-License-Identifier: BUSL-1.1

use crate::schema::{Artifact, Component, Config, Parameter};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub const IR_FORMAT_VERSION: u32 = 1;
pub const CMP_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CMP_HASH_ALGO: &str = "sha256";
pub const CMP_CANONICALIZATION_VERSION: u32 = 1;
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

#[derive(Serialize)]
struct IrIndexContent {
    format_version: u32,
    chunks: Vec<IrChunkRef>,
    component_index: BTreeMap<String, String>,
    definition_index: BTreeMap<String, String>,
    artifact_index: BTreeMap<String, String>,
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

        Self {
            format_version: IR_FORMAT_VERSION,
            chunk_hash: chunk_hash.to_string(),
            source_id: source_id.to_string(),
            definitions,
            components,
            artifacts,
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
    ) -> Result<Self> {
        let content = IrIndexContent {
            format_version: IR_FORMAT_VERSION,
            chunks,
            component_index,
            definition_index,
            artifact_index,
        };
        let config_hash = hash_index_content(&content)?;
        Ok(Self {
            format_version: content.format_version,
            chunks: content.chunks,
            component_index: content.component_index,
            definition_index: content.definition_index,
            artifact_index: content.artifact_index,
            config_hash,
        })
    }

    pub fn compute_config_hash(&self) -> Result<String> {
        let content = IrIndexContent {
            format_version: self.format_version,
            chunks: self.chunks.clone(),
            component_index: self.component_index.clone(),
            definition_index: self.definition_index.clone(),
            artifact_index: self.artifact_index.clone(),
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
    use crate::schema::{Artifact, Component, Config, Parameter, Value};
    use std::collections::{BTreeMap, HashMap};
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let index =
            IrIndex::from_parts(chunks, BTreeMap::new(), BTreeMap::new(), BTreeMap::new()).unwrap();
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
        };

        let chunk_hash = "hash_ok";
        let chunk = IrChunk::from_config("source.toml", chunk_hash, &config);

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "configflux-ir-verify-{}-{}",
            std::process::id(),
            unique
        ));
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
        )
        .unwrap();

        let temp_dir =
            std::env::temp_dir().join(format!("configflux-ir-missing-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let err = verify_index_integrity(&index, &temp_dir).unwrap_err();
        assert!(format!("{err}").contains("Missing IR chunk"), "err: {err}");
        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
