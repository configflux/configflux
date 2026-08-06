// SPDX-License-Identifier: BUSL-1.1

//! Tool-identity provenance sidecar — ADR-0044 D1 (amended 2026-07-02;
//! `configflux-pq2w.1` / OBS-1).
//!
//! Every FILE-writing artifact operation (compile → the CMP directory and
//! its sibling CCM directory) additionally writes a deterministic,
//! NON-hashed `provenance.json` next to its primary output. The sidecar
//! records the tool name, the workspace `tool_version` (read from the
//! repo-root `/VERSION` file — the single source of truth), the relevant
//! schema/format versions, and the SHA-256 content hashes of the primary
//! artifacts it accompanies.
//!
//! The sidecar is NEVER part of any hash preimage: no existing artifact byte
//! changes, and by default the sidecar itself is byte-identical across runs
//! because it carries no wall-clock. Wall-clock is opt-in via `--stamp-time`
//! (sets `stamped_at`). This is the same "run-varying data lives on a
//! side-channel, never in hashed bytes" discipline the compiler already
//! applies to progress, RSS, and `BudgetReport`.
//!
//! Why not `env!("CARGO_PKG_VERSION")`: under Bazel the compiler LIBRARY
//! target carries no `version` attr, so that macro resolves to `"0.0.0"`.
//! `/VERSION` is wired in as `compile_data` (`//:VERSION`) and read with
//! `include_str!` so the real workspace version is baked into the library.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Human-readable name recorded in the sidecar `tool` field.
pub const PROVENANCE_TOOL_NAME: &str = "configflux-compiler";

/// Sidecar filename, written as a sibling of the primary artifact set.
pub const PROVENANCE_SIDECAR_FILENAME: &str = "provenance.json";

/// The workspace version, read from the repo-root `/VERSION` file at compile
/// time. `/VERSION` is provided as `compile_data` (`//:VERSION`) on the
/// compiler crate; `include_str!` resolves it relative to this source file
/// (`compiler/src/` → repo root is `../../`).
pub fn tool_version() -> &'static str {
    include_str!("../../VERSION").trim()
}

/// A deterministic, non-hashed provenance record written next to a
/// file-writing artifact set. Serializes to canonical, sorted-key JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvenanceSidecar {
    /// Producing tool (`configflux-compiler`).
    pub tool: String,
    /// Workspace version that produced the artifacts (from `/VERSION`).
    pub tool_version: String,
    /// Relevant schema/format versions, keyed by a stable short name.
    pub schema_versions: BTreeMap<String, u32>,
    /// Relative artifact path → SHA-256 content hash (hex). Recomputable by
    /// any holder of the artifacts for a tamper check.
    pub artifacts: BTreeMap<String, String>,
    /// Opt-in wall-clock stamp (`--stamp-time`, RFC3339 UTC). Absent — NOT
    /// zeroed — by default so the sidecar is byte-stable across runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stamped_at: Option<String>,
}

impl ProvenanceSidecar {
    /// Build a sidecar stamped with the current workspace `tool_version`.
    pub fn new(
        schema_versions: BTreeMap<String, u32>,
        artifacts: BTreeMap<String, String>,
        stamped_at: Option<String>,
    ) -> Self {
        Self {
            tool: PROVENANCE_TOOL_NAME.to_string(),
            tool_version: tool_version().to_string(),
            schema_versions,
            artifacts,
            stamped_at,
        }
    }

    /// Serialize to canonical, sorted-key, pretty JSON with a trailing
    /// newline. The recursive canonicalization pins key order for the whole
    /// document regardless of struct field declaration order, so the bytes
    /// are stable across compiler builds.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>> {
        let value =
            serde_json::to_value(self).context("Failed to serialize provenance sidecar")?;
        let canonical = canonicalize_json_value(value);
        let mut bytes = serde_json::to_vec_pretty(&canonical)
            .context("Failed to render canonical provenance sidecar JSON")?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Atomically write the sidecar into `dir` as `provenance.json` (temp
    /// file + rename, mirroring the `telemetry_sink.rs` atomic-write
    /// precedent so a partial write can never leave a torn artifact).
    pub fn write_to_dir(&self, dir: &Path) -> Result<()> {
        let bytes = self.to_canonical_json()?;
        let final_path = dir.join(PROVENANCE_SIDECAR_FILENAME);
        let temp_path = dir.join(format!("{PROVENANCE_SIDECAR_FILENAME}.tmp"));
        {
            let mut file = File::create(&temp_path).with_context(|| {
                format!(
                    "Failed to create provenance temp file '{}'",
                    temp_path.display()
                )
            })?;
            file.write_all(&bytes).with_context(|| {
                format!(
                    "Failed to write provenance temp file '{}'",
                    temp_path.display()
                )
            })?;
            file.flush().with_context(|| {
                format!(
                    "Failed to flush provenance temp file '{}'",
                    temp_path.display()
                )
            })?;
        }
        fs::rename(&temp_path, &final_path).with_context(|| {
            format!(
                "Failed to atomically place provenance sidecar '{}'",
                final_path.display()
            )
        })
    }
}

/// SHA-256 (hex) of a file's bytes — the content hash recorded in the
/// sidecar `artifacts` map.
pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)
        .with_context(|| format!("Failed to read artifact for hashing '{}'", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Current time as an RFC3339 UTC string (`YYYY-MM-DDThh:mm:ssZ`). Used only
/// under `--stamp-time`; the result is intentionally run-varying, which is
/// why it is opt-in and lives on the sidecar rather than in any hashed byte.
pub fn now_rfc3339_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339_utc(secs)
}

/// Convert whole seconds since the Unix epoch to an RFC3339 UTC timestamp.
/// Uses Howard Hinnant's civil-from-days algorithm (proleptic Gregorian);
/// kept dependency-free (the crate has no `chrono`/`time`).
fn format_rfc3339_utc(secs_since_epoch: u64) -> String {
    let days = (secs_since_epoch / 86_400) as i64;
    let secs_of_day = secs_since_epoch % 86_400;
    let (hour, minute, second) = (secs_of_day / 3_600, (secs_of_day % 3_600) / 60, secs_of_day % 60);

    // days since 1970-01-01 → civil (year, month, day).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };

    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    )
}

/// Recursively sort object keys so the emitted JSON is canonical regardless
/// of insertion order (mirrors `loader_api::shared_ops`'s canonicalizer).
fn canonicalize_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut entries: Vec<(String, serde_json::Value)> = object.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut ordered = serde_json::Map::new();
            for (key, value) in entries {
                ordered.insert(key, canonicalize_json_value(value));
            }
            serde_json::Value::Object(ordered)
        }
        serde_json::Value::Array(array) => {
            serde_json::Value::Array(array.into_iter().map(canonicalize_json_value).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario_test_support::unique_temp_path;

    fn sample(stamped_at: Option<String>) -> ProvenanceSidecar {
        let mut schema_versions = BTreeMap::new();
        schema_versions.insert("product".to_string(), 2);
        schema_versions.insert("cmp_manifest".to_string(), 1);
        let mut artifacts = BTreeMap::new();
        artifacts.insert("cmp.manifest.json".to_string(), "abc123".to_string());
        ProvenanceSidecar::new(schema_versions, artifacts, stamped_at)
    }

    #[test]
    fn tool_version_is_non_placeholder() {
        // The /VERSION file must be wired in; a `0.0.0` here means the
        // env!/compile_data plumbing regressed.
        let v = tool_version();
        assert!(!v.is_empty(), "tool_version must be non-empty");
        assert_ne!(v, "0.0.0", "tool_version must come from /VERSION, not the cargo default");
    }

    #[test]
    fn canonical_json_is_byte_stable_across_serializations() {
        let a = sample(None).to_canonical_json().unwrap();
        let b = sample(None).to_canonical_json().unwrap();
        assert_eq!(a, b, "sidecar bytes must be identical across runs");
        // Ends with a single trailing newline.
        assert_eq!(*a.last().unwrap(), b'\n');
        // Absent stamp must not appear at all (skip_serializing_if).
        let text = String::from_utf8(a).unwrap();
        assert!(!text.contains("stamped_at"), "stamped_at must be absent by default");
    }

    #[test]
    fn stamped_at_present_only_when_set() {
        let text =
            String::from_utf8(sample(Some("2026-07-02T00:00:00Z".to_string())).to_canonical_json().unwrap())
                .unwrap();
        assert!(text.contains("\"stamped_at\""));
        assert!(text.contains("2026-07-02T00:00:00Z"));
    }

    #[test]
    fn keys_are_sorted_canonically() {
        let text = String::from_utf8(sample(None).to_canonical_json().unwrap()).unwrap();
        // Top-level keys must appear in sorted order: artifacts, schema_versions, tool, tool_version.
        let pos = |needle: &str| text.find(needle).unwrap();
        assert!(pos("\"artifacts\"") < pos("\"schema_versions\""));
        assert!(pos("\"schema_versions\"") < pos("\"tool\""));
        assert!(pos("\"tool\"") < pos("\"tool_version\""));
    }

    #[test]
    fn hash_file_recomputes_sha256() {
        let dir = unique_temp_path("configflux-prov", "hash");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("artifact.bin");
        fs::write(&path, b"hello").unwrap();
        // sha256("hello")
        assert_eq!(
            hash_file(&path).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rfc3339_formats_known_epochs() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
        // 2026-07-02T00:00:00Z == 1_782_000_000? verify a fixed known value:
        // 1_609_459_200 == 2021-01-01T00:00:00Z
        assert_eq!(format_rfc3339_utc(1_609_459_200), "2021-01-01T00:00:00Z");
        assert_eq!(format_rfc3339_utc(1_609_459_200 + 86_399), "2021-01-01T23:59:59Z");
    }
}
